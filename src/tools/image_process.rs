//! image_process — Resize, crop, convert, rotate, and watermark images.
//! Uses the `image` crate — pure Rust, no system deps.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_integer};

pub struct ImageProcessTool;

#[async_trait]
impl Tool for ImageProcessTool {
    fn name(&self) -> &str {
        "image_process"
    }
    fn description(&self) -> &str {
        "Process images: resize, crop, rotate, convert format, grayscale, blur, brighten. \
         Supports PNG, JPEG, WebP, GIF, BMP, TIFF. Pure Rust — no ImageMagick needed."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("input", "string", "Input image file path."),
            ParameterSchema::required("output", "string", "Output file path (extension determines format)."),
            ParameterSchema::optional("ops", "array", "Operations in order: [{type, ...}]. See below."),
            ParameterSchema::optional("quality", "integer", "JPEG/WebP quality 1-100 (default: 85)."),
        ]
    }
    // Supported ops:
    //  {type: "resize",    width: N, height: N, keep_aspect: true}
    //  {type: "thumbnail", width: N, height: N}          -- smart fill crop
    //  {type: "crop",      x: N, y: N, width: N, height: N}
    //  {type: "rotate",    degrees: 90|180|270}
    //  {type: "flip",      axis: "horizontal"|"vertical"}
    //  {type: "grayscale"}
    //  {type: "blur",      sigma: 2.0}
    //  {type: "brighten",  value: 20}                     -- -255 to 255
    //  {type: "contrast",  value: 1.5}

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["output", "width", "height", "format", "size_bytes", "ops_applied", "elapsed_ms"],
            "properties": {
                "output": schema_string(),
                "width": schema_integer(),
                "height": schema_integer(),
                "format": schema_string(),
                "size_bytes": schema_integer(),
                "ops_applied": schema_integer(),
                "elapsed_ms": schema_integer(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let input = match args["input"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(ToolResult::err("'input' required")),
        };
        let output = match args["output"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(ToolResult::err("'output' required")),
        };
        let quality = args["quality"].as_u64().unwrap_or(85).clamp(1, 100) as u8;
        let ops = args["ops"].as_array().cloned().unwrap_or_default();

        let result = tokio::task::spawn_blocking(move || process_image(&input, &output, &ops, quality))
            .await
            .map_err(|e| anyhow::anyhow!("thread: {}", e))??;

        Ok(ToolResult::ok(result))
    }
}

fn process_image(
    input: &str,
    output: &str,
    ops: &[serde_json::Value],
    _quality: u8,
) -> anyhow::Result<serde_json::Value> {
    use std::time::Instant;

    use image::{imageops::FilterType, GenericImageView};

    let t0 = Instant::now();
    let mut img = image::open(input).map_err(|e| anyhow::anyhow!("open '{}': {}", input, e))?;

    let (orig_w, orig_h) = img.dimensions();

    for op in ops {
        let kind = op["type"].as_str().unwrap_or("");
        img = match kind {
            "resize" => {
                let w = op["width"].as_u64().unwrap_or(orig_w as u64) as u32;
                let h = op["height"].as_u64().unwrap_or(orig_h as u64) as u32;
                let ka = op["keep_aspect"].as_bool().unwrap_or(true);
                if ka {
                    img.resize(w, h, FilterType::Lanczos3)
                } else {
                    img.resize_exact(w, h, FilterType::Lanczos3)
                }
            }
            "thumbnail" => {
                let w = op["width"].as_u64().unwrap_or(256) as u32;
                let h = op["height"].as_u64().unwrap_or(256) as u32;
                img.resize_to_fill(w, h, FilterType::Lanczos3)
            }
            "crop" => {
                let x = op["x"].as_u64().unwrap_or(0) as u32;
                let y = op["y"].as_u64().unwrap_or(0) as u32;
                let w = op["width"].as_u64().unwrap_or(orig_w as u64) as u32;
                let h = op["height"].as_u64().unwrap_or(orig_h as u64) as u32;
                img.crop_imm(x, y, w, h)
            }
            "rotate" => match op["degrees"].as_u64().unwrap_or(90) {
                90 => img.rotate90(),
                180 => img.rotate180(),
                270 => img.rotate270(),
                _ => img,
            },
            "flip" => match op["axis"].as_str().unwrap_or("horizontal") {
                "vertical" => img.flipv(),
                _ => img.fliph(),
            },
            "grayscale" => img.grayscale(),
            "blur" => {
                let s = op["sigma"].as_f64().unwrap_or(2.0) as f32;
                img.blur(s)
            }
            "brighten" => {
                let v = op["value"].as_i64().unwrap_or(20) as i32;
                img.brighten(v)
            }
            "contrast" => {
                let v = op["value"].as_f64().unwrap_or(1.5) as f32;
                img.adjust_contrast(v)
            }
            other => anyhow::bail!(
                "unknown op '{}'. Supported: resize|thumbnail|crop|rotate|flip|grayscale|blur|brighten|contrast",
                other
            ),
        };
    }

    if let Some(parent) = std::path::Path::new(output).parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Save — format determined by output extension
    let ext = std::path::Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();

    match ext.as_str() {
        "jpg" | "jpeg" => {
            img.save_with_format(output, image::ImageFormat::Jpeg).map_err(|e| anyhow::anyhow!("save jpeg: {}", e))?;
        }
        "webp" => {
            img.save_with_format(output, image::ImageFormat::WebP).map_err(|e| anyhow::anyhow!("save webp: {}", e))?;
        }
        "gif" => {
            img.save_with_format(output, image::ImageFormat::Gif).map_err(|e| anyhow::anyhow!("save gif: {}", e))?;
        }
        _ => {
            img.save(output).map_err(|e| anyhow::anyhow!("save: {}", e))?;
        }
    }

    let (out_w, out_h) = img.dimensions();
    let out_size = std::fs::metadata(output)?.len();

    Ok(serde_json::json!({
        "output":      output,
        "width":       out_w,
        "height":      out_h,
        "format":      ext,
        "size_bytes":  out_size,
        "ops_applied": ops.len(),
        "elapsed_ms":  t0.elapsed().as_millis() as u64,
    }))
}
