//! wasm_compile — Compile WAT (WebAssembly Text format) to binary .wasm.
//!
//! Agents can write human-readable WAT then compile it to binary for wasm_exec.
//! Also validates existing .wasm files and reports module exports/imports.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct WasmCompileTool;

#[async_trait]
impl Tool for WasmCompileTool {
    fn name(&self) -> &str {
        "wasm_compile"
    }
    fn description(&self) -> &str {
        "Compile WAT (WebAssembly Text Format) source to a .wasm binary, \
         or validate and inspect an existing .wasm file. \
         Returns the base64-encoded binary and module metadata (exports, imports, size). \
         Use wasm_exec to run the result."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::optional("wat", "string", "WAT source text to compile to WASM."),
            ParameterSchema::optional("wat_path", "string", "Path to a .wat source file to compile."),
            ParameterSchema::optional("wasm_path", "string", "Path to an existing .wasm to validate and inspect."),
            ParameterSchema::optional("output", "string", "Output path for the compiled .wasm (optional)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        use base64::Engine;
        use wasmtime::Engine as WasmEngine;

        // ── Load WAT or WASM ──────────────────────────────────────────────
        let wasm_bytes: Vec<u8> = if let Some(wat_src) = args["wat"].as_str() {
            match wat::parse_str(wat_src) {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::err(format!("WAT parse error: {}", e))),
            }
        } else if let Some(wat_path) = args["wat_path"].as_str() {
            let src = match tokio::fs::read_to_string(wat_path).await {
                Ok(s) => s,
                Err(e) => return Ok(ToolResult::err(format!("read '{}': {}", wat_path, e))),
            };
            match wat::parse_str(&src) {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::err(format!("WAT parse error: {}", e))),
            }
        } else if let Some(wasm_path) = args["wasm_path"].as_str() {
            match tokio::fs::read(wasm_path).await {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::err(format!("read '{}': {}", wasm_path, e))),
            }
        } else {
            return Ok(ToolResult::err("one of 'wat', 'wat_path', or 'wasm_path' is required"));
        };

        if wasm_bytes.len() < 4 || &wasm_bytes[..4] != b"\0asm" {
            return Ok(ToolResult::err("resulting binary is not valid WASM"));
        }

        // ── Validate via wasmtime ─────────────────────────────────────────
        let engine = WasmEngine::default();
        let module = match wasmtime::Module::from_binary(&engine, &wasm_bytes) {
            Ok(m) => m,
            Err(e) => return Ok(ToolResult::err(format!("WASM validation failed: {}", e))),
        };

        // ── Collect module info ────────────────────────────────────────────
        let exports: Vec<serde_json::Value> = module
            .exports()
            .map(|e| serde_json::json!({ "name": e.name(), "type": format!("{:?}", e.ty()) }))
            .collect();
        let imports: Vec<serde_json::Value> = module
            .imports()
            .map(|i| serde_json::json!({ "module": i.module(), "name": i.name(), "type": format!("{:?}", i.ty()) }))
            .collect();

        // ── Optionally save to disk ────────────────────────────────────────
        if let Some(out_path) = args["output"].as_str() {
            if let Some(parent) = std::path::Path::new(out_path).parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(out_path, &wasm_bytes)
                .await
                .map_err(|e| anyhow::anyhow!("write '{}': {}", out_path, e))?;
        }

        let b64 = base64::engine::general_purpose::STANDARD.encode(&wasm_bytes);

        Ok(ToolResult::ok(serde_json::json!({
            "valid":       true,
            "size_bytes":  wasm_bytes.len(),
            "exports":     exports,
            "imports":     imports,
            "bytes_b64":   b64,
            "tip":         "Pass bytes_b64 directly to wasm_exec to run this module.",
        })))
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use tempfile::tempdir;

    use super::*;
    use crate::tools::Tool;

    fn sample_wat() -> &'static str {
        r#"(module
            (func (export "add") (param i32 i32) (result i32)
                local.get 0
                local.get 1
                i32.add)
        )"#
    }

    #[tokio::test]
    async fn test_execute_compiles_wat_and_reports_exports() {
        let tool = WasmCompileTool;

        let result = tool.execute(serde_json::json!({ "wat": sample_wat() })).await.expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["valid"], serde_json::json!(true));
        assert!(result.output["size_bytes"].as_u64().unwrap_or_default() > 8);
        assert!(result.output["bytes_b64"].as_str().unwrap_or_default().len() > 10);
        assert!(result.output["exports"]
            .as_array()
            .map(|exports| exports.iter().any(|export| export["name"] == serde_json::json!("add")))
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn test_execute_writes_output_file_and_can_reinspect_saved_wasm() {
        let tool = WasmCompileTool;
        let dir = tempdir().expect("tempdir should exist");
        let output = dir.path().join("nested").join("module.wasm");

        let compiled = tool
            .execute(serde_json::json!({
                "wat": sample_wat(),
                "output": output.to_string_lossy().to_string(),
            }))
            .await
            .expect("compile should succeed");

        assert!(compiled.success);
        assert!(output.exists());

        let saved_bytes = tokio::fs::read(&output).await.expect("saved wasm should be readable");
        let encoded = compiled.output["bytes_b64"].as_str().expect("encoded bytes should exist");
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded).expect("base64 should decode");
        assert_eq!(decoded, saved_bytes);

        let reinspected = tool
            .execute(serde_json::json!({ "wasm_path": output.to_string_lossy().to_string() }))
            .await
            .expect("wasm validation should succeed");
        assert!(reinspected.success);
        assert_eq!(reinspected.output["valid"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn test_execute_rejects_invalid_wat_and_missing_input() {
        let tool = WasmCompileTool;

        let missing = tool.execute(serde_json::json!({})).await.expect("tool should execute");
        assert!(!missing.success);
        assert!(missing.error.unwrap_or_default().contains("one of 'wat', 'wat_path', or 'wasm_path'"));

        let invalid = tool.execute(serde_json::json!({ "wat": "(module (func" })).await.expect("tool should execute");
        assert!(!invalid.success);
        assert!(invalid.error.unwrap_or_default().contains("WAT parse error"));
    }
}
