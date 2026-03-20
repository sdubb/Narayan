//! wasm_call — Call a specific exported function in a WASM module directly.
//!
//! Unlike wasm_exec (which runs _start / the full WASI command), wasm_call
//! targets a named export. Useful for:
//!   - Reactor modules (library-style WASM without _start)
//!   - Calling specific functions for computation
//!   - Running pure functions with typed i32/i64/f32/f64 arguments

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct WasmCallTool;

#[async_trait]
impl Tool for WasmCallTool {
    fn name(&self) -> &str {
        "wasm_call"
    }
    fn description(&self) -> &str {
        "Call a specific exported function in a WebAssembly module by name. \
         Supports i32, i64, f32, f64 arguments and return values. \
         For WASI command modules with _start, use wasm_exec instead."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::optional("path", "string", "Path to .wasm file."),
            ParameterSchema::optional("bytes_b64", "string", "Base64-encoded .wasm bytes."),
            ParameterSchema::required("function", "string", "Name of the exported function to call."),
            ParameterSchema::optional("args", "array", "Function arguments as numbers or strings ('1', '3.14')."),
            ParameterSchema::optional("fuel", "integer", "Max instructions. Omit for unlimited."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        use base64::Engine;

        let bytes: Vec<u8> = if let Some(p) = args["path"].as_str() {
            match tokio::fs::read(p).await {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::err(format!("read '{}': {}", p, e))),
            }
        } else if let Some(b64) = args["bytes_b64"].as_str() {
            match base64::engine::general_purpose::STANDARD.decode(b64) {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::err(format!("base64: {}", e))),
            }
        } else {
            return Ok(ToolResult::err("'path' or 'bytes_b64' required"));
        };

        let func_name = match args["function"].as_str() {
            Some(f) => f.to_string(),
            None => return Ok(ToolResult::err("'function' is required")),
        };

        let fuel = args["fuel"].as_u64();
        let raw_args: Vec<serde_json::Value> = args["args"].as_array().cloned().unwrap_or_default();

        let join = tokio::task::spawn_blocking(move || call_wasm_func(bytes, func_name, raw_args, fuel));

        match join.await {
            Ok(Ok(v)) => Ok(ToolResult::ok(v)),
            Ok(Err(e)) => Ok(ToolResult::err(format!("wasm_call error: {}", e))),
            Err(e) => Ok(ToolResult::err(format!("thread panic: {}", e))),
        }
    }
}

fn call_wasm_func(
    bytes: Vec<u8>,
    func_name: String,
    raw_args: Vec<serde_json::Value>,
    fuel: Option<u64>,
) -> anyhow::Result<serde_json::Value> {
    use std::time::Instant;

    use wasmtime::*;

    let t0 = Instant::now();
    let mut cfg = Config::new();
    cfg.async_support(false);
    if fuel.is_some() {
        cfg.consume_fuel(true);
    }
    let engine = Engine::new(&cfg)?;

    let module = Module::from_binary(&engine, &bytes)?;
    let mut store: Store<()> = Store::new(&engine, ());
    if let Some(f) = fuel {
        store.set_fuel(f)?;
    }
    let linker: Linker<()> = Linker::new(&engine);
    let instance = linker.instantiate(&mut store, &module)?;

    let func = instance
        .get_func(&mut store, &func_name)
        .ok_or_else(|| anyhow::anyhow!("function '{}' not found — use wasm_inspect to list exports", func_name))?;

    let func_ty = func.ty(&store);

    // Convert JSON args to wasmtime Val based on function type signature
    let wasm_args: Vec<Val> = func_ty
        .params()
        .zip(raw_args.iter().chain(std::iter::repeat(&serde_json::Value::Null)))
        .map(|(ty, v)| json_to_val(v, ty))
        .collect::<Result<Vec<_>, _>>()?;

    let mut results = vec![Val::I32(0); func_ty.results().len()];
    func.call(&mut store, &wasm_args, &mut results)?;

    let fuel_used = fuel.and_then(|f| store.get_fuel().ok().map(|r| f.saturating_sub(r)));
    let elapsed_ms = t0.elapsed().as_millis() as u64;

    let result_vals: Vec<serde_json::Value> = results.iter().map(val_to_json).collect();
    let result = if result_vals.len() == 1 { result_vals[0].clone() } else { serde_json::json!(result_vals) };

    Ok(serde_json::json!({
        "function":   func_name,
        "result":     result,
        "elapsed_ms": elapsed_ms,
        "fuel_used":  fuel_used,
        "success":    true,
    }))
}

fn json_to_val(v: &serde_json::Value, ty: wasmtime::ValType) -> anyhow::Result<wasmtime::Val> {
    use wasmtime::ValType;
    match ty {
        ValType::I32 => Ok(wasmtime::Val::I32(
            v.as_i64().unwrap_or_else(|| v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0)) as i32,
        )),
        ValType::I64 => {
            Ok(wasmtime::Val::I64(v.as_i64().unwrap_or_else(|| v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0))))
        }
        ValType::F32 => Ok(wasmtime::Val::F32(
            (v.as_f64().unwrap_or_else(|| v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0)) as f32).to_bits(),
        )),
        ValType::F64 => Ok(wasmtime::Val::F64(
            v.as_f64().unwrap_or_else(|| v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0)).to_bits(),
        )),
        other => anyhow::bail!("unsupported WASM type {:?} — only i32/i64/f32/f64 supported", other),
    }
}

fn val_to_json(v: &wasmtime::Val) -> serde_json::Value {
    match v {
        wasmtime::Val::I32(n) => serde_json::json!(n),
        wasmtime::Val::I64(n) => serde_json::json!(n),
        wasmtime::Val::F32(b) => serde_json::json!(f32::from_bits(*b)),
        wasmtime::Val::F64(b) => serde_json::json!(f64::from_bits(*b)),
        other => serde_json::json!(format!("{:?}", other)),
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::*;
    use crate::tools::Tool;

    fn add_module_b64() -> String {
        let bytes = wat::parse_str(
            r#"(module
                (func (export "add") (param i32 i32) (result i32)
                    local.get 0
                    local.get 1
                    i32.add)
            )"#,
        )
        .expect("wat should compile");
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[tokio::test]
    async fn test_execute_calls_exported_function_and_coerces_args() {
        let tool = WasmCallTool;

        let result = tool
            .execute(serde_json::json!({
                "bytes_b64": add_module_b64(),
                "function": "add",
                "args": ["2", 3],
                "fuel": 10_000,
            }))
            .await
            .expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["function"], serde_json::json!("add"));
        assert_eq!(result.output["result"], serde_json::json!(5));
        assert_eq!(result.output["success"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn test_execute_reports_missing_function_cleanly() {
        let tool = WasmCallTool;

        let result = tool
            .execute(serde_json::json!({
                "bytes_b64": add_module_b64(),
                "function": "missing_export",
            }))
            .await
            .expect("tool should execute");

        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("function 'missing_export' not found"));
    }

    #[tokio::test]
    async fn test_execute_requires_source_and_function() {
        let tool = WasmCallTool;

        let no_source = tool.execute(serde_json::json!({ "function": "add" })).await.expect("tool should execute");
        assert!(!no_source.success);
        assert!(no_source.error.unwrap_or_default().contains("'path' or 'bytes_b64' required"));

        let no_function =
            tool.execute(serde_json::json!({ "bytes_b64": add_module_b64() })).await.expect("tool should execute");
        assert!(!no_function.success);
        assert!(no_function.error.unwrap_or_default().contains("'function' is required"));
    }
}
