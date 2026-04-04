//! docker — Container management via Bollard (Docker Engine API).
//! Pull, run, exec, logs, stop, ps, build — all async, no docker CLI needed.

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::Docker;
use futures_util::StreamExt;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_boolean, schema_integer, schema_array};

pub struct DockerTool;

#[async_trait]
impl Tool for DockerTool {
    fn name(&self) -> &str {
        "docker"
    }
    fn description(&self) -> &str {
        "Manage Docker containers via the Docker Engine API. \
         Actions: pull | run | exec | logs | stop | ps | inspect | build | remove"
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("action", "string", "Action: pull|run|exec|logs|stop|ps|inspect|build|remove"),
            ParameterSchema::optional("image", "string", "Image name (pull/run/build)."),
            ParameterSchema::optional(
                "container_id",
                "string",
                "Container ID or name (exec/logs/stop/inspect/remove).",
            ),
            ParameterSchema::optional("command", "string", "Command to exec inside container."),
            ParameterSchema::optional("env", "object", "Environment variables for run: {KEY: value}."),
            ParameterSchema::optional("ports", "object", "Port bindings for run: {container_port: host_port}."),
            ParameterSchema::optional("volumes", "array", "Volume mounts: ['host_path:container_path']."),
            ParameterSchema::optional("detach", "boolean", "Run container in background (default: true)."),
            ParameterSchema::optional("tail", "integer", "Log lines to fetch (default: 100)."),
            ParameterSchema::optional("dockerfile", "string", "Dockerfile content for build action."),
            ParameterSchema::optional("build_context", "string", "Build context directory path."),
            ParameterSchema::optional("tag", "string", "Image tag for build."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["containers", "count"],
                    "properties": {
                        "containers": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["id", "image", "names", "status", "state"],
                            "properties": {
                                "id": schema_string(),
                                "image": schema_string(),
                                "names": schema_array(schema_string()),
                                "status": schema_string(),
                                "state": schema_string(),
                            },
                            "additionalProperties": true,
                        })),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["pulled", "image", "status"],
                    "properties": {
                        "pulled": schema_boolean(),
                        "image": schema_string(),
                        "status": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["started", "container_id", "detached"],
                    "properties": {
                        "started": schema_boolean(),
                        "container_id": schema_string(),
                        "detached": schema_boolean(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["exec_id", "output"],
                    "properties": {
                        "exec_id": schema_string(),
                        "output": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["container_id", "logs"],
                    "properties": {
                        "container_id": schema_string(),
                        "logs": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["stopped", "container_id"],
                    "properties": {
                        "stopped": schema_boolean(),
                        "container_id": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["removed", "container_id"],
                    "properties": {
                        "removed": schema_boolean(),
                        "container_id": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["Id"],
                    "properties": {
                        "Id": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| anyhow::anyhow!("Docker connect: {} — is Docker running?", e))?;

        let action = match args["action"].as_str() {
            Some(a) => a,
            None => return Ok(ToolResult::err("'action' required")),
        };

        match action {
            "ps" => {
                use bollard::query_parameters::ListContainersOptions;
                let opts = ListContainersOptions { all: true, ..Default::default() };
                let containers =
                    docker.list_containers(Some(opts)).await.map_err(|e| anyhow::anyhow!("list_containers: {}", e))?;
                let list: Vec<serde_json::Value> = containers
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "id":     c.id.as_deref().unwrap_or("").chars().take(12).collect::<String>(),
                            "image":  c.image,
                            "names":  c.names,
                            "status": c.status,
                            "state":  c.state,
                        })
                    })
                    .collect();
                Ok(ToolResult::ok(serde_json::json!({"containers": list, "count": list.len()})))
            }

            "pull" => {
                let image = match args["image"].as_str() {
                    Some(i) => i,
                    None => return Ok(ToolResult::err("'image' required")),
                };
                use bollard::query_parameters::CreateImageOptions;
                let opts = CreateImageOptions { from_image: Some(image.to_string()), ..Default::default() };
                let mut stream = docker.create_image(Some(opts), None, None);
                let mut last_status = String::new();
                while let Some(item) = stream.next().await {
                    if let Ok(info) = item {
                        if let Some(s) = info.status {
                            last_status = s;
                        }
                    }
                }
                Ok(ToolResult::ok(serde_json::json!({"pulled": true, "image": image, "status": last_status})))
            }

            "run" => {
                let image = match args["image"].as_str() {
                    Some(i) => i,
                    None => return Ok(ToolResult::err("'image' required")),
                };
                let detach = args["detach"].as_bool().unwrap_or(true);
                let cmd_arg = args["command"].as_str().map(|c| vec!["/bin/sh", "-c", c]);

                let env: Vec<String> = args["env"]
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| format!("{}={}", k, v.as_str().unwrap_or(""))).collect())
                    .unwrap_or_default();

                use bollard::{
                    models::{HostConfig, PortBinding},
                    query_parameters::{CreateContainerOptions, StartContainerOptions},
                };

                let port_bindings: HashMap<String, Option<Vec<PortBinding>>> = args["ports"]
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .map(|(container_port, host_port)| {
                                let binding = PortBinding {
                                    host_ip: Some("0.0.0.0".into()),
                                    host_port: host_port.as_str().map(String::from),
                                };
                                (format!("{}/tcp", container_port), Some(vec![binding]))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let binds: Vec<String> = args["volumes"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();

                let config = bollard::models::ContainerCreateBody {
                    image: Some(image.to_string()),
                    cmd: cmd_arg.map(|c| c.iter().map(|s| s.to_string()).collect()),
                    env: Some(env),
                    host_config: Some(HostConfig {
                        port_bindings: Some(port_bindings),
                        binds: Some(binds),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                let container = docker
                    .create_container(None::<CreateContainerOptions>, config)
                    .await
                    .map_err(|e| anyhow::anyhow!("create_container: {}", e))?;

                docker
                    .start_container(&container.id, None::<StartContainerOptions>)
                    .await
                    .map_err(|e| anyhow::anyhow!("start_container: {}", e))?;

                Ok(ToolResult::ok(
                    serde_json::json!({"started": true, "container_id": &container.id[..12.min(container.id.len())], "detached": detach}),
                ))
            }

            "exec" => {
                let cid = match args["container_id"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::err("'container_id' required")),
                };
                let cmd = match args["command"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::err("'command' required")),
                };
                use bollard::exec::{CreateExecOptions, StartExecResults};

                let exec = docker
                    .create_exec(
                        cid,
                        CreateExecOptions {
                            cmd: Some(vec!["/bin/sh", "-c", cmd]),
                            attach_stdout: Some(true),
                            attach_stderr: Some(true),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("create_exec: {}", e))?;

                let mut output = String::new();
                if let StartExecResults::Attached { output: mut stream, .. } =
                    docker.start_exec(&exec.id, None).await.map_err(|e| anyhow::anyhow!("start_exec: {}", e))?
                {
                    while let Some(Ok(msg)) = stream.next().await {
                        output.push_str(&msg.to_string());
                    }
                }
                Ok(ToolResult::ok(
                    serde_json::json!({"exec_id": exec.id, "output": crate::util::truncate(&output, 50_000)}),
                ))
            }

            "logs" => {
                let cid = match args["container_id"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::err("'container_id' required")),
                };
                let tail = args["tail"].as_u64().unwrap_or(100);
                use bollard::query_parameters::LogsOptions;
                let opts = LogsOptions { stdout: true, stderr: true, tail: tail.to_string(), ..Default::default() };
                let mut log = String::new();
                let mut stream = docker.logs(cid, Some(opts));
                while let Some(Ok(line)) = stream.next().await {
                    log.push_str(&line.to_string());
                }
                Ok(ToolResult::ok(
                    serde_json::json!({"container_id": cid, "logs": crate::util::truncate(&log, 100_000)}),
                ))
            }

            "stop" => {
                let cid = match args["container_id"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::err("'container_id' required")),
                };
                docker.stop_container(cid, None).await.map_err(|e| anyhow::anyhow!("stop: {}", e))?;
                Ok(ToolResult::ok(serde_json::json!({"stopped": true, "container_id": cid})))
            }

            "remove" => {
                let cid = match args["container_id"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::err("'container_id' required")),
                };
                use bollard::query_parameters::RemoveContainerOptions;
                docker
                    .remove_container(cid, Some(RemoveContainerOptions { force: true, ..Default::default() }))
                    .await
                    .map_err(|e| anyhow::anyhow!("remove: {}", e))?;
                Ok(ToolResult::ok(serde_json::json!({"removed": true, "container_id": cid})))
            }

            "inspect" => {
                let cid = match args["container_id"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::err("'container_id' required")),
                };
                let info = docker.inspect_container(cid, None).await.map_err(|e| anyhow::anyhow!("inspect: {}", e))?;
                Ok(ToolResult::ok(serde_json::to_value(info)?))
            }

            other => Ok(ToolResult::err(format!(
                "unknown action '{}' — use: pull|run|exec|logs|stop|ps|inspect|build|remove",
                other
            ))),
        }
    }
}
