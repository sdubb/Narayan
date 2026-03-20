//! kubernetes — Manage k8s resources via `kube` crate (official Rust k8s client).

use async_trait::async_trait;
use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Namespace, Pod},
};
use kube::{
    api::{DeleteParams, ListParams, PostParams},
    Api, Client, ResourceExt,
};

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct KubernetesTool;

#[async_trait]
impl Tool for KubernetesTool {
    fn name(&self) -> &str {
        "kubernetes"
    }
    fn description(&self) -> &str {
        "Manage Kubernetes resources. Actions: get | list | apply | delete | scale | logs | rollout. \
         Uses kubeconfig from ~/.kube/config or KUBECONFIG env var."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "action",
                "string",
                "Action: get|list|apply|delete|scale|logs|rollout|namespaces",
            ),
            ParameterSchema::required("kind", "string", "Resource kind: pod|deployment|service|namespace"),
            ParameterSchema::optional("name", "string", "Resource name (for get/delete/scale/logs/rollout)."),
            ParameterSchema::optional("namespace", "string", "Kubernetes namespace (default: 'default')."),
            ParameterSchema::optional("manifest", "object", "Resource manifest JSON for apply."),
            ParameterSchema::optional("replicas", "integer", "Desired replicas (for scale action)."),
            ParameterSchema::optional("label", "string", "Label selector filter: 'app=myapp'."),
            ParameterSchema::optional("tail", "integer", "Log lines to fetch (default: 50)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = match args["action"].as_str() {
            Some(a) => a,
            None => return Ok(ToolResult::err("'action' required")),
        };
        let kind = match args["kind"].as_str() {
            Some(k) => k,
            None => return Ok(ToolResult::err("'kind' required")),
        };
        let ns = args["namespace"].as_str().unwrap_or("default");
        let name = args["name"].as_str();

        let client =
            Client::try_default().await.map_err(|e| anyhow::anyhow!("k8s client: {} — check KUBECONFIG", e))?;

        match (action, kind.to_lowercase().as_str()) {
            ("namespaces", _) | ("list", "namespace") => {
                let api: Api<Namespace> = Api::all(client);
                let list = api.list(&ListParams::default()).await?;
                let names: Vec<String> = list.items.iter().filter_map(|n| n.metadata.name.clone()).collect();
                Ok(ToolResult::ok(serde_json::json!({"namespaces": names})))
            }

            ("list", "pod") => {
                let api: Api<Pod> = Api::namespaced(client, ns);
                let lp = if let Some(l) = args["label"].as_str() {
                    ListParams::default().labels(l)
                } else {
                    ListParams::default()
                };
                let pods = api.list(&lp).await?;
                let items: Vec<serde_json::Value> = pods
                    .items
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "name":   p.name_any(),
                            "phase":  p.status.as_ref().and_then(|s| s.phase.clone()),
                            "ready":  pod_ready(p),
                            "node":   p.spec.as_ref().and_then(|s| s.node_name.clone()),
                        })
                    })
                    .collect();
                Ok(ToolResult::ok(serde_json::json!({"namespace": ns, "pods": items, "count": items.len()})))
            }

            ("list", "deployment") => {
                let api: Api<Deployment> = Api::namespaced(client, ns);
                let deps = api.list(&ListParams::default()).await?;
                let items: Vec<serde_json::Value> = deps
                    .items
                    .iter()
                    .map(|d| {
                        serde_json::json!({
                            "name":             d.name_any(),
                            "replicas":         d.spec.as_ref().and_then(|s| s.replicas),
                            "ready_replicas":   d.status.as_ref().and_then(|s| s.ready_replicas),
                            "available":        d.status.as_ref().and_then(|s| s.available_replicas),
                        })
                    })
                    .collect();
                Ok(ToolResult::ok(serde_json::json!({"namespace": ns, "deployments": items})))
            }

            ("get", "pod") => {
                let n = name.ok_or_else(|| anyhow::anyhow!("'name' required for get"))?;
                let api: Api<Pod> = Api::namespaced(client, ns);
                let pod = api.get(n).await?;
                Ok(ToolResult::ok(serde_json::to_value(pod)?))
            }

            ("logs", "pod") => {
                let n = name.ok_or_else(|| anyhow::anyhow!("'name' required for logs"))?;
                let tail = args["tail"].as_u64().unwrap_or(50) as i64;
                let api: Api<Pod> = Api::namespaced(client, ns);
                let lp = kube::api::LogParams { tail_lines: Some(tail), ..Default::default() };
                let log = api.logs(n, &lp).await?;
                Ok(ToolResult::ok(serde_json::json!({"pod": n, "logs": crate::util::truncate(&log, 100_000)})))
            }

            ("scale", "deployment") => {
                let n = name.ok_or_else(|| anyhow::anyhow!("'name' required"))?;
                let replicas = args["replicas"].as_u64().ok_or_else(|| anyhow::anyhow!("'replicas' required"))? as i32;
                let api: Api<Deployment> = Api::namespaced(client, ns);
                let mut dep = api.get(n).await?;
                if let Some(ref mut spec) = dep.spec {
                    spec.replicas = Some(replicas);
                }
                api.replace(n, &PostParams::default(), &dep).await?;
                Ok(ToolResult::ok(serde_json::json!({"scaled": true, "deployment": n, "replicas": replicas})))
            }

            ("delete", "pod") => {
                let n = name.ok_or_else(|| anyhow::anyhow!("'name' required"))?;
                let api: Api<Pod> = Api::namespaced(client, ns);
                api.delete(n, &DeleteParams::default()).await?;
                Ok(ToolResult::ok(serde_json::json!({"deleted": true, "pod": n})))
            }

            ("rollout", "deployment") => {
                let n = name.ok_or_else(|| anyhow::anyhow!("'name' required"))?;
                let api: Api<Deployment> = Api::namespaced(client, ns);
                let dep = api.get(n).await?;
                let status = dep.status.as_ref();
                Ok(ToolResult::ok(serde_json::json!({
                    "deployment":    n,
                    "replicas":      status.and_then(|s| s.replicas),
                    "ready":         status.and_then(|s| s.ready_replicas),
                    "available":     status.and_then(|s| s.available_replicas),
                    "updated":       status.and_then(|s| s.updated_replicas),
                })))
            }

            (a, k) => Ok(ToolResult::err(format!("unsupported action/kind combo: {}/{}", a, k))),
        }
    }
}

fn pod_ready(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|cs| cs.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
        .unwrap_or(false)
}
