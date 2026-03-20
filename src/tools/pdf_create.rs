//! pdf_create — Generate PDFs from scratch with `printpdf`. No browser needed.

use std::io::BufWriter;

use async_trait::async_trait;
use printpdf::*;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct PdfCreateTool;

#[async_trait]
impl Tool for PdfCreateTool {
    fn name(&self) -> &str {
        "pdf_create"
    }
    fn description(&self) -> &str {
        "Generate a PDF from structured text content — no browser or HTML needed. \
         Supports multiple pages, font sizes, bold, and basic layout. \
         Returns base64 PDF and saves to path if specified."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "content",
                "string",
                "Text content. Use \\n for line breaks, --- for page breaks.",
            ),
            ParameterSchema::optional("path", "string", "Output file path."),
            ParameterSchema::optional("title", "string", "Document title (default: 'Document')."),
            ParameterSchema::optional("font_size", "number", "Body font size in pt (default: 11.0)."),
            ParameterSchema::optional("margin_mm", "number", "Page margin in mm (default: 20.0)."),
            ParameterSchema::optional("paper", "string", "Page size: A4 (default) | letter"),
            ParameterSchema::optional("sections", "array", "Structured sections: [{heading, body, font_size?}]"),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = args["path"].as_str().map(String::from);
        let title = args["title"].as_str().unwrap_or("Document").to_string();
        let font_size = args["font_size"].as_f64().unwrap_or(11.0) as f32;
        let margin_mm = args["margin_mm"].as_f64().unwrap_or(20.0) as f32;
        let paper = args["paper"].as_str().unwrap_or("A4").to_string();

        let content = args["content"].as_str().unwrap_or("").to_string();
        let sections: Vec<(String, String, f32)> = args["sections"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        let heading = s["heading"].as_str()?.to_string();
                        let body = s["body"].as_str()?.to_string();
                        let fs = s["font_size"].as_f64().unwrap_or(font_size as f64) as f32;
                        Some((heading, body, fs))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let result = tokio::task::spawn_blocking(move || {
            build_pdf(&title, &content, &sections, font_size, margin_mm, &paper, path.as_deref())
        })
        .await
        .map_err(|e| anyhow::anyhow!("thread: {}", e))??;

        Ok(ToolResult::ok(result))
    }
}

fn build_pdf(
    title: &str,
    content: &str,
    sections: &[(String, String, f32)],
    font_size: f32,
    margin_mm: f32,
    paper: &str,
    path: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    use base64::Engine;

    let (w_mm, h_mm) = if paper.to_lowercase() == "letter" {
        (215.9_f32, 279.4_f32)
    } else {
        (210.0_f32, 297.0_f32) // A4
    };

    let (doc, page1, layer1) = PdfDocument::new(title, Mm(w_mm), Mm(h_mm), "Layer 1");
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;

    let page_w = w_mm as f32;
    let page_h = h_mm as f32;
    let margin = margin_mm;
    let usable_w = page_w - 2.0 * margin;
    let _line_h = font_size * 0.35278; // pt to mm

    let current_layer = doc.get_page(page1).get_layer(layer1);

    // Write title
    current_layer.use_text(title, font_size + 4.0, Mm(margin as f32), Mm((page_h - margin) as f32), &font_bold);
    let mut y = page_h - margin - (font_size + 4.0) * 0.35278 - 5.0;

    // Helper: write wrapped text
    let chars_per_line = (usable_w / (font_size * 0.5 * 0.35278)) as usize;
    let write_wrapped = |layer: &PdfLayerReference, text: &str, x: f32, y: &mut f32, fs: f32, bold: bool| {
        let the_font = if bold { &font_bold } else { &font };
        for line in text.split('\n') {
            let chunks: Vec<&str> =
                line.as_bytes().chunks(chars_per_line.max(1)).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
            for chunk in &chunks {
                if *y < margin + 10.0 {
                    break;
                }
                layer.use_text(*chunk, fs, Mm(x as f32), Mm(*y as f32), the_font);
                *y -= fs * 0.35278 + 1.0;
            }
        }
        *y -= 3.0; // paragraph gap
    };

    // Sections
    for (heading, body, fs) in sections {
        if y < margin + 20.0 {
            y = page_h - margin;
        }
        write_wrapped(&current_layer, heading, margin, &mut y, fs + 2.0, true);
        write_wrapped(&current_layer, body, margin, &mut y, *fs, false);
    }

    // Plain content (if no sections)
    if sections.is_empty() && !content.is_empty() {
        for paragraph in content.split("---") {
            write_wrapped(&current_layer, paragraph.trim(), margin, &mut y, font_size, false);
        }
    }

    // Serialize to bytes
    let mut buf = BufWriter::new(Vec::new());
    doc.save(&mut buf)?;
    let bytes = buf.into_inner()?;

    if let Some(p) = path {
        if let Some(parent) = std::path::Path::new(p).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, &bytes)?;
    }

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(serde_json::json!({
        "title":      title,
        "pages":      1,
        "size_bytes": bytes.len(),
        "saved_to":   path,
        "pdf_b64":    b64,
    }))
}
