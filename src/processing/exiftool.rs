use std::{fs, path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::processing::cache::{generate_cache_key, save_preview_to_cache, save_thumbnail_to_cache};
use crate::processing::raw::{scale_jpeg_bytes};


fn exiftool_extract_preview(file_path: &str) -> Result<Vec<u8>, String> {
    log::info!("Attempting exiv2 preview extraction for: {file_path}");

    // Create a unique temporary directory for extraction
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let tmp_dir: PathBuf = std::env::temp_dir().join(format!(
        "imagefind_exiv2_{}_{}",
        generate_cache_key(file_path), ts
    ));
    if let Err(e) = fs::create_dir_all(&tmp_dir) {
        log::warn!("Failed to create temp dir for exiv2: {e}");
        return Err(format!("Temp dir create failed: {e}"));
    }
    log::trace!("Created temp dir for exiv2: {}", tmp_dir.display());

    // Run: exiv2 -ep <file>
    // We set current_dir to tmp_dir so the previews are written there.
    let output = Command::new("exiv2")
        .arg("-f")
        .arg("-l")
        .arg(&tmp_dir)
        .arg("-ep")
        .arg(file_path)
        .current_dir(&tmp_dir)
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);
                log::error!("exiv2 failed for {file_path}: {stderr}");
                log::error!("stdout: {stdout}");
                // Cleanup and propagate error
                let _ = fs::remove_dir_all(&tmp_dir);
                return Err(format!("exiv2 failed: {stderr}"));
            }
        }
        Err(e) => {
            log::warn!("Failed to execute exiv2 for {file_path}: {e}");
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(format!("exiv2 exec failed: {e}"));
        }
    }
    log::trace!("exiv2 preview extraction completed for: {file_path}");


    // Find the largest preview file produced (usually *-preview*.jpg/jpeg)
    let mut best_file: Option<(PathBuf, u64)> = None;
    if let Ok(entries) = fs::read_dir(&tmp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            log::trace!("Found exiv2 output file: {}", path.display());
            if !path.is_file() { 
                log::warn!("Skipping non-file entry: {}", path.display());
                continue; 
            }
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            let is_jpeg = name.ends_with(".jpg") || name.ends_with(".jpeg");
            if !is_jpeg { 
                log::warn!("Skipping non-JPEG entry: {}", path.display());
                continue; 
            }
            if !(name.contains("preview") || name.contains("thumb") || name.contains("jpg")) {
                log::trace!("Skipping non-JPEG entry: {}", path.display());
                // Keep generic jpgs too, but prioritize preview-ish names by size anyway
            }
            if let Ok(meta) = fs::metadata(&path) {
                let size = meta.len();
                match &best_file {
                    Some((_, best_size)) if size <= *best_size => {}
                    _ => { best_file = Some((path.clone(), size)); }
                }
            }
        }
    } else {
        log::warn!("Failed to read temp dir for exiv2 outputs: {}", tmp_dir.display());
    }

    // Read best preview and cleanup temp directory
    let result = match best_file {
        Some((path, size)) => {
            log::info!("exiv2 preview selected: {} ({} bytes)", path.display(), size);
            fs::read(&path).map_err(|e| format!("Failed to read exiv2 output {}: {}", path.display(), e))
        }
        None => Err("No exiv2 preview files produced".to_string()),
    };
    let _ = fs::remove_dir_all(&tmp_dir);
    result
}

pub fn generate_exiftool_preview(file_path: &str) -> Option<String> {
    log::info!("Generating RAW preview for: {file_path}");

    let cache_key = generate_cache_key(file_path);

    // First try exiftool-based extraction
    match exiftool_extract_preview(file_path)
        .and_then(|bytes| scale_jpeg_bytes(&bytes, 1980, 60))
    {
        Ok(jpeg_bytes) => {
            if let Err(e) = save_preview_to_cache(&cache_key, &jpeg_bytes) {
                log::warn!("Failed to cache exiftool preview: {e}");
            }
            let base64_result = BASE64.encode(&jpeg_bytes);
            log::info!("Successfully generated RAW preview via exiftool, base64 length: {}", base64_result.len());
            Some(base64_result)
        }
        Err(e) => {
            log::error!("exiftool preview failed for {file_path}: {e}");
            None
        }
    }
}

pub fn generate_exiftool_thumbnail(file_path: &str) -> Option<String> {
    log::info!("Generating RAW thumbnail for: {file_path}");

    let cache_key = generate_cache_key(file_path);

    // First try exif2-based extraction
    match exiftool_extract_preview(file_path)
        .and_then(|bytes| scale_jpeg_bytes(&bytes, 200, 50))
    {
        Ok(jpeg_bytes) => {
            if let Err(e) = save_thumbnail_to_cache(&cache_key, &jpeg_bytes) {
                log::warn!("Failed to cache exiftool thumbnail: {e}");
            }
            let base64_result = BASE64.encode(&jpeg_bytes);
            log::info!("Successfully generated RAW thumbnail via exiftool, base64 length: {}", base64_result.len());
            Some(base64_result)
        }
        Err(e) => {
            log::error!("exiftool thumbnail failed for {file_path}: {e}");
            None
        }
    }
}