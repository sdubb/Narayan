//! `run_registered_wasm` - execute tenant-registered WASM tools with strict limits.
//!
//! Unlike `wasm_exec` (raw module execution), this tool only runs modules that
//! were pre-registered by the tenant and stored in Postgres. Each registered tool
//! carries explicit permissions and hard resource limits (memory, fuel, timeout).

use std::{collections::HashSet, path::Path, sync::Arc, time::Instant};

use async_trait::async_trait;
use chrono::Utc;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimitsBuilder};
use wasmtime_wasi::{
    pipe::{MemoryInputPipe, MemoryOutputPipe},
    preview1::{self, WasiP1Ctx},
    DirPerms, FilePerms, WasiCtxBuilder,
};

use crate::{
    agent::definition::{TenantWasmTool, WasmToolRunAudit},
    storage::PostgresStore,
    tools::{ParameterSchema, Tool, ToolResult},
};

const TOOL_NAME: &str = "run_registered_wasm";
const MAX_STDOUT_CAPTURE_BYTES: usize = 512 * 1024;
const MAX_STDERR_CAPTURE_BYTES: usize = 256 * 1024;

pub struct RunRegisteredWasmTool {
    store: Option<Arc<PostgresStore>>,
}

impl RunRegisteredWasmTool {
    pub fn new() -> Self {
        Self { store: None }
    }

    pub fn with_store(mut self, store: Arc<PostgresStore>) -> Self {
        self.store = Some(store);
        self
    }
}

