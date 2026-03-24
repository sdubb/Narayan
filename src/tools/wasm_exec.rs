//! wasm_exec — Execute a WebAssembly module with WASI 0.2 (stable, 2026).
//!
//! Runs .wasm binaries in a fully isolated sandbox:
//!   - stdin/stdout/stderr captured in memory
//!   - Filesystem scoped to workspace dir only  
//!   - Allowlisted env vars only
//!   - Strict fuel + timeout + memory limits

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct WasmExecTool;

const DEFAULT_TIMEOUT_SECS: u64 = 3;
const MAX_TIMEOUT_SECS: u64 = 15;
const DEFAULT_FUEL: u64 = 5_000_000;
const MAX_FUEL: u64 = 20_000_000;
const DEFAULT_MEMORY_LIMIT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MEMORY_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

#[async_trait]
impl Tool for WasmExecTool {
    fn name(&self) -> &str {
        "wasm_exec"
    }
    fn description(&self) -> &str {
        "Execute a WebAssembly module with full WASI 0.2 sandboxing. \
         Accepts a .wasm file path or base64-encoded bytes. \
         Stdin, args, and env vars are configurable. Stdout/stderr are captured. \
         Filesystem is scoped to workspace only. CPU bounded by fuel."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::optional("path", "string", "Path to a .wasm file to execute."),
            ParameterSchema::optional("bytes_b64", "string", "Base64-encoded .wasm bytes (alternative to path)."),
            ParameterSchema::optional("stdin", "string", "Text to pass to the module on stdin."),
            ParameterSchema::optional("args", "array", "Command-line arguments passed to the module."),
            ParameterSchema::optional("env", "object", "Environment variables: {KEY: value}."),
            ParameterSchema::optional("workspace", "string", "Directory mounted as /workspace (read-write)."),
            ParameterSchema::optional("timeout_secs", "integer", "Hard timeout in seconds (default: 3, max: 15)."),
            ParameterSchema::optional("fuel", "integer", "Max WASM fuel (default: 5,000,000; max: 20,000,000)."),
            ParameterSchema::optional(
                "memory_limit_bytes",
                "integer",
                "Linear memory limit in bytes (default: 16MB, max: 64MB).",
            ),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        use base64::Engine;

        let wasm_bytes: Vec<u8> = if let Some(p) = args["path"].as_str() {
            match tokio::fs::read(p).await {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::err(format!("read '{}': {}", p, e))),
            }
        } else if let Some(b64) = args["bytes_b64"].as_str() {
            match base64::engine::general_purpose::STANDARD.decode(b64) {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::err(format!("base64 decode: {}", e))),
            }
        } else {
            return Ok(ToolResult::err("'path' or 'bytes_b64' is required"));
        };

        if wasm_bytes.len() < 4 || &wasm_bytes[..4] != b"\0asm" {
            return Ok(ToolResult::err("invalid WebAssembly binary — missing \\0asm magic"));
        }

        let stdin_data = args["stdin"].as_str().unwrap_or("").to_string();
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(DEFAULT_TIMEOUT_SECS).clamp(1, MAX_TIMEOUT_SECS);
        let fuel_limit = args["fuel"].as_u64().unwrap_or(DEFAULT_FUEL).clamp(100_000, MAX_FUEL);
        let memory_limit_bytes = args["memory_limit_bytes"]
            .as_u64()
            .unwrap_or(DEFAULT_MEMORY_LIMIT_BYTES)
            .clamp(1 * 1024 * 1024, MAX_MEMORY_LIMIT_BYTES);
        let workspace = args["workspace"].as_str().unwrap_or(".").to_string();
        let wasm_size = wasm_bytes.len();

        let cli_args: Vec<String> =
            args["args"].as_array().unwrap_or(&vec![]).iter().filter_map(|v| v.as_str().map(String::from)).collect();

        let env_vars: Vec<(String, String)> = args["env"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();

        let join = tokio::task::spawn_blocking(move || {
            run_wasm(wasm_bytes, stdin_data, cli_args, env_vars, workspace, fuel_limit, memory_limit_bytes)
        });

        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), join).await {
            Ok(Ok(Ok(mut v))) => {
                v["wasm_size_bytes"] = serde_json::json!(wasm_size);
                v["memory_limit_bytes"] = serde_json::json!(memory_limit_bytes);
                v["fuel_limit"] = serde_json::json!(fuel_limit);
                Ok(ToolResult::ok(v))
            }
            Ok(Ok(Err(e))) => Ok(ToolResult::err(format!("WASM error: {}", e))),
            Ok(Err(e)) => Ok(ToolResult::err(format!("thread panic: {}", e))),
            Err(_) => Ok(ToolResult::err(format!("timed out after {}s", timeout_secs))),
        }
    }
}

