//! ssh_exec — Execute commands on remote hosts via SSH using `russh`.
//! Pure async Rust — no libssh2 or system ssh binary needed.

use std::sync::Arc;

use async_trait::async_trait;
use russh::{client, keys::PublicKey};
use russh::client::AuthResult;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct SshExecTool;

struct NoVerify;
impl client::Handler for NoVerify {
    type Error = anyhow::Error;
    #[allow(refining_impl_trait)]
    fn check_server_key(
        &mut self,
        _key: &PublicKey,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = core::result::Result<bool, Self::Error>> + Send>> {
        Box::pin(async { Ok(true) })
    }
}

#[async_trait]
impl Tool for SshExecTool {
    fn name(&self) -> &str {
        "ssh_exec"
    }
    fn description(&self) -> &str {
        "Execute a command on a remote host via SSH. \
         Authenticate with a private key or password stored via request_credential."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("host", "string", "Remote hostname or IP."),
            ParameterSchema::required("command", "string", "Shell command to run on remote host."),
            ParameterSchema::optional("port", "integer", "SSH port (default: 22)."),
            ParameterSchema::optional("username", "string", "SSH username (default: 'ubuntu')."),
            ParameterSchema::optional("key_cred", "string", "Credential key holding PEM private key (preferred)."),
            ParameterSchema::optional("password_cred", "string", "Credential key holding password (fallback)."),
            ParameterSchema::optional("timeout_secs", "integer", "Connection + execution timeout (default: 30)."),
            ParameterSchema::optional("stdin", "string", "Data to send to command stdin."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let host = match args["host"].as_str() {
            Some(h) => h,
            None => return Ok(ToolResult::err("'host' required")),
        };
        let command = match args["command"].as_str() {
            Some(c) => c,
            None => return Ok(ToolResult::err("'command' required")),
        };
        let port = args["port"].as_u64().unwrap_or(22) as u16;
        let username = args["username"].as_str().unwrap_or("ubuntu");
        let timeout = args["timeout_secs"].as_u64().unwrap_or(30);
        let stdin_s = args["stdin"].as_str().map(String::from);

        // Resolve credentials
        let key_pem = args["key_cred"]
            .as_str()
            .and_then(|k| crate::tools::memory_store_internal::get(&format!("credential:{}", k)));
        let password = args["password_cred"]
            .as_str()
            .and_then(|k| crate::tools::memory_store_internal::get(&format!("credential:{}", k)));

        if key_pem.is_none() && password.is_none() {
            return Ok(ToolResult::err("'key_cred' or 'password_cred' is required"));
        }

        let addr = format!("{}:{}", host, port);
        let config = Arc::new(client::Config::default());
        let handler = NoVerify;

        let start = std::time::Instant::now();

        let mut session =
            tokio::time::timeout(std::time::Duration::from_secs(timeout), client::connect(config, &addr, handler))
                .await
                .map_err(|_| anyhow::anyhow!("connection timed out after {}s", timeout))?
                .map_err(|e| anyhow::anyhow!("SSH connect to '{}': {}", addr, e))?;

        // Authenticate
        let auth_res = if let Some(ref pem) = key_pem {
            let private_key =
                russh::keys::decode_secret_key(pem, None).map_err(|e| anyhow::anyhow!("parse private key: {}", e))?;
            // Wrap PrivateKey in Arc and use PrivateKeyWithHashAlg with SHA256
            let keypair = russh::keys::PrivateKeyWithHashAlg::new(
                Arc::new(private_key), 
                Some(russh::keys::HashAlg::Sha256)
            );
            session
                .authenticate_publickey(username, keypair)
                .await
                .map_err(|e| anyhow::anyhow!("pubkey auth: {}", e))?
        } else if let Some(ref pass) = password {
            session.authenticate_password(username, pass).await.map_err(|e| anyhow::anyhow!("password auth: {}", e))?
        } else {
            return Ok(ToolResult::err("SSH authentication failed — check credentials"));
        };

        if !matches!(auth_res, AuthResult::Success) {
            return Ok(ToolResult::err("SSH authentication failed — check credentials"));
        }

        // Open channel and exec
        let mut channel = session.channel_open_session().await.map_err(|e| anyhow::anyhow!("open channel: {}", e))?;
        channel.exec(true, command).await.map_err(|e| anyhow::anyhow!("exec: {}", e))?;

        // Send stdin if provided
        if let Some(ref input) = stdin_s {
            channel.data(input.as_bytes()).await.ok();
            channel.eof().await.ok();
        }

        // Collect output
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code = 0i32;

        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Data { data }) => stdout.extend_from_slice(&data),
                Some(russh::ChannelMsg::ExtendedData { data, .. }) => stderr.extend_from_slice(&data),
                Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = exit_status as i32;
                }
                Some(russh::ChannelMsg::Eof) | None => break,
                _ => {}
            }
        }

        let _ = session.disconnect(russh::Disconnect::ByApplication, "done", "en").await;

        let stdout_s = String::from_utf8_lossy(&stdout).into_owned();
        let stderr_s = String::from_utf8_lossy(&stderr).into_owned();
        let elapsed = start.elapsed().as_millis() as u64;
        let ok = exit_code == 0;

        let out = serde_json::json!({
            "host":        host,
            "command":     command,
            "stdout":      crate::util::truncate(&stdout_s, 50_000),
            "stderr":      crate::util::truncate(&stderr_s, 10_000),
            "exit_code":   exit_code,
            "elapsed_ms":  elapsed,
        });
        if ok {
            Ok(ToolResult::ok(out))
        } else {
            Ok(ToolResult { success: false, output: out, error: Some(stderr_s.trim().to_string()) })
        }
    }
}


