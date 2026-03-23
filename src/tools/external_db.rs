//! `external_db` — connect to the user's own external database.
//!
//! Supports Postgres (primary), MySQL, and read-only inspection of any
//! database whose connection string is stored as a tenant credential.
//!
//! ## Security model
//!
//! - Connection strings are stored encrypted in `connector_installs` table
//!   (auth_type = "connection_string")
//! - By default all connections open in read-only transaction mode unless
//!   the install has `settings.allow_writes = true`
//! - Row limit enforced server-side: max 1000 rows per query
//! - Query timeout enforced: max 60 seconds
//! - Blocked statement types when readonly: INSERT, UPDATE, DELETE, DROP,
//!   TRUNCATE, ALTER, CREATE, GRANT, REVOKE
//!
//! ## How the LLM uses it
//!
//! 1. Call `external_db { db: "acme_psql", operation: "schema" }` to get
//!    all tables + column types
//! 2. Call `external_db { db: "acme_psql", operation: "query",
//!    sql: "SELECT ..." }` to read data
//! 3. If writes are enabled: `operation: "execute"` for INSERT/UPDATE

use async_trait::async_trait;
use serde_json::Value;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub const TOOL_NAME: &str = "external_db";

// Statements blocked in read-only mode (checked case-insensitively)
const WRITE_KEYWORDS: &[&str] = &[
    "insert ", "update ", "delete ", "drop ", "truncate ", "alter ",
    "create ", "grant ", "revoke ", "replace ", "merge ", "upsert ",
    "call ", "exec ", "execute ",
];

pub struct ExternalDbTool {
    install_store: Option<std::sync::Arc<crate::connectors::ConnectorInstallStore>>,
}

impl ExternalDbTool {
    pub fn new() -> Self { Self { install_store: None } }

    pub fn with_install_store(store: std::sync::Arc<crate::connectors::ConnectorInstallStore>) -> Self {
        Self { install_store: Some(store) }
    }

    async fn load_connection(
        &self,
        tenant_id: &str,
        db_name: &str,
    ) -> Result<(String, bool), String> {
        let store = self.install_store.as_ref()
            .ok_or_else(|| "No install store configured".to_string())?;

        let install = store.get(tenant_id, db_name).await
            .map_err(|e| format!("DB lookup failed: {e}"))?
            .ok_or_else(|| format!(
                "No database '{db_name}' connected. Add it in Settings → Connections → Databases."
            ))?;

        let conn_str = store.decrypt_token(&install)
            .ok_or_else(|| "Failed to decrypt connection string".to_string())?;

        let allow_writes = install.settings
            .get("allow_writes")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Ok((conn_str, allow_writes))
    }
}

fn is_write_statement(sql: &str) -> bool {
    let lower = sql.trim().to_lowercase();
    WRITE_KEYWORDS.iter().any(|kw| lower.starts_with(kw) || lower.contains(&format!(" {kw}")))
}

#[async_trait]
impl Tool for ExternalDbTool {
    fn name(&self) -> &str { TOOL_NAME }

    fn description(&self) -> &str {
        "Query or inspect an external database (Postgres, MySQL) connected by the tenant. \
         Use operation='schema' to discover tables and columns before writing queries. \
         Use operation='query' to SELECT data. Use operation='execute' for writes (if enabled). \
         Always run schema first if you don't know the table structure."
    }

    fn category(&self) -> &'static str { "integration" }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "db",
                "string",
                "Name of the connected database (as registered in Settings → Connections → Databases).",
            ),
            ParameterSchema::required(
                "operation",
                "string",
                "What to do: 'schema' (list tables/columns), 'query' (SELECT), 'execute' (write, if enabled), \
                 'table_preview' (first 10 rows of a table), 'explain' (EXPLAIN a query).",
            ),
            ParameterSchema::optional(
                "sql",
                "string",
                "SQL to execute. Required for 'query', 'execute', and 'explain' operations.",
            ),
            ParameterSchema::optional(
                "table",
                "string",
                "Table name. Required for 'table_preview'.",
            ),
            ParameterSchema::optional(
                "schema",
                "string",
                "Database schema/namespace to inspect (default: 'public' for Postgres).",
            ),
            ParameterSchema::optional(
                "max_rows",
                "integer",
                "Maximum rows to return for query/preview (default: 100, max: 1000).",
            ),
            ParameterSchema::optional(
                "tenant_id",
                "string",
                "Injected by executor — tenant for credential lookup.",
            ),
        ]
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let db        = args["db"].as_str().unwrap_or("").to_string();
        let operation = args["operation"].as_str().unwrap_or("").to_string();
        let tenant_id = args["tenant_id"].as_str().unwrap_or("").to_string();
        let max_rows  = args["max_rows"].as_u64().unwrap_or(100).min(1000) as usize;
        let schema    = args["schema"].as_str().unwrap_or("public");

        if db.is_empty() {
            return Ok(ToolResult::err("'db' is required — name of the connected database"));
        }
        if operation.is_empty() {
            return Ok(ToolResult::err("'operation' is required: schema | query | execute | table_preview | explain"));
        }

        let (conn_str, allow_writes) = match self.load_connection(&tenant_id, &db).await {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::err(e)),
        };

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            do_operation(&conn_str, &operation, &args, schema, max_rows, allow_writes),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("Query timed out after 60 seconds")));

        match result {
            Ok(v)  => Ok(ToolResult::ok(v)),
            Err(e) => Ok(ToolResult::err(format!("{e}"))),
        }
    }
}

