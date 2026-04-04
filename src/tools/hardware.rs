use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct HardwareBoardInfoTool;
pub struct HardwareMemoryMapTool;
pub struct HardwareMemoryReadTool;

#[async_trait]
impl Tool for HardwareBoardInfoTool {
    fn name(&self) -> &str {
        "hardware_board_info"
    }
    fn description(&self) -> &str {
        "Get system board / hardware information. Reads from /proc/cpuinfo and dmidecode."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "type": "object", "additionalProperties": true }))
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let cpuinfo = tokio::fs::read_to_string("/proc/cpuinfo").await.ok();
        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("uname -a && cat /proc/cpuinfo 2>/dev/null | head -20")
            .output()
            .await;
        match out {
            Ok(o) => Ok(ToolResult::ok(serde_json::json!({"info": String::from_utf8_lossy(&o.stdout).to_string()}))),
            Err(_) => {
                Ok(ToolResult::ok(serde_json::json!({"info": cpuinfo.unwrap_or_else(|| "not available".into())})))
            }
        }
    }
}

#[async_trait]
impl Tool for HardwareMemoryMapTool {
    fn name(&self) -> &str {
        "hardware_memory_map"
    }
    fn description(&self) -> &str {
        "Read the system memory map from /proc/iomem."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![]
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        match tokio::fs::read_to_string("/proc/iomem").await {
            Ok(content) => Ok(ToolResult::ok(serde_json::json!({"memory_map": content}))),
            Err(_) => Ok(ToolResult::err("Cannot read /proc/iomem — may require elevated permissions")),
        }
    }
}

#[async_trait]
impl Tool for HardwareMemoryReadTool {
    fn name(&self) -> &str {
        "hardware_memory_read"
    }
    fn description(&self) -> &str {
        "Read system memory statistics from /proc/meminfo."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![]
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        match tokio::fs::read_to_string("/proc/meminfo").await {
            Ok(content) => {
                let stats: std::collections::HashMap<String, String> = content
                    .lines()
                    .filter_map(|l| {
                        let mut p = l.splitn(2, ':');
                        Some((p.next()?.trim().to_string(), p.next()?.trim().to_string()))
                    }))
                    .collect();
                Ok(ToolResult::ok(serde_json::json!(stats)))
            }
            Err(_) => Ok(ToolResult::err("Cannot read /proc/meminfo")),
        }
    }
}