#[async_trait]
impl Tool for RunRegisteredWasmTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Run a tenant-registered WASM tool by name. \
         Enforces strict per-tool limits (memory, fuel, timeout) and sandbox permissions. \
         Use this for user-specific deterministic logic with low CPU/memory usage."
    }

    fn category(&self) -> &'static str {
        "other"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("tool_name", "string", "Registered WASM tool name."),
            ParameterSchema::optional("input", "object", "JSON input payload. If provided, serialized to stdin."),
            ParameterSchema::optional("stdin", "string", "Raw stdin text. Overrides input serialization."),
            ParameterSchema::optional("args", "array", "CLI arguments passed to the module."),
            ParameterSchema::optional("env", "object", "Optional env vars. Keys outside allowlist are ignored."),
            ParameterSchema::optional("workspace", "string", "Workspace path (injected by executor)."),
            ParameterSchema::optional(
                "timeout_secs",
                "integer",
                "Requested timeout, capped by the registered tool timeout limit.",
            ),
            ParameterSchema::optional("fuel", "integer", "Requested fuel, capped by the registered tool fuel limit."),
            ParameterSchema::optional(
                "memory_limit_bytes",
                "integer",
                "Requested memory cap, capped by the registered tool memory limit.",
            ),
            ParameterSchema::optional("tenant_id", "string", "Injected by executor."),
            ParameterSchema::optional("agent_id", "string", "Injected by executor for audit."),
            ParameterSchema::optional("role_id", "string", "Injected by executor for audit."),
            ParameterSchema::optional("goal_instance_id", "string", "Injected by executor for audit."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let Some(store) = self.store.as_ref() else {
            return Ok(ToolResult::err("store not configured for run_registered_wasm"));
        };

        let tenant_id = args["tenant_id"].as_str().unwrap_or("").trim();
        if tenant_id.is_empty() {
            return Ok(ToolResult::err("tenant_id is required (injected by executor)"));
        }

        let tool_name = args["tool_name"].as_str().unwrap_or("").trim().to_string();
        if tool_name.is_empty() {
            return Ok(ToolResult::err("'tool_name' is required"));
        }

        let Some((tool_meta, module_bytes)) = store.get_tenant_wasm_tool_with_module(tenant_id, &tool_name).await?
        else {
            return Ok(ToolResult::err(format!("registered WASM tool '{}' not found", tool_name)));
        };

        if !tool_meta.enabled {
            return Ok(ToolResult::err(format!("registered WASM tool '{}' is disabled", tool_name)));
        }
        if module_bytes.len() < 4 || &module_bytes[..4] != b"\0asm" {
            return Ok(ToolResult::err(format!("registered WASM tool '{}' has invalid module bytes", tool_name)));
        }

        let limits = tool_meta.limits.clamped();
        let effective_timeout =
            args["timeout_secs"].as_u64().unwrap_or(limits.timeout_secs).clamp(1, limits.timeout_secs);
        let effective_fuel = args["fuel"].as_u64().unwrap_or(limits.max_fuel).clamp(100_000, limits.max_fuel);
        let effective_memory = args["memory_limit_bytes"]
            .as_u64()
            .unwrap_or(limits.max_memory_bytes)
            .clamp(1 * 1024 * 1024, limits.max_memory_bytes);

        let stdin_data = build_stdin_payload(&args);
        let cli_args: Vec<String> = args["args"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|value| value.as_str().map(String::from))
            .collect();
        let env_vars = filter_allowed_env(&tool_meta, &args["env"]);

        let workspace = args["workspace"].as_str().map(str::trim).filter(|v| !v.is_empty()).map(String::from);

        let tool_version = tool_meta.version;
        let run_started = Instant::now();
        let join = tokio::task::spawn_blocking(move || {
            execute_registered_module(
                module_bytes,
                stdin_data,
                cli_args,
                env_vars,
                workspace,
                tool_meta,
                effective_memory,
                effective_fuel,
            )
        });

        let (success, output, error, elapsed_ms, fuel_used) =
            match tokio::time::timeout(std::time::Duration::from_secs(effective_timeout), join).await {
                Ok(Ok(Ok(run))) => (run.success, run.output, None, run.elapsed_ms, run.fuel_used),
                Ok(Ok(Err(err))) => (
                    false,
                    serde_json::Value::Null,
                    Some(format!("WASM execution error: {}", err)),
                    run_started.elapsed().as_millis() as u64,
                    None,
                ),
                Ok(Err(join_err)) => (
                    false,
                    serde_json::Value::Null,
                    Some(format!("WASM worker panic: {}", join_err)),
                    run_started.elapsed().as_millis() as u64,
                    None,
                ),
                Err(_) => (
                    false,
                    serde_json::Value::Null,
                    Some(format!("timed out after {}s", effective_timeout)),
                    run_started.elapsed().as_millis() as u64,
                    None,
                ),
            };

        let audit = WasmToolRunAudit {
            id: uuid::Uuid::new_v4().to_string(),
            tenant_id: tenant_id.to_string(),
            tool_name: tool_name.clone(),
            tool_version,
            agent_id: args["agent_id"].as_str().map(String::from),
            role_id: args["role_id"].as_str().map(String::from),
            goal_instance_id: args["goal_instance_id"].as_str().map(String::from),
            success,
            elapsed_ms,
            fuel_used,
            memory_limit_bytes: effective_memory,
            error: error.clone(),
            created_at: Utc::now(),
        };

        if let Err(e) = store.insert_wasm_tool_run_audit(&audit).await {
            tracing::warn!(tenant_id = %tenant_id, tool = %tool_name, error = %e, "failed to write WASM run audit");
        }
        if success {
            let _ = store.touch_tenant_wasm_tool_last_used(tenant_id, &tool_name).await;
        }

        if success {
            Ok(ToolResult::ok(output))
        } else {
            Ok(ToolResult { success: false, output, error: Some(error.unwrap_or_else(|| "WASM run failed".into())) })
        }
    }
}

struct RegisteredWasmRunResult {
    success: bool,
    output: serde_json::Value,
    elapsed_ms: u64,
    fuel_used: Option<u64>,
}

struct WasmStoreState {
    wasi: WasiP1Ctx,
    limits: wasmtime::StoreLimits,
}

