use async_trait::async_trait;
use regex::Regex;
use scraper::{Html, Selector};

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct DataExtractorTool;

#[async_trait]
impl Tool for DataExtractorTool {
    fn name(&self) -> &str {
        "data_extractor"
    }
    fn description(&self) -> &str {
        "Extract structured data from HTML or plain text. Supports: tables→CSV, CSS selectors, \
         regex patterns, emails, URLs, phone numbers, prices."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ content, extract, selector?, pattern?, attribute? }. content and extract are required.".into())
    }

    fn output_contract(&self) -> Option<String> {
        Some("{ count, tables? | links? | emails? | prices? | phones? | urls? | items? }. Output shape depends on extract mode.".into())
    }

    fn when_to_use(&self) -> Option<String> {
        Some("Use to pull structured records or fields out of HTML/text before passing the result to data_engine.".into())
    }

    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when the data is already structured records or when a deterministic record transform can be done directly in data_engine.".into())
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("content", "string", "HTML or text content to extract from."),
            ParameterSchema::required(
                "extract",
                "string",
                "What to extract: 'tables'|'links'|'emails'|'prices'|'phones'|'urls'|'selector'|'regex'",
            ),
            ParameterSchema::optional("selector", "string", "CSS selector (when extract='selector')."),
            ParameterSchema::optional("pattern", "string", "Regex pattern (when extract='regex')."),
            ParameterSchema::optional("attribute", "string", "HTML attribute to extract (e.g. 'href', 'src')."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let content = match args["content"].as_str() {
            Some(c) => c,
            None => return Ok(ToolResult::err("'content' required")),
        };
        let extract = match args["extract"].as_str() {
            Some(e) => e,
            None => return Ok(ToolResult::err("'extract' required")),
        };
        match extract {
            "tables" => extract_tables(content),
            "links" => extract_links(content),
            "emails" => extract_regex(content, r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}", "emails"),
            "prices" => extract_regex(
                content,
                r"(?:USD|EUR|GBP|[$€£¥])?\s*\d[\d,]*(?:\.\d{1,2})?(?:\s*(?:USD|EUR|GBP|per\s+\w+))?",
                "prices",
            ),
            "phones" => {
                extract_regex(content, r"(?:\+?1[-.\s]?)?\(?[0-9]{3}\)?[-.\s]?[0-9]{3}[-.\s]?[0-9]{4}", "phones")
            }
            "urls" => extract_regex(content, r#"https?://[^\s<>"']+"#, "urls"),
            "selector" => {
                let sel = match args["selector"].as_str() {
                    Some(s) => s,
                    None => return Ok(ToolResult::err("'selector' required for extract='selector'")),
                };
                let attr = args["attribute"].as_str();
                extract_selector(content, sel, attr)
            }
            "regex" => {
                let pat = match args["pattern"].as_str() {
                    Some(p) => p,
                    None => return Ok(ToolResult::err("'pattern' required for extract='regex'")),
                };
                extract_regex(content, pat, "matches")
            }
            other => Ok(ToolResult::err(format!("Unknown extract type: '{other}'"))),
        }
    }
}

fn extract_tables(html: &str) -> anyhow::Result<ToolResult> {
    let doc = Html::parse_document(html);
    let table_sel = Selector::parse("table").map_err(|e| anyhow::anyhow!("{e}"))?;
    let tr_sel = Selector::parse("tr").map_err(|e| anyhow::anyhow!("{e}"))?;
    let td_sel = Selector::parse("td, th").map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut tables = Vec::new();
    for table in doc.select(&table_sel) {
        let rows: Vec<Vec<String>> = table
            .select(&tr_sel)
            .map(|row| row.select(&td_sel).map(|cell| cell.text().collect::<String>().trim().to_string()).collect())
            .collect();
        tables.push(rows);
    }
    Ok(ToolResult::ok(serde_json::json!({"tables": tables, "count": tables.len()})))
}

fn extract_links(html: &str) -> anyhow::Result<ToolResult> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href]").map_err(|e| anyhow::anyhow!("{e}"))?;
    let links: Vec<serde_json::Value> = doc
        .select(&sel)
        .map(|el| {
            serde_json::json!({
                "href": el.value().attr("href").unwrap_or(""),
                "text": el.text().collect::<String>().trim().to_string(),
            })
        })
        .collect();
    Ok(ToolResult::ok(serde_json::json!({"links": links, "count": links.len()})))
}

fn extract_regex(content: &str, pattern: &str, label: &str) -> anyhow::Result<ToolResult> {
    let re = Regex::new(pattern).map_err(|e| anyhow::anyhow!("invalid pattern: {e}"))?;
    let matches: Vec<String> = re.find_iter(content).map(|m| m.as_str().trim().to_string()).collect();
    let deduped: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        matches.into_iter().filter(|m| seen.insert(m.clone())).collect()
    };
    Ok(ToolResult::ok(serde_json::json!({label: deduped, "count": deduped.len()})))
}

fn extract_selector(html: &str, css: &str, attr: Option<&str>) -> anyhow::Result<ToolResult> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse(css).map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let items: Vec<String> = doc
        .select(&sel)
        .map(|el| {
            if let Some(a) = attr {
                el.value().attr(a).unwrap_or("").to_string()
            } else {
                el.text().collect::<String>().trim().to_string()
            }
        })
        .collect();
    Ok(ToolResult::ok(serde_json::json!({"items": items, "count": items.len(), "selector": css})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_extract_tables() {
        let tool = DataExtractorTool;
        let html = r#"<html><body><table><tr><th>Name</th><th>Age</th></tr><tr><td>Alice</td><td>30</td></tr></table></body></html>"#;
        let result = tool
            .execute(serde_json::json!({
                "content": html,
                "extract": "tables"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output["count"].as_u64().unwrap() > 0, "expected at least 1 table");
    }

    #[tokio::test]
    async fn test_extract_links() {
        let tool = DataExtractorTool;
        let html = r#"<html><body><a href="https://example.com">Link</a></body></html>"#;
        let result = tool
            .execute(serde_json::json!({
                "content": html,
                "extract": "links"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output["count"].as_u64().unwrap() > 0, "expected at least 1 link");
    }

    #[tokio::test]
    async fn test_extract_emails() {
        let tool = DataExtractorTool;
        let text = "Contact us at user@example.com for more info.";
        let result = tool
            .execute(serde_json::json!({
                "content": text,
                "extract": "emails"
            }))
            .await
            .unwrap();
        assert!(result.success);
        let emails = result.output["emails"].as_array().unwrap();
        let email_strs: Vec<&str> = emails.iter().filter_map(|v| v.as_str()).collect();
        assert!(email_strs.contains(&"user@example.com"), "expected 'user@example.com' in emails");
    }

    #[tokio::test]
    async fn test_extract_selector() {
        let tool = DataExtractorTool;
        let html = r#"<html><body><div class="target">Hello</div><div class="other">World</div></body></html>"#;
        let result = tool
            .execute(serde_json::json!({
                "content": html,
                "extract": "selector",
                "selector": ".target"
            }))
            .await
            .unwrap();
        assert!(result.success);
        let items = result.output["items"].as_array().unwrap();
        let item_strs: Vec<&str> = items.iter().filter_map(|v| v.as_str()).collect();
        assert!(item_strs.contains(&"Hello"), "expected 'Hello' in selector results");
    }
}
