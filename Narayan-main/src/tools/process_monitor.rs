//! process_monitor — System process info using `sysinfo`. Cross-platform.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct ProcessMonitorTool;

#[async_trait]
impl Tool for ProcessMonitorTool {
    fn name(&self) -> &str {
        "process_monitor"
    }
    fn description(&self) -> &str {
        "Monitor system processes, CPU, and memory. List all processes, \
         find by name, get system resource usage, or kill a process by PID."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("action", "string", "Action: list | find | system | kill | top"),
            ParameterSchema::optional("name", "string", "Process name filter (for find action)."),
            ParameterSchema::optional("pid", "integer", "Process ID (for kill action)."),
            ParameterSchema::optional("top_n", "integer", "Number of top CPU/mem processes (default: 10)."),
            ParameterSchema::optional("sort_by", "string", "Sort for 'top': cpu | memory (default: cpu)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = match args["action"].as_str() {
            Some(a) => a.to_string(),
            None => return Ok(ToolResult::err("'action' required")),
        };
        let n_arg = args.clone();

        let result = tokio::task::spawn_blocking(move || run_action(&action, &n_arg))
            .await
            .map_err(|e| anyhow::anyhow!("thread: {}", e))??;

        Ok(ToolResult::ok(result))
    }
}

fn run_action(action: &str, args: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    use sysinfo::{Pid, Signal, System};

    let mut sys = System::new();

    match action {
        "system" => {
            sys.refresh_all();
            Ok(serde_json::json!({
                "cpu_usage_pct":   sys.global_cpu_usage(),
                "total_memory_mb": sys.total_memory() / 1_048_576,
                "used_memory_mb":  sys.used_memory()  / 1_048_576,
                "total_swap_mb":   sys.total_swap()   / 1_048_576,
                "used_swap_mb":    sys.used_swap()    / 1_048_576,
                "process_count":   sys.processes().len(),
                "uptime_secs":     System::uptime(),
                "os":              System::long_os_version(),
                "kernel":          System::kernel_version(),
            }))
        }

        "list" => {
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            let procs: Vec<serde_json::Value> = sys.processes().iter().map(|(pid, p)| proc_to_json(*pid, p)).collect();
            Ok(serde_json::json!({"processes": procs, "count": procs.len()}))
        }

        "find" => {
            let name = args["name"].as_str().unwrap_or("").to_lowercase();
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            let found: Vec<serde_json::Value> = sys
                .processes()
                .iter()
                .filter(|(_, p)| p.name().to_string_lossy().to_lowercase().contains(&name))
                .map(|(pid, p)| proc_to_json(*pid, p))
                .collect();
            Ok(serde_json::json!({"query": name, "processes": found, "count": found.len()}))
        }

        "top" => {
            let n = args["top_n"].as_u64().unwrap_or(10) as usize;
            let sort_by = args["sort_by"].as_str().unwrap_or("cpu");
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

            let mut procs: Vec<serde_json::Value> =
                sys.processes().iter().map(|(pid, p)| proc_to_json(*pid, p)).collect();

            if sort_by == "memory" {
                procs.sort_by(|a, b| {
                    b["memory_mb"]
                        .as_f64()
                        .unwrap_or(0.0)
                        .partial_cmp(&a["memory_mb"].as_f64().unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            } else {
                procs.sort_by(|a, b| {
                    b["cpu_pct"]
                        .as_f64()
                        .unwrap_or(0.0)
                        .partial_cmp(&a["cpu_pct"].as_f64().unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }

            Ok(serde_json::json!({"sort_by": sort_by, "processes": &procs[..n.min(procs.len())]}))
        }

        "kill" => {
            let pid_n = match args["pid"].as_u64() {
                Some(p) => p,
                None => return Err(anyhow::anyhow!("'pid' required for kill")),
            };
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            let pid = Pid::from_u32(pid_n as u32);
            match sys.process(pid) {
                Some(p) => {
                    let killed = p.kill_with(Signal::Term).unwrap_or(false) || p.kill();
                    Ok(serde_json::json!({"pid": pid_n, "killed": killed}))
                }
                None => Err(anyhow::anyhow!("process {} not found", pid_n)),
            }
        }

        other => Err(anyhow::anyhow!("unknown action '{}' — use: list | find | system | kill | top", other)),
    }
}

fn proc_to_json(pid: sysinfo::Pid, p: &sysinfo::Process) -> serde_json::Value {
    serde_json::json!({
        "pid":       pid.as_u32(),
        "name":      p.name().to_string_lossy(),
        "cpu_pct":   (p.cpu_usage() * 10.0).round() / 10.0,
        "memory_mb": p.memory() / 1_048_576,
        "status":    format!("{:?}", p.status()),
        "exe":       p.exe().map(|e| e.display().to_string()),
    })
}