fn execute_registered_module(
    module_bytes: Vec<u8>,
    stdin_data: String,
    args: Vec<String>,
    env_vars: Vec<(String, String)>,
    workspace: Option<String>,
    tool_meta: TenantWasmTool,
    memory_limit_bytes: u64,
    fuel_limit: u64,
) -> anyhow::Result<RegisteredWasmRunResult> {
    let started = Instant::now();

    let mut cfg = Config::new();
    cfg.async_support(false);
    cfg.consume_fuel(true);
    let engine = Engine::new(&cfg)?;

    let stdout_pipe = MemoryOutputPipe::new(MAX_STDOUT_CAPTURE_BYTES);
    let stderr_pipe = MemoryOutputPipe::new(MAX_STDERR_CAPTURE_BYTES);
    let stdout_read = stdout_pipe.clone();
    let stderr_read = stderr_pipe.clone();

    let mut wasi_builder = WasiCtxBuilder::new();
    if !stdin_data.is_empty() {
        wasi_builder.stdin(MemoryInputPipe::new(stdin_data.into_bytes()));
    }
    wasi_builder.stdout(stdout_pipe).stderr(stderr_pipe);
    wasi_builder.args(&args);
    for (key, value) in &env_vars {
        wasi_builder.env(key, value);
    }

    if let Some(workspace_path) = workspace {
        let ws = Path::new(&workspace_path);
        if ws.exists() && (tool_meta.permissions.allow_workspace_read || tool_meta.permissions.allow_workspace_write) {
            let (dir_perms, file_perms) = if tool_meta.permissions.allow_workspace_write {
                (DirPerms::all(), FilePerms::all())
            } else {
                (DirPerms::READ, FilePerms::READ)
            };
            wasi_builder.preopened_dir(ws, "/workspace", dir_perms, file_perms)?;
        }
    }

    let wasm_limits = StoreLimitsBuilder::new()
        .memory_size((memory_limit_bytes.min(usize::MAX as u64)) as usize)
        .instances(1)
        .tables(2)
        .memories(1)
        .build();

    let state = WasmStoreState { wasi: wasi_builder.build_p1(), limits: wasm_limits };
    let mut store = Store::new(&engine, state);
    store.limiter(|state| &mut state.limits);
    store.set_fuel(fuel_limit)?;

    let mut linker: Linker<WasmStoreState> = Linker::new(&engine);
    preview1::add_to_linker_sync(&mut linker, |state| &mut state.wasi)?;

    let module = Module::from_binary(&engine, &module_bytes)?;
    let instance = linker.instantiate(&mut store, &module)?;

    let exit_code: i32 = match instance.get_typed_func::<(), ()>(&mut store, "_start") {
        Ok(func) => match func.call(&mut store, ()) {
            Ok(()) => 0,
            Err(error) => error.downcast_ref::<wasmtime_wasi::I32Exit>().map(|exit| exit.0).unwrap_or(1),
        },
        Err(_) => {
            if let Ok(init) = instance.get_typed_func::<(), ()>(&mut store, "_initialize") {
                init.call(&mut store, ())?;
            }
            0
        }
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let fuel_used = store.get_fuel().ok().map(|remaining| fuel_limit.saturating_sub(remaining));
    let stdout = String::from_utf8_lossy(&stdout_read.try_into_inner().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&stderr_read.try_into_inner().unwrap_or_default()).into_owned();

    let output = serde_json::json!({
        "tool_name": tool_meta.name,
        "tool_version": tool_meta.version,
        "success": exit_code == 0,
        "exit_code": exit_code,
        "stdout": crate::util::truncate(&stdout, 50_000),
        "stderr": crate::util::truncate(&stderr, 10_000),
        "elapsed_ms": elapsed_ms,
        "fuel_used": fuel_used,
        "fuel_limit": fuel_limit,
        "memory_limit_bytes": memory_limit_bytes,
    });

    Ok(RegisteredWasmRunResult { success: exit_code == 0, output, elapsed_ms, fuel_used })
}

fn build_stdin_payload(args: &serde_json::Value) -> String {
    if let Some(stdin) = args["stdin"].as_str() {
        return stdin.to_string();
    }

    if args.get("input").is_some() {
        let input = &args["input"];
        if let Some(s) = input.as_str() {
            s.to_string()
        } else {
            serde_json::to_string(input).unwrap_or_default()
        }
    } else {
        String::new()
    }
}

fn filter_allowed_env(tool: &TenantWasmTool, env_value: &serde_json::Value) -> Vec<(String, String)> {
    if !tool.permissions.allow_env {
        return Vec::new();
    }

    let allow: HashSet<String> = tool.permissions.allowed_env_keys.iter().map(|key| key.to_ascii_lowercase()).collect();
    env_value
        .as_object()
        .map(|values| {
            values
                .iter()
                .filter_map(|(key, value)| {
                    if !allow.contains(&key.to_ascii_lowercase()) {
                        return None;
                    }
                    value.as_str().map(|s| (key.clone(), s.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}
