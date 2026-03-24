//! sql_query — Execute SQL against any Postgres, MySQL, or SQLite database.
//! Uses sqlx (already in deps). Connection strings stored via request_credential.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct SqlQueryTool;

#[async_trait]
impl Tool for SqlQueryTool {
    fn name(&self) -> &str {
        "sql_query"
    }
    fn description(&self) -> &str {
        "Execute a SQL query against a Postgres, MySQL, or SQLite database. \
         Store the connection string with request_credential first. \
         Returns rows as JSON. Use parameterised $1/$2 placeholders for safety."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("query", "string", "SQL query to execute."),
            ParameterSchema::required(
                "connection_key",
                "string",
                "Credential key holding the connection string (e.g. 'prod_db').",
            ),
            ParameterSchema::optional("params", "array", "Query parameters for $1, $2, … placeholders."),
            ParameterSchema::optional("max_rows", "integer", "Max rows to return (default: 500)."),
            ParameterSchema::optional("timeout_secs", "integer", "Query timeout (default: 30)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = match args["query"].as_str() {
            Some(q) => q,
            None => return Ok(ToolResult::err("'query' required")),
        };
        let cred_k = match args["connection_key"].as_str() {
            Some(k) => k,
            None => return Ok(ToolResult::err("'connection_key' required")),
        };

        let conn_str = match crate::tools::memory_store_internal::get(&format!("credential:{}", cred_k)) {
            Some(s) => s,
            None => {
                return Ok(ToolResult::err(format!(
                    "credential '{}' not found — store it first with request_credential",
                    cred_k
                )))
            }
        };

        let max_rows = args["max_rows"].as_u64().unwrap_or(500).min(5000) as usize;
        let timeout_sec = args["timeout_secs"].as_u64().unwrap_or(30);
        let raw_params: Vec<serde_json::Value> = args["params"].as_array().cloned().unwrap_or_default();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_sec),
            run_query(conn_str, query.to_string(), raw_params, max_rows),
        )
        .await;

        match result {
            Ok(Ok(v)) => Ok(ToolResult::ok(v)),
            Ok(Err(e)) => Ok(ToolResult::err(format!("SQL error: {}", e))),
            Err(_) => Ok(ToolResult::err(format!("query timed out after {}s", timeout_sec))),
        }
    }
}

async fn run_query(
    conn_str: String,
    query: String,
    params: Vec<serde_json::Value>,
    max_rows: usize,
) -> anyhow::Result<serde_json::Value> {
    use sqlx::{postgres::PgPoolOptions, Column, Row};

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&conn_str)
        .await
        .map_err(|e| anyhow::anyhow!("connect failed: {}", e))?;

    let start = std::time::Instant::now();

    // Build query with bound parameters
    let mut q = sqlx::query(&query);
    for p in &params {
        q = match p {
            serde_json::Value::Number(n) if n.is_i64() => q.bind(n.as_i64().unwrap()),
            serde_json::Value::Number(n) => q.bind(n.as_f64().unwrap()),
            serde_json::Value::String(s) => q.bind(s.as_str()),
            serde_json::Value::Bool(b) => q.bind(*b),
            serde_json::Value::Null => q.bind(Option::<String>::None),
            other => q.bind(other.to_string()),
        };
    }

    let rows = q.fetch_all(&pool).await.map_err(|e| anyhow::anyhow!("query error: {}", e))?;

    pool.close().await;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let is_mutation = ["insert", "update", "delete", "create", "drop", "alter", "truncate"]
        .iter()
        .any(|kw| query.trim().to_lowercase().starts_with(kw));

    if rows.is_empty() || is_mutation {
        return Ok(serde_json::json!({
            "rows":       [],
            "row_count":  rows.len(),
            "elapsed_ms": elapsed_ms,
        }));
    }

    // Convert rows to JSON — read column names and types
    let cols: Vec<String> = rows[0].columns().iter().map(|c| c.name().to_string()).collect();
    let json_rows: Vec<serde_json::Value> = rows
        .iter()
        .take(max_rows)
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in cols.iter().enumerate() {
                let val = row
                    .try_get_raw(i)
                    .ok()
                    .map(|_raw| {
                        // Try common types in order
                        if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
                            return v.map(|n| serde_json::json!(n)).unwrap_or(serde_json::Value::Null);
                        }
                        if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
                            return v.map(|n| serde_json::json!(n)).unwrap_or(serde_json::Value::Null);
                        }
                        if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
                            return v.map(|b| serde_json::json!(b)).unwrap_or(serde_json::Value::Null);
                        }
                        if let Ok(v) = row.try_get::<Option<String>, _>(i) {
                            return v.map(|s| serde_json::json!(s)).unwrap_or(serde_json::Value::Null);
                        }
                        serde_json::Value::String("[unreadable]".into())
                    })
                    .unwrap_or(serde_json::Value::Null);
                obj.insert(col.clone(), val);
            }
            serde_json::Value::Object(obj)
        })
        .collect();

    Ok(serde_json::json!({
        "rows":       json_rows,
        "row_count":  rows.len(),
        "columns":    cols,
        "truncated":  rows.len() > max_rows,
        "elapsed_ms": elapsed_ms,
    }))
}
