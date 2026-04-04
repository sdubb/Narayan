//! compress / decompress — zip, tar.gz, tar.bz2 with `zip`, `flate2`, `tar`.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_integer};

pub struct CompressTool;
pub struct DecompressTool;

fn parse_paths(value: &serde_json::Value) -> Vec<String> {
    if let Some(paths) = value.as_array() {
        return paths.iter().filter_map(|v| v.as_str().map(String::from)).collect();
    }

    value.as_str().map(|path| vec![path.to_string()]).unwrap_or_default()
}

#[async_trait]
impl Tool for CompressTool {
    fn name(&self) -> &str {
        "compress"
    }
    fn description(&self) -> &str {
        "Compress files or directories into a zip, tar.gz, or tar.bz2 archive."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ output, paths, format?, level? }. output and paths are required.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ output, format, files, output_bytes, elapsed_ms }. Describes the created archive.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use when the final artifact should be an archive containing files or directories.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when you only need to transform data or when the output should remain as individual files.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("output", "string", "Output archive path (e.g. 'archive.zip', 'out.tar.gz')."),
            ParameterSchema::required("paths", "array", "Files or directories to include."),
            ParameterSchema::optional(
                "format",
                "string",
                "Format: zip | tar.gz | tar.bz2 (auto-detected from output extension).",
            ),
            ParameterSchema::optional("level", "integer", "Compression level 0-9 (default: 6)."),
        ]
    }


    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["output", "format", "files", "output_bytes", "elapsed_ms"],
            "properties": {
                "output": schema_string(),
                "format": schema_string(),
                "files": schema_integer(),
                "output_bytes": schema_integer(),
                "elapsed_ms": schema_integer(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let output = match args["output"].as_str() {
            Some(o) => o.to_string(),
            None => return Ok(ToolResult::err("'output' required")),
        };
        let paths = parse_paths(&args["paths"]);
        if paths.is_empty() {
            return Ok(ToolResult::err("'paths' must not be empty"));
        }
        let level = args["level"].as_u64().unwrap_or(6).min(9) as u32;
        let fmt = args["format"].as_str().map(String::from).unwrap_or_else(|| {
            if output.ends_with(".tar.gz") || output.ends_with(".tgz") {
                "tar.gz".into()
            } else if output.ends_with(".tar.bz2") {
                "tar.bz2".into()
            } else {
                "zip".into()
            }
        });

        let out = output.clone();
        let paths2 = paths.clone();
        let result = tokio::task::spawn_blocking(move || do_compress(&out, &paths2, &fmt, level))
            .await
            .map_err(|e| anyhow::anyhow!("compress thread: {}", e))??;

        Ok(ToolResult::ok(result))
    }
}

fn do_compress(output: &str, paths: &[String], fmt: &str, level: u32) -> anyhow::Result<serde_json::Value> {
    use std::{fs::File, io::Write};

    if let Some(p) = std::path::Path::new(output).parent() {
        std::fs::create_dir_all(p)?;
    }

    let start = std::time::Instant::now();
    let mut total_files = 0usize;

    match fmt {
        "zip" => {
            let file = File::create(output)?;
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(level as i64));

            for src in paths {
                let src_path = std::path::Path::new(src);
                if src_path.is_dir() {
                    for entry in walkdir::WalkDir::new(src_path).into_iter().flatten() {
                        if entry.file_type().is_file() {
                            let rel = entry.path().strip_prefix(src_path).unwrap_or(entry.path());
                            let name = format!(
                                "{}/{}",
                                src_path.file_name().unwrap_or_default().to_string_lossy(),
                                rel.display()
                            );
                            zip.start_file(&name, opts)?;
                            let data = std::fs::read(entry.path())?;
                            zip.write_all(&data)?;
                            total_files += 1;
                        }
                    }
                } else {
                    let name = src_path.file_name().unwrap_or_default().to_string_lossy();
                    zip.start_file(name.as_ref(), opts)?;
                    let data = std::fs::read(src_path)?;
                    zip.write_all(&data)?;
                    total_files += 1;
                }
            }
            zip.finish()?;
        }
        "tar.gz" | "tgz" => {
            let file = File::create(output)?;
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::new(level));
            let mut tar = tar::Builder::new(enc);
            for src in paths {
                let p = std::path::Path::new(src);
                if p.is_dir() {
                    tar.append_dir_all(p.file_name().unwrap_or_default(), p)?;
                } else {
                    tar.append_path_with_name(p, p.file_name().unwrap_or_default())?;
                }
                total_files += 1;
            }
            tar.finish()?;
        }
        "tar.bz2" => {
            anyhow::bail!("tar.bz2 format is not supported (bzip2 crate not available). Use tar.gz instead.");
        }
        other => anyhow::bail!("unsupported format '{}'. Use: zip | tar.gz | tar.bz2", other),
    }

    let out_size = std::fs::metadata(output)?.len();
    Ok(serde_json::json!({
        "output":      output,
        "format":      fmt,
        "files":       total_files,
        "output_bytes": out_size,
        "elapsed_ms":  start.elapsed().as_millis() as u64,
    }))
}

