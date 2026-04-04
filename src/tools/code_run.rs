//! code_run — Execute code snippets in Python, Node.js, Ruby, Deno, or Bash.
//! Writes code to a temp file and runs the correct interpreter.
//! Safer than shell for structured code execution — no PATH injection.

use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_integer};

const MAX_OUTPUT: usize = 1_048_576;

pub struct CodeRunTool;

#[async_trait]
impl Tool for CodeRunTool {
    fn name(&self) -> &str {
        "code_run"
    }
    fn description(&self) -> &str {
        "Execute a code snippet in Python 3, Node.js, Deno, Ruby, Bash, or Bun."
    }
    fn input_contract(&self) -> Option<String> {
        Some(
            "{ code, language, stdin?, packages?, timeout_secs?, workspace?, env? }. language must be one of python, node, deno, bun, ruby, or bash.".into(),
        )
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ stdout, stderr, exit_code, elapsed_ms, language }. Non-zero exits return success=false with stderr in error.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use for structured snippets, quick scripts, and small deterministic code tasks that fit one of the supported runtimes.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Do not use for multi-file applications, long-running services, or workflows better expressed as typed data_engine operations.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("code", "string", "Source code to execute."),
            ParameterSchema::required("language", "string", "Runtime: python | node | deno | ruby | bash | bun"),
            ParameterSchema::optional("stdin", "string", "Data to pass on stdin."),
            ParameterSchema::optional("packages", "array", "Packages to install before running (pip/npm/gem)."),
            ParameterSchema::optional("timeout_secs", "integer", "Max runtime in seconds (default: 60, max: 300)."),
            ParameterSchema::optional("workspace", "string", "Working directory (default: current dir)."),
            ParameterSchema::optional("env", "object", "Extra environment variables."),
        ]
    }



    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["stdout", "stderr", "exit_code", "elapsed_ms", "language"],
            "properties": {
                "stdout": schema_string(),
                "stderr": schema_string(),
                "exit_code": schema_integer(),
                "elapsed_ms": schema_integer(),
                "language": schema_string(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let code = match args["code"].as_str() {
            Some(c) => c,
            None => return Ok(ToolResult::err("'code' required")),
        };
        let lang = match args["language"].as_str() {
            Some(l) => l,
            None => return Ok(ToolResult::err("'language' required")),
        };
        let timeout = args["timeout_secs"].as_u64().unwrap_or(60).min(300);
        let stdin_s = args["stdin"].as_str().map(String::from);
        let workspace = args["workspace"].as_str().unwrap_or(".").to_string();

        let (ext, interpreter, install_cmd) = match lang.to_lowercase().as_str() {
            "python" | "python3" | "py" => ("py", vec!["python3"], Some("pip install -q")),
            "node" | "nodejs" | "js" => ("js", vec!["node"], Some("npm install -g")),
            "deno" => ("ts", vec!["deno", "run"], None),
            "bun" => ("js", vec!["bun", "run"], None),
            "ruby" | "rb" => ("rb", vec!["ruby"], Some("gem install")),
            "bash" | "sh" => ("sh", vec!["bash"], None),
            other => {
                return Ok(ToolResult::err(format!(
                    "unsupported language '{}'. Use: python | node | deno | bun | ruby | bash",
                    other
                )))
            }
        };

        // Install packages first
        if let (Some(pkgs), Some(install)) = (args["packages"].as_array(), install_cmd) {
            let pkg_list: Vec<&str> = pkgs.iter().filter_map(|v| v.as_str()).collect();
            if !pkg_list.is_empty() {
                let install_cmd_str = format!("{} {}", install, pkg_list.join(" "));
                let _ = Command::new("sh").arg("-c").arg(&install_cmd_str).output().await;
            }
        }

        // Write code to temp file
        let tmp = std::env::temp_dir().join(format!("narayan_code_{}.{}", crate::util::new_id(), ext));
        tokio::fs::write(&tmp, code).await?;

        // Build command
        let mut cmd = Command::new(&interpreter[0]);
        for arg in &interpreter[1..] {
            cmd.arg(arg);
        }
        cmd.arg(&tmp).current_dir(&workspace);

        // Safe env — no secrets leaked
        cmd.env_clear();
        for var in &["PATH", "HOME", "TMPDIR", "LANG"] {
            if let Ok(v) = std::env::var(var) {
                cmd.env(var, v);
            }
        }
        if let Some(env) = args["env"].as_object() {
            for (k, v) in env {
                if let Some(val) = v.as_str() {
                    cmd.env(k, val);
                }
            }
        }

        if stdin_s.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        }

        let start = std::time::Instant::now();

        let out = tokio::time::timeout(Duration::from_secs(timeout), async {
            if let Some(ref input) = stdin_s {
                cmd.stdin(std::process::Stdio::piped());
                let mut child = cmd.spawn()?;
                if let Some(mut stdin_h) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    stdin_h.write_all(input.as_bytes()).await.ok();
                }
                child.wait_with_output().await
            } else {
                cmd.output().await
            }
        })
        .await;

        // Clean up temp file
        tokio::fs::remove_file(&tmp).await.ok();

        let elapsed_ms = start.elapsed().as_millis() as u64;

        match out {
            Ok(Ok(o)) => {
                let mut stdout = String::from_utf8_lossy(&o.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&o.stderr).into_owned();
                if stdout.len() > MAX_OUTPUT {
                    stdout.truncate(MAX_OUTPUT);
                    stdout.push_str("\n[truncated]");
                }
                if stderr.len() > MAX_OUTPUT {
                    stderr.truncate(MAX_OUTPUT);
                }
                let code = o.status.code().unwrap_or(-1);
                let ok = o.status.success();
                let out = serde_json::json!({"stdout": stdout, "stderr": stderr, "exit_code": code, "elapsed_ms": elapsed_ms, "language": lang});
                if ok {
                    Ok(ToolResult::ok(out))
                } else {
                    Ok(ToolResult { success: false, output: out, error: Some(stderr.trim().to_string()) })
                }
            }
            Ok(Err(e)) => Ok(ToolResult::err(format!("spawn failed — is {} installed? {}", lang, e))),
            Err(_) => Ok(ToolResult::err(format!("timed out after {}s", timeout))),
        }
    }
}