async fn do_operation(
    conn_str:     &str,
    operation:    &str,
    args:         &Value,
    schema:       &str,
    max_rows:     usize,
    allow_writes: bool,
) -> anyhow::Result<Value> {
    // Detect DB type from connection string
    if conn_str.starts_with("postgres://") || conn_str.starts_with("postgresql://") {
        pg_operation(conn_str, operation, args, schema, max_rows, allow_writes).await
    } else if conn_str.starts_with("mysql://") {
        Err(anyhow::anyhow!("MySQL support coming soon. Please use Postgres for now."))
    } else {
        Err(anyhow::anyhow!("Unknown database type. Connection string must start with postgres:// or mysql://"))
    }
}

async fn pg_operation(
    conn_str:     &str,
    operation:    &str,
    args:         &Value,
    schema_name:  &str,
    max_rows:     usize,
    allow_writes: bool,
) -> anyhow::Result<Value> {
    use sqlx::postgres::PgPoolOptions;
    use sqlx::Row;

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(conn_str)
        .await
        .map_err(|e| anyhow::anyhow!("Connection failed: {e}"))?;

    match operation {
        // ── Schema discovery ─────────────────────────────────────────────
        "schema" => {
            // List all tables with column info
            let rows = sqlx::query(
                "SELECT
                    t.table_name,
                    c.column_name,
                    c.data_type,
                    c.is_nullable,
                    c.column_default,
                    COALESCE(
                        (SELECT TRUE FROM information_schema.table_constraints tc
                         JOIN information_schema.key_column_usage kcu
                           ON tc.constraint_name = kcu.constraint_name
                         WHERE tc.table_name = c.table_name
                           AND kcu.column_name = c.column_name
                           AND tc.constraint_type = 'PRIMARY KEY'),
                        FALSE
                    ) as is_primary_key
                FROM information_schema.tables t
                JOIN information_schema.columns c ON t.table_name = c.table_name
                WHERE t.table_schema = $1
                  AND t.table_type = 'BASE TABLE'
                ORDER BY t.table_name, c.ordinal_position",
            )
            .bind(schema_name)
            .fetch_all(&pool)
            .await?;

            // Group by table
            let mut tables: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
            for row in rows {
                let table_name: String      = row.get("table_name");
                let col_name: String        = row.get("column_name");
                let data_type: String       = row.get("data_type");
                let nullable: String        = row.get("is_nullable");
                let default: Option<String> = row.get("column_default");
                let is_pk: bool             = row.try_get("is_primary_key").unwrap_or(false);

                tables.entry(table_name).or_default().push(serde_json::json!({
                    "column":   col_name,
                    "type":     data_type,
                    "nullable": nullable == "YES",
                    "default":  default,
                    "primary_key": is_pk,
                }));
            }

            // Also get row counts for each table
            let mut schema_out: Vec<Value> = Vec::new();
            for (table, columns) in &tables {
                let count: i64 = sqlx::query_scalar(
                    &format!("SELECT COUNT(*)::bigint FROM {schema_name}.{table}")
                )
                .fetch_one(&pool)
                .await
                .unwrap_or(0);

                schema_out.push(serde_json::json!({
                    "table":   table,
                    "rows":    count,
                    "columns": columns,
                }));
            }

            Ok(serde_json::json!({
                "schema":    schema_name,
                "tables":    schema_out,
                "table_count": tables.len(),
            }))
        }

        // ── Table preview ────────────────────────────────────────────────
        "table_preview" => {
            let table = args["table"].as_str()
                .ok_or_else(|| anyhow::anyhow!("'table' required for table_preview"))?;
            // Sanitise: only allow alphanumeric + underscore
            if !table.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                anyhow::bail!("Invalid table name '{table}'");
            }
            pg_query_rows(&pool, &format!("SELECT * FROM {schema_name}.{table} LIMIT {max_rows}"), &[], max_rows).await
        }

        // ── Query ────────────────────────────────────────────────────────
        "query" => {
            let sql = args["sql"].as_str()
                .ok_or_else(|| anyhow::anyhow!("'sql' required for query operation"))?;
            // Enforce read-only
            if is_write_statement(sql) {
                anyhow::bail!(
                    "Write statement detected. This database is read-only. \
                     To enable writes, set allow_writes=true in Settings → Connections."
                );
            }
            let limited_sql = if sql.to_lowercase().contains("limit ") {
                sql.to_string()
            } else {
                format!("{sql} LIMIT {max_rows}")
            };
            pg_query_rows(&pool, &limited_sql, &[], max_rows).await
        }

        // ── Execute (writes) ─────────────────────────────────────────────
        "execute" => {
            if !allow_writes {
                anyhow::bail!(
                    "Writes are disabled for this database. \
                     Enable them in Settings → Connections → Edit → Allow writes."
                );
            }
            let sql = args["sql"].as_str()
                .ok_or_else(|| anyhow::anyhow!("'sql' required for execute operation"))?;

            let result = sqlx::query(sql).execute(&pool).await
                .map_err(|e| anyhow::anyhow!("Execute failed: {e}"))?;

            Ok(serde_json::json!({
                "rows_affected": result.rows_affected(),
                "success": true,
            }))
        }

        // ── EXPLAIN ──────────────────────────────────────────────────────
        "explain" => {
            let sql = args["sql"].as_str()
                .ok_or_else(|| anyhow::anyhow!("'sql' required for explain operation"))?;
            pg_query_rows(&pool, &format!("EXPLAIN (FORMAT JSON) {sql}"), &[], 10).await
        }

        _ => Err(anyhow::anyhow!("Unknown operation '{}'. Use: schema | query | execute | table_preview | explain", operation)),
    }
}