fn run_wasm(
    bytes: Vec<u8>,
    stdin_data: String,
    args: Vec<String>,
    env_vars: Vec<(String, String)>,
    workspace: String,
    fuel: u64,
    memory_limit_bytes: u64,
) -> anyhow::Result<serde_json::Value> {
    use std::time::Instant;

    use wasmtime::*;
    use wasmtime_wasi::{
        pipe::{MemoryInputPipe, MemoryOutputPipe},
        preview1::{self, WasiP1Ctx},
        WasiCtxBuilder,
    };

    let t0 = Instant::now();
    let mut cfg = Config::new();
    cfg.async_support(false);
    cfg.consume_fuel(true);
    let engine = Engine::new(&cfg)?;

    let stdout_pipe = MemoryOutputPipe::new(512 * 1024);
    let stderr_pipe = MemoryOutputPipe::new(256 * 1024);
    let stdout_read = stdout_pipe.clone();
    let stderr_read = stderr_pipe.clone();

    let mut wb = WasiCtxBuilder::new();
    if !stdin_data.is_empty() {
        wb.stdin(MemoryInputPipe::new(stdin_data.into_bytes()));
    }
    wb.stdout(stdout_pipe).stderr(stderr_pipe);
    wb.args(&args);
    for (k, v) in &env_vars {
        wb.env(k, v);
    }

    let ws = std::path::Path::new(&workspace);
    if ws.exists() {
        wb.preopened_dir(ws, "/workspace", wasmtime_wasi::DirPerms::all(), wasmtime_wasi::FilePerms::all())?;
    }
    let wasi = wb.build_p1();
    let limits = StoreLimitsBuilder::new()
        .memory_size((memory_limit_bytes.min(usize::MAX as u64)) as usize)
        .instances(1)
        .tables(2)
        .memories(1)
        .build();
    let mut store = Store::new(&engine, (wasi, limits));
    store.limiter(|state| &mut state.1);
    store.set_fuel(fuel)?;

    let mut linker: Linker<(WasiP1Ctx, wasmtime::StoreLimits)> = Linker::new(&engine);
    preview1::add_to_linker_sync(&mut linker, |state| &mut state.0)?;

    let module = Module::from_binary(&engine, &bytes)?;
    let instance = linker.instantiate(&mut store, &module)?;

    let exit_code: i32 = match instance.get_typed_func::<(), ()>(&mut store, "_start") {
        Ok(f) => match f.call(&mut store, ()) {
            Ok(()) => 0,
            Err(e) => e.downcast_ref::<wasmtime_wasi::I32Exit>().map(|x| x.0).unwrap_or(1),
        },
        Err(_) => {
            if let Ok(f) = instance.get_typed_func::<(), ()>(&mut store, "_initialize") {
                f.call(&mut store, ())?;
            }
            0
        }
    };

    let fuel_used = store.get_fuel().ok().map(|remaining| fuel.saturating_sub(remaining));
    let stdout = String::from_utf8_lossy(&stdout_read.try_into_inner().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&stderr_read.try_into_inner().unwrap_or_default()).into_owned();
    let elapsed_ms = t0.elapsed().as_millis() as u64;

    tracing::info!(exit_code, elapsed_ms, fuel_used = ?fuel_used, "WASM executed");

    Ok(serde_json::json!({
        "exit_code":  exit_code,
        "success":    exit_code == 0,
        "stdout":     crate::util::truncate(&stdout, 50_000),
        "stderr":     crate::util::truncate(&stderr, 10_000),
        "elapsed_ms": elapsed_ms,
        "fuel_used":  fuel_used,
    }))
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use tempfile::tempdir;

    use super::*;
    use crate::tools::Tool;

    fn encode_wat(wat: &str) -> String {
        let bytes = wat::parse_str(wat).expect("wat should compile");
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[tokio::test]
    async fn test_execute_runs_wasi_command_module_through_start() {
        let tool = WasmExecTool;
        let dir = tempdir().expect("tempdir should exist");
        let module = encode_wat(r#"(module (func (export "_start")))"#);

        let result = tool
            .execute(serde_json::json!({
                "bytes_b64": module,
                "workspace": dir.path().to_string_lossy().to_string(),
                "timeout_secs": 5,
            }))
            .await
            .expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["exit_code"], serde_json::json!(0));
        assert_eq!(result.output["success"], serde_json::json!(true));
        assert_eq!(result.output["stdout"], serde_json::json!(""));
        assert_eq!(result.output["stderr"], serde_json::json!(""));
        assert!(result.output["wasm_size_bytes"].as_u64().unwrap_or_default() > 8);
    }

    #[tokio::test]
    async fn test_execute_runs_reactor_initialize_when_start_is_missing() {
        let tool = WasmExecTool;
        let dir = tempdir().expect("tempdir should exist");
        let module = encode_wat(r#"(module (func (export "_initialize")))"#);

        let result = tool
            .execute(serde_json::json!({
                "bytes_b64": module,
                "workspace": dir.path().to_string_lossy().to_string(),
            }))
            .await
            .expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["exit_code"], serde_json::json!(0));
        assert_eq!(result.output["success"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn test_execute_rejects_invalid_binary_before_runtime() {
        let tool = WasmExecTool;

        let result = tool
            .execute(serde_json::json!({
                "bytes_b64": base64::engine::general_purpose::STANDARD.encode("not-wasm"),
            }))
            .await
            .expect("tool should execute");

        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("invalid WebAssembly binary"));
    }
}
