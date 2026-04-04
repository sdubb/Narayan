//! spreadsheet — Read XLSX/XLS/ODS and write XLSX using calamine + rust_xlsxwriter.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_integer, schema_array};

pub struct SpreadsheetReadTool;
pub struct SpreadsheetWriteTool;

#[async_trait]
impl Tool for SpreadsheetReadTool {
    fn name(&self) -> &str {
        "spreadsheet_read"
    }
    fn description(&self) -> &str {
        "Read data from XLSX, XLS, or ODS spreadsheets. Returns rows as JSON arrays. \
         Supports sheet selection, header row detection, and row limits."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ path, sheet?, header_row?, max_rows?, start_row? }. path is required.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some(
            "{ sheet, sheets, headers, rows, row_count }. rows are returned as arrays or objects depending on headers."
                .into(),
        )
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use to read structured spreadsheet data into JSON before downstream transforms or analysis.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when the source is not a spreadsheet or when a CSV/record transform can be done directly from workspace files.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("path", "string", "Spreadsheet file path."),
            ParameterSchema::optional("sheet", "string", "Sheet name (default: first sheet)."),
            ParameterSchema::optional("header_row", "boolean", "Treat first row as column headers (default: true)."),
            ParameterSchema::optional("max_rows", "integer", "Max data rows to return (default: 1000)."),
            ParameterSchema::optional("start_row", "integer", "First data row index (0-based, default: 0)."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["sheet", "sheets", "headers", "rows", "row_count"],
            "properties": {
                "sheet": schema_string(),
                "sheets": schema_array(schema_string()),
                "headers": schema_array(schema_string()),
                "rows": schema_array(serde_json::json!({})),
                "row_count": schema_integer(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = match args["path"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(ToolResult::err("'path' required")),
        };
        let sheet_name = args["sheet"].as_str().map(String::from);
        let header = args["header_row"].as_bool().unwrap_or(true);
        let max_rows = args["max_rows"].as_u64().unwrap_or(1000).min(50_000) as usize;
        let start_row = args["start_row"].as_u64().unwrap_or(0) as usize;

        let result = tokio::task::spawn_blocking(move || {
            read_spreadsheet(&path, sheet_name.as_deref(), header, max_rows, start_row)
        })
        .await
        .map_err(|e| anyhow::anyhow!("thread: {}", e))??;

        Ok(ToolResult::ok(result))
    }
}

fn read_spreadsheet(
    path: &str,
    sheet_name: Option<&str>,
    header: bool,
    max_rows: usize,
    start_row: usize,
) -> anyhow::Result<serde_json::Value> {
    use calamine::{open_workbook_auto, Data, Reader};

    let mut wb = open_workbook_auto(path).map_err(|e| anyhow::anyhow!("open '{}': {}", path, e))?;

    let sheet_names = wb.sheet_names().to_vec();
    let target = sheet_name.map(String::from).unwrap_or_else(|| sheet_names.first().cloned().unwrap_or_default());

    let range = wb
        .worksheet_range(&target)
        .map_err(|e| anyhow::anyhow!("read sheet '{}': {} — available: {}", target, e, sheet_names.join(", ")))?;

    let mut rows: Vec<Vec<serde_json::Value>> = range
        .rows()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    Data::Int(n) => serde_json::json!(n),
                    Data::Float(f) => serde_json::json!(f),
                    Data::String(s) => serde_json::json!(s),
                    Data::Bool(b) => serde_json::json!(b),
                    Data::Empty => serde_json::Value::Null,
                    other => serde_json::json!(format!("{:?}", other)),
                })
                .collect()
        })
        .collect();

    let headers: Vec<String> = if header && !rows.is_empty() {
        rows.remove(0).iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()
    } else {
        vec![]
    };

    let data_rows: Vec<serde_json::Value> = rows
        .into_iter()
        .skip(start_row)
        .take(max_rows)
        .map(|row| {
            if !headers.is_empty() {
                let obj: serde_json::Map<String, serde_json::Value> = headers
                    .iter()
                    .zip(row.iter().chain(std::iter::repeat(&serde_json::Value::Null)))
                    .map(|(h, v)| (h.clone(), v.clone()))
                    .collect();
                serde_json::Value::Object(obj)
            } else {
                serde_json::Value::Array(row)
            }
        })
        .collect();

    Ok(serde_json::json!({
        "sheet":    target,
        "sheets":   sheet_names,
        "headers":  headers,
        "rows":     data_rows,
        "row_count": data_rows.len(),
    }))
}