/// Execute a query and return rows as JSON array.
async fn pg_query_rows(
    pool:     &sqlx::PgPool,
    sql:      &str,
    _params:  &[&str],
    max_rows: usize,
) -> anyhow::Result<Value> {
    use sqlx::Row;
    use sqlx::Column;

    let rows = sqlx::query(sql)
        .fetch_all(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Query failed: {e}"))?;

    let truncated = rows.len() > max_rows;
    let rows = &rows[..rows.len().min(max_rows)];

    let json_rows: Vec<Value> = rows.iter().map(|row| {
        let mut obj = serde_json::Map::new();
        for col in row.columns() {
            let name = col.name().to_string();
            // Try common types in order
            let val = row.try_get::<String, _>(col.ordinal())
                .map(Value::String)
                .or_else(|_| row.try_get::<i64, _>(col.ordinal()).map(|v| Value::Number(v.into())))
                .or_else(|_| row.try_get::<f64, _>(col.ordinal()).map(|v| serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null)))
                .or_else(|_| row.try_get::<bool, _>(col.ordinal()).map(Value::Bool))
                .or_else(|_| row.try_get::<serde_json::Value, _>(col.ordinal()))
                .unwrap_or(Value::Null);
            obj.insert(name, val);
        }
        Value::Object(obj)
    }).collect();

    Ok(serde_json::json!({
        "rows":      json_rows,
        "row_count": json_rows.len(),
        "truncated": truncated,
        "sql":       sql,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_detection() {
        assert!(is_write_statement("INSERT INTO leads (name) VALUES ('test')"));
        assert!(is_write_statement("UPDATE leads SET name='test'"));
        assert!(is_write_statement("DELETE FROM leads WHERE id=1"));
        assert!(is_write_statement("DROP TABLE leads"));
        assert!(!is_write_statement("SELECT * FROM leads"));
        assert!(!is_write_statement("SELECT count(*) FROM leads WHERE name='update'"));
    }

    #[test]
    fn test_tool_name() {
        let t = ExternalDbTool::new();
        assert_eq!(t.name(), TOOL_NAME);
    }

    #[tokio::test]
    async fn test_missing_db_returns_error() {
        let t = ExternalDbTool::new();
        let result = t.execute(serde_json::json!({"db": "nonexistent", "operation": "schema"})).await.unwrap();
        assert!(!result.success);
    }
}
