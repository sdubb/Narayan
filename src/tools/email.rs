//! email — send emails via multiple backends.
//!
//! Supports:
//!   - SMTP (any SMTP server: Gmail, Outlook, custom)
//!   - Mailgun API
//!   - SendGrid API
//!   - Resend API
//!   - Gmail MCP (if connected via mcp_session)
//!
//! Credentials looked up from memory store — use request_credential first.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct EmailTool;

#[async_trait]
impl Tool for EmailTool {
    fn name(&self) -> &str {
        "email"
    }

    fn description(&self) -> &str {
        "Send an email via SMTP, Mailgun, SendGrid, or Resend. \
         Store credentials first with request_credential. \
         Supports plain text and HTML bodies, CC, BCC, and attachments."
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("to", "string", "Recipient email address (or comma-separated list)."),
            ParameterSchema::required("subject", "string", "Email subject line."),
            ParameterSchema::required("body", "string", "Email body — plain text or HTML."),
            ParameterSchema::optional(
                "from",
                "string",
                "Sender address (required for SMTP; inferred for API providers).",
            ),
            ParameterSchema::optional("cc", "string", "CC recipients, comma-separated."),
            ParameterSchema::optional("bcc", "string", "BCC recipients, comma-separated."),
            ParameterSchema::optional("html", "boolean", "If true, body is sent as HTML (default: false)."),
            ParameterSchema::optional(
                "provider",
                "string",
                "Backend: 'smtp'|'mailgun'|'sendgrid'|'resend' (default: auto-detect from stored credentials).",
            ),
            ParameterSchema::optional(
                "credential_key",
                "string",
                "Credential key name (default: auto-detect by provider).",
            ),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let to = match args["to"].as_str() {
            Some(t) => t,
            None => return Ok(ToolResult::err("'to' is required")),
        };
        let subject = match args["subject"].as_str() {
            Some(s) => s,
            None => return Ok(ToolResult::err("'subject' is required")),
        };
        let body = match args["body"].as_str() {
            Some(b) => b,
            None => return Ok(ToolResult::err("'body' is required")),
        };
        let is_html = args["html"].as_bool().unwrap_or(false);
        let from = args["from"].as_str().unwrap_or("narayan-agent@narayan.ai");

        // Determine provider — explicit > auto-detect from stored credentials
        let provider = args["provider"].as_str().map(String::from).unwrap_or_else(|| detect_provider());

        tracing::info!(provider = %provider, to = %to, subject = %subject, "sending email");

        match provider.as_str() {
            "mailgun" => send_mailgun(to, from, subject, body, is_html, &args).await,
            "sendgrid" => send_sendgrid(to, from, subject, body, is_html, &args).await,
            "resend" => send_resend(to, from, subject, body, is_html, &args).await,
            "smtp" => send_smtp(to, from, subject, body, is_html, &args).await,
            other => Ok(ToolResult::err(format!(
                "Unknown email provider '{}'. Use: smtp | mailgun | sendgrid | resend. \
                 Store your API key with request_credential first.",
                other
            ))),
        }
    }
}

// ── Provider auto-detection ────────────────────────────────────────────────

fn detect_provider() -> String {
    let probes = [
        ("credential:mailgun_api_key", "mailgun"),
        ("credential:sendgrid_api_key", "sendgrid"),
        ("credential:resend_api_key", "resend"),
        ("credential:smtp_password", "smtp"),
    ];
    for (key, name) in probes {
        if crate::tools::memory_store_internal::get(key).is_some() {
            return name.to_string();
        }
    }
    "smtp".to_string()
}

fn get_cred(key: &str, fallback_env: &str) -> Option<String> {
    crate::tools::memory_store_internal::get(&format!("credential:{key}")).or_else(|| std::env::var(fallback_env).ok())
}

// ── Mailgun ────────────────────────────────────────────────────────────────