#[async_trait]
impl Tool for SpreadsheetWriteTool {
    fn name(&self) -> &str {
        "spreadsheet_write"
    }
    fn description(&self) -> &str {
        "Write data to a new XLSX spreadsheet. Accepts rows as JSON arrays or objects. \
         Supports multiple sheets, column headers, and basic cell formatting."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ output, rows, headers?, sheet? }. output and rows are required.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ output, rows, columns, sheet, size_bytes }. Indicates the written spreadsheet file.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use when the final artifact should be a spreadsheet file or tabular export.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some(
            "Avoid when you only need a JSON transform or when the output should remain in the workspace as text."
                .into(),
        )
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("output", "string", "Output .xlsx file path."),
            ParameterSchema::required("rows", "array", "Data rows: array of objects [{col: val}] or arrays."),
            ParameterSchema::optional(
                "headers",
                "array",
                "Column headers (auto-detected from first object if omitted).",
            ),
            ParameterSchema::optional("sheet", "string", "Sheet name (default: 'Sheet1')."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let output = match args["output"].as_str() {
            Some(o) => o.to_string(),
            None => return Ok(ToolResult::err("'output' required")),
        };
        let rows = args["rows"].as_array().cloned().unwrap_or_default();
        let sheet = args["sheet"].as_str().unwrap_or("Sheet1").to_string();
        let headers: Vec<String> = args["headers"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_else(|| {
                if let Some(first) = rows.first() {
                    if let Some(obj) = first.as_object() {
                        return obj.keys().cloned().collect();
                    }
                }
                vec![]
            });

        let result = tokio::task::spawn_blocking(move || write_spreadsheet(&output, &rows, &headers, &sheet))
            .await
            .map_err(|e| anyhow::anyhow!("thread: {}", e))??;

        Ok(ToolResult::ok(result))
    }
}

fn write_spreadsheet(
    output: &str,
    rows: &[serde_json::Value],
    headers: &[String],
    sheet: &str,
) -> anyhow::Result<serde_json::Value> {
    use rust_xlsxwriter::{Format, Workbook};

    if let Some(p) = std::path::Path::new(output).parent() {
        std::fs::create_dir_all(p)?;
    }

    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name(sheet)?;

    let bold = Format::new().set_bold();

    // Write headers
    for (c, h) in headers.iter().enumerate() {
        ws.write_with_format(0, c as u16, h.as_str(), &bold)?;
    }

    // Write data rows
    for (r, row) in rows.iter().enumerate() {
        let row_idx = (r + if !headers.is_empty() { 1 } else { 0 }) as u32;
        match row {
            serde_json::Value::Object(obj) => {
                for (c, h) in headers.iter().enumerate() {
                    let val = obj.get(h).unwrap_or(&serde_json::Value::Null);
                    write_cell(ws, row_idx, c as u16, val)?;
                }
            }
            serde_json::Value::Array(arr) => {
                for (c, val) in arr.iter().enumerate() {
                    write_cell(ws, row_idx, c as u16, val)?;
                }
            }
            _ => {}
        }
    }

    if !headers.is_empty() {
        ws.autofit();
    }
    wb.save(output).map_err(|e| anyhow::anyhow!("save: {}", e))?;

    let size = std::fs::metadata(output)?.len();
    Ok(serde_json::json!({
        "output":     output,
        "rows":       rows.len(),
        "columns":    headers.len(),
        "sheet":      sheet,
        "size_bytes": size,
    }))
}

fn write_cell(ws: &mut rust_xlsxwriter::Worksheet, row: u32, col: u16, val: &serde_json::Value) -> anyhow::Result<()> {
    match val {
        serde_json::Value::Number(n) => {
            ws.write(row, col, n.as_f64().unwrap_or(0.0))?;
        }
        serde_json::Value::Bool(b) => {
            ws.write(row, col, *b)?;
        }
        serde_json::Value::Null => {}
        other => {
            ws.write(row, col, other.as_str().unwrap_or(&other.to_string()))?;
        }
    }
    Ok(())
}