#[async_trait]
impl Tool for DecompressTool {
    fn name(&self) -> &str {
        "decompress"
    }
    fn description(&self) -> &str {
        "Extract a zip, tar.gz, or tar.bz2 archive to a directory."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ path, output_dir? }. path is required and must point to an archive.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ extracted, output_dir, files, elapsed_ms }. Indicates extraction result.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use when you need to unpack an archive and inspect or process its contents.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid for ordinary file moves or when no archive extraction is needed.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("path", "string", "Archive file to extract."),
            ParameterSchema::optional("output_dir", "string", "Directory to extract into (default: next to archive)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = match args["path"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(ToolResult::err("'path' required")),
        };
        let outdir = args["output_dir"].as_str().map(String::from).unwrap_or_else(|| {
            std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new(".")).to_string_lossy().into_owned()
        });

        let path2 = path.clone();
        let outdir2 = outdir.clone();
        let result = tokio::task::spawn_blocking(move || do_decompress(&path2, &outdir2))
            .await
            .map_err(|e| anyhow::anyhow!("decompress thread: {}", e))??;

        Ok(ToolResult::ok(result))
    }
}

fn do_decompress(path: &str, output_dir: &str) -> anyhow::Result<serde_json::Value> {
    use std::fs::File;
    std::fs::create_dir_all(output_dir)?;
    let start = std::time::Instant::now();
    let mut count = 0usize;

    if path.ends_with(".zip") {
        let file = File::open(path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        count = zip.len();
        zip.extract(output_dir)?;
    } else if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
        let file = File::open(path)?;
        let dec = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(dec);
        tar.unpack(output_dir)?;
    } else if path.ends_with(".tar.bz2") {
        anyhow::bail!("tar.bz2 format is not supported (bzip2 crate not available). Use tar.gz instead.");
    } else {
        anyhow::bail!("unsupported archive format for '{}'. Supported: .zip .tar.gz .tgz .tar.bz2", path);
    }

    Ok(serde_json::json!({
        "extracted":  true,
        "output_dir": output_dir,
        "files":      count,
        "elapsed_ms": start.elapsed().as_millis() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_zip_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("hello.txt"), "hello world").unwrap();

        let archive_path = tmp.path().join("out.zip");

        // Compress
        let compress_tool = CompressTool;
        let result = compress_tool
            .execute(serde_json::json!({
                "output": archive_path.display().to_string(),
                "paths": [src_dir.display().to_string()]
            }))
            .await
            .unwrap();
        assert!(result.success, "compress failed: {:?}", result.error);

        // Decompress
        let extract_dir = tmp.path().join("extracted");
        std::fs::create_dir_all(&extract_dir).unwrap();
        let decompress_tool = DecompressTool;
        let result = decompress_tool
            .execute(serde_json::json!({
                "path": archive_path.display().to_string(),
                "output_dir": extract_dir.display().to_string()
            }))
            .await
            .unwrap();
        assert!(result.success, "decompress failed: {:?}", result.error);

        // Verify extracted content exists (the file should be under src/hello.txt inside the zip)
        let extracted_file = extract_dir.join("src").join("hello.txt");
        assert!(extracted_file.exists(), "expected extracted file at {:?}", extracted_file);
    }

    #[tokio::test]
    async fn test_empty_paths_error() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_path = tmp.path().join("empty.zip");
        let compress_tool = CompressTool;
        let result = compress_tool
            .execute(serde_json::json!({
                "output": archive_path.display().to_string(),
                "paths": []
            }))
            .await
            .unwrap();
        assert!(!result.success, "expected error for empty paths");
    }

    #[test]
    fn test_parse_paths_accepts_single_string() {
        let paths = parse_paths(&serde_json::json!("payload.txt"));
        assert_eq!(paths, vec!["payload.txt"]);
    }
}