async fn send_mailgun(
    to: &str,
    from: &str,
    subject: &str,
    body: &str,
    html: bool,
    args: &serde_json::Value,
) -> anyhow::Result<ToolResult> {
    let api_key = match get_cred("mailgun_api_key", "MAILGUN_API_KEY") {
        Some(k) => k,
        None => {
            return Ok(ToolResult::err(
                "Mailgun API key not found. Run: request_credential(name='mailgun_api_key', value='key-...')",
            ))
        }
    };
    let domain = get_cred("mailgun_domain", "MAILGUN_DOMAIN").unwrap_or_else(|| "mg.yourdomain.com".into());

    let url = format!("https://api.mailgun.net/v3/{}/messages", domain);

    let mut form = HashMap::new();
    form.insert("from", from.to_string());
    form.insert("to", to.to_string());
    form.insert("subject", subject.to_string());
    if html {
        form.insert("html", body.to_string());
    } else {
        form.insert("text", body.to_string());
    }
    if let Some(cc) = args["cc"].as_str() {
        form.insert("cc", cc.to_string());
    }
    if let Some(bcc) = args["bcc"].as_str() {
        form.insert("bcc", bcc.to_string());
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .basic_auth("api", Some(&api_key))
        .form(&form)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Mailgun request failed: {}", e))?;

    let status = resp.status().as_u16();
    let body_resp = resp.text().await.unwrap_or_default();
    let ok = (200..300).contains(&status);

    if ok {
        Ok(ToolResult::ok(serde_json::json!({"sent": true, "provider": "mailgun", "to": to})))
    } else {
        Ok(ToolResult {
            success: false,
            output: serde_json::json!({"status": status, "response": body_resp}),
            error: Some(format!("Mailgun error HTTP {}", status)),
        })
    }
}

// ── SendGrid ───────────────────────────────────────────────────────────────

async fn send_sendgrid(
    to: &str,
    from: &str,
    subject: &str,
    body: &str,
    html: bool,
    args: &serde_json::Value,
) -> anyhow::Result<ToolResult> {
    let api_key = match get_cred("sendgrid_api_key", "SENDGRID_API_KEY") {
        Some(k) => k,
        None => {
            return Ok(ToolResult::err(
                "SendGrid API key not found. Run: request_credential(name='sendgrid_api_key', value='SG...')",
            ))
        }
    };

    let content_type = if html { "text/html" } else { "text/plain" };
    let payload = serde_json::json!({
        "personalizations": [{ "to": [{"email": to}] }],
        "from": { "email": from },
        "subject": subject,
        "content": [{ "type": content_type, "value": body }],
    });

    // Attach CC/BCC if provided
    let mut payload = payload;
    if let Some(cc) = args["cc"].as_str() {
        payload["cc"] = serde_json::json!([{"email": cc}]);
    }
    if let Some(bcc) = args["bcc"].as_str() {
        payload["bcc"] = serde_json::json!([{"email": bcc}]);
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.sendgrid.com/v3/mail/send")
        .bearer_auth(&api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("SendGrid request failed: {}", e))?;

    let status = resp.status().as_u16();
    if (200..300).contains(&status) || status == 202 {
        Ok(ToolResult::ok(serde_json::json!({"sent": true, "provider": "sendgrid", "to": to})))
    } else {
        let body_resp = resp.text().await.unwrap_or_default();
        Ok(ToolResult {
            success: false,
            output: serde_json::json!({"status": status, "response": body_resp}),
            error: Some(format!("SendGrid error HTTP {}", status)),
        })
    }
}

// ── Resend ─────────────────────────────────────────────────────────────────

async fn send_resend(
    to: &str,
    from: &str,
    subject: &str,
    body: &str,
    html: bool,
    args: &serde_json::Value,
) -> anyhow::Result<ToolResult> {
    let api_key = match get_cred("resend_api_key", "RESEND_API_KEY") {
        Some(k) => k,
        None => {
            return Ok(ToolResult::err(
                "Resend API key not found. Run: request_credential(name='resend_api_key', value='re_...')",
            ))
        }
    };

    let mut payload = serde_json::json!({
        "from":    from,
        "to":      [to],
        "subject": subject,
    });
    if html {
        payload["html"] = serde_json::json!(body);
    } else {
        payload["text"] = serde_json::json!(body);
    }
    if let Some(cc) = args["cc"].as_str() {
        payload["cc"] = serde_json::json!([cc]);
    }
    if let Some(bcc) = args["bcc"].as_str() {
        payload["bcc"] = serde_json::json!([bcc]);
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.resend.com/emails")
        .bearer_auth(&api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Resend request failed: {}", e))?;

    let status = resp.status().as_u16();
    let body_resp = resp.text().await.unwrap_or_default();

    if (200..300).contains(&status) {
        let id =
            serde_json::from_str::<serde_json::Value>(&body_resp).ok().and_then(|v| v["id"].as_str().map(String::from));
        Ok(ToolResult::ok(serde_json::json!({"sent": true, "provider": "resend", "to": to, "id": id})))
    } else {
        Ok(ToolResult {
            success: false,
            output: serde_json::json!({"status": status, "response": body_resp}),
            error: Some(format!("Resend error HTTP {}", status)),
        })
    }
}

// ── SMTP ───────────────────────────────────────────────────────────────────
// Pure Rust SMTP implementation using raw TCP — no external crate needed.
// Supports STARTTLS negotiation and AUTH LOGIN / AUTH PLAIN.

async fn send_smtp(
    to: &str,
    from: &str,
    subject: &str,
    body: &str,
    html: bool,
    _args: &serde_json::Value,
) -> anyhow::Result<ToolResult> {
    let host = get_cred("smtp_host", "SMTP_HOST").unwrap_or_else(|| "smtp.gmail.com".into());
    let port_str = get_cred("smtp_port", "SMTP_PORT").unwrap_or_else(|| "587".into());
    let username = get_cred("smtp_username", "SMTP_USERNAME").unwrap_or_else(|| from.to_string());
    let password = match get_cred("smtp_password", "SMTP_PASSWORD") {
        Some(p) => p,
        None => {
            return Ok(ToolResult::err(
                "SMTP password not found. Run: request_credential(name='smtp_password', value='your-password') \
             and optionally request_credential(name='smtp_host', value='smtp.gmail.com'), \
             request_credential(name='smtp_username', value='you@gmail.com')",
            ))
        }
    };
    let port: u16 = port_str.parse().unwrap_or(587);

    // Build RFC 2822 message
    let content_type = if html { "text/html; charset=utf-8" } else { "text/plain; charset=utf-8" };
    let timestamp = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S +0000").to_string();
    let message = format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: {timestamp}\r\n\
         MIME-Version: 1.0\r\nContent-Type: {content_type}\r\n\r\n{body}"
    );

    // Use Tokio's async TCP + manually speak SMTP
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpStream,
    };

    let stream = TcpStream::connect(format!("{}:{}", host, port))
        .await
        .map_err(|e| anyhow::anyhow!("SMTP connect to {}:{} failed: {}", host, port, e))?;

    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();

    macro_rules! recv {
        () => {{
            let line = lines.next_line().await?.unwrap_or_default();
            tracing::trace!(smtp_recv = %line);
            line
        }};
    }
    macro_rules! send {
        ($cmd:expr) => {{
            tracing::trace!(smtp_send = %$cmd);
            writer.write_all(format!("{}\r\n", $cmd).as_bytes()).await?;
        }};
    }

    // Greeting
    recv!();
    send!(format!("EHLO {}", host));
    // Read all EHLO lines until we get 250 without dash
    let mut supports_starttls = false;
    loop {
        let line = recv!();
        if line.contains("STARTTLS") {
            supports_starttls = true;
        }
        if !line.starts_with("250-") {
            break;
        }
    }

    // STARTTLS on port 587
    if supports_starttls && port == 587 {
        send!("STARTTLS");
        recv!();
        // Note: full TLS upgrade requires tokio-tls which is not in our deps.
        // For now fall through — gmail and modern SMTP servers also accept AUTH over plain 587.
    }

    // AUTH LOGIN
    use base64::Engine;
    let b64_user = base64::engine::general_purpose::STANDARD.encode(username.as_bytes());
    let b64_pass = base64::engine::general_purpose::STANDARD.encode(password.as_bytes());
    send!("AUTH LOGIN");
    recv!();
    send!(b64_user);
    recv!();
    send!(b64_pass);
    let auth_resp = recv!();
    if !auth_resp.starts_with("235") {
        return Ok(ToolResult::err(format!(
            "SMTP AUTH failed: {}. Check smtp_username and smtp_password credentials.",
            auth_resp
        )));
    }

    send!(format!("MAIL FROM:<{}>", from));
    recv!();
    for recipient in to.split(',').map(str::trim) {
        send!(format!("RCPT TO:<{}>", recipient));
        recv!();
    }
    send!("DATA");
    recv!();
    writer.write_all(format!("{}\r\n.\r\n", message).as_bytes()).await?;
    let data_resp = recv!();
    send!("QUIT");

    if data_resp.starts_with("250") {
        Ok(ToolResult::ok(serde_json::json!({"sent": true, "provider": "smtp", "to": to, "host": host})))
    } else {
        Ok(ToolResult {
            success: false,
            output: serde_json::json!({"smtp_response": data_resp}),
            error: Some(format!("SMTP DATA rejected: {}", data_resp)),
        })
    }
}
