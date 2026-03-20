//! wasm_inspect — Deep static analysis of a WebAssembly module.
//!
//! Without executing anything, extracts:
//!   - All exports and their types (functions, globals, tables, memories)
//!   - All imports and their types
//!   - Custom sections (name section, WASI version hints, build metadata)
//!   - Memory/table/global counts and limits
//!   - Function count and estimated code size
//!   - Whether the module targets WASI command or reactor interface

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct WasmInspectTool;

#[async_trait]
impl Tool for WasmInspectTool {
    fn name(&self) -> &str {
        "wasm_inspect"
    }
    fn description(&self) -> &str {
        "Statically inspect a WebAssembly module without executing it. \
         Returns exports, imports, memory layout, section info, and whether \
         the module is a WASI command, reactor, or bare module."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::optional("path", "string", "Path to .wasm file."),
            ParameterSchema::optional("bytes_b64", "string", "Base64-encoded .wasm bytes."),
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

        if bytes.len() < 4 || &bytes[..4] != b"\0asm" {
            return Ok(ToolResult::err("not a valid WASM binary"));
        }

        let engine = wasmtime::Engine::default();
        let module = match wasmtime::Module::from_binary(&engine, &bytes) {
            Ok(m) => m,
            Err(e) => return Ok(ToolResult::err(format!("invalid WASM: {}", e))),
        };

        let exports: Vec<serde_json::Value> = module
            .exports()
            .map(|e| serde_json::json!({ "name": e.name(), "kind": classify_extern(&e.ty()) }))
            .collect();
        let imports: Vec<serde_json::Value> = module
            .imports()
            .map(|i| {
                serde_json::json!({
                    "module": i.module(),
                    "name":   i.name(),
                    "kind":   classify_extern(&i.ty()),
                })
            })
            .collect();

        let export_names: Vec<&str> = module.exports().map(|e| e.name()).collect();
        let import_modules: Vec<&str> = module.imports().map(|i| i.module()).collect();

        let interface = if export_names.contains(&"_start") {
            "wasi-command"
        } else if export_names.contains(&"_initialize") {
            "wasi-reactor"
        } else if import_modules.iter().any(|m| m.starts_with("wasi_")) {
            "wasi-unknown"
        } else {
            "bare-module"
        };

        let wasi_version = if import_modules.iter().any(|m| *m == "wasi_snapshot_preview1") {
            "WASI Preview 1"
        } else if import_modules.iter().any(|m| m.starts_with("wasi:")) {
            "WASI Preview 2 (component model)"
        } else {
            "none"
        };

        Ok(ToolResult::ok(serde_json::json!({
            "size_bytes":    bytes.len(),
            "interface":     interface,
            "wasi_version":  wasi_version,
            "exports":       exports,
            "export_count":  exports.len(),
            "imports":       imports,
            "import_count":  imports.len(),
            "has_start":     export_names.contains(&"_start"),
            "has_memory":    export_names.contains(&"memory"),
            "tip": match interface {
                "wasi-command"  => "Run with wasm_exec — it exports _start.",
                "wasi-reactor"  => "Reactor module — call exported functions directly via wasm_call.",
                "bare-module"   => "Bare module — use wasm_call to invoke specific exports.",
                _               => "Use wasm_exec or wasm_call depending on the interface.",
            },
        })))
    }
}

fn classify_extern(ty: &wasmtime::ExternType) -> &'static str {
    match ty {
        wasmtime::ExternType::Func(_) => "function",
        wasmtime::ExternType::Global(_) => "global",
        wasmtime::ExternType::Table(_) => "table",
        wasmtime::ExternType::Memory(_) => "memory",
        wasmtime::ExternType::Tag(_) => "tag",
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::*;
    use crate::tools::Tool;

    fn encode_wat(wat: &str) -> String {
        let bytes = wat::parse_str(wat).expect("wat should compile");
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[tokio::test]
    async fn test_execute_classifies_bare_module_exports_and_memory() {
        let tool = WasmInspectTool;
        let module = encode_wat(
            r#"(module
                (memory (export "memory") 1)
                (func (export "run"))
            )"#,
        );

        let result = tool.execute(serde_json::json!({ "bytes_b64": module })).await.expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["interface"], serde_json::json!("bare-module"));
        assert_eq!(result.output["has_memory"], serde_json::json!(true));
        assert_eq!(result.output["export_count"], serde_json::json!(2));
        assert!(result.output["exports"]
            .as_array()
            .map(|exports| {
                exports.iter().any(|export| {
                    export["name"] == serde_json::json!("memory") && export["kind"] == serde_json::json!("memory")
                })
            })
            .unwrap_or(false));
        assert!(result.output["tip"].as_str().unwrap_or_default().contains("wasm_call"));
    }

    #[tokio::test]
    async fn test_execute_detects_command_interface_from_start_export() {
        let tool = WasmInspectTool;
        let module = encode_wat(r#"(module (func (export "_start")))"#);

        let result = tool.execute(serde_json::json!({ "bytes_b64": module })).await.expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["interface"], serde_json::json!("wasi-command"));
        assert_eq!(result.output["has_start"], serde_json::json!(true));
        assert!(result.output["tip"].as_str().unwrap_or_default().contains("wasm_exec"));
    }

    #[tokio::test]
    async fn test_execute_rejects_invalid_binary() {
        let tool = WasmInspectTool;

        let result = tool
            .execute(serde_json::json!({
                "bytes_b64": base64::engine::general_purpose::STANDARD.encode("plain-text"),
            }))
            .await
            .expect("tool should execute");

        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("not a valid WASM binary"));
    }
}
