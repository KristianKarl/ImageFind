use std::{fs, path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::processing::cache::{generate_cache_key, save_preview_to_cache, save_thumbnail_to_cache};


fn magick_extract_preview(file_path: &str) -> Result<Vec<u8>, String> {
    log::info!("Attempting magick preview extraction for: {file_path}");

    // Create a unique temporary directory for extraction
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let tmp_dir: PathBuf = std::env::temp_dir().join(format!(
        "imagefind_magick{}_{}",
        generate_cache_key(file_path), ts
    ));
    if let Err(e) = fs::create_dir_all(&tmp_dir) {
        log::warn!("Failed to create temp dir for magick: {e}");
        return Err(format!("Temp dir create failed: {e}"));
    }
    log::trace!("Created temp dir for magick: {}", tmp_dir.display());

    // Run: magick -w output.jpg -b -PreviewImage <file>
    // We set current_dir to tmp_dir so the previews are written there.
    let output = Command::new("magick")
        .arg(file_path)
        .arg(tmp_dir.join("output.jpg"))
        .current_dir(&tmp_dir)
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);
                log::error!("magick failed for {file_path}: {stderr}");
                log::error!("stdout: {stdout}");
                // Cleanup and propagate error
                let _ = fs::remove_dir_all(&tmp_dir);
                return Err(format!("magick failed: {stderr}"));
            }
        }
        Err(e) => {
            log::warn!("Failed to execute magick for {file_path}: {e}");
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(format!("magick exec failed: {e}"));
        }
    }
    log::trace!("magick preview extraction completed for: {file_path}");
    let result = fs::read(tmp_dir.join("output.jpg")).map_err(|e| format!("Failed to read magick output: {e}"));
    let size = fs::metadata(tmp_dir.join("output.jpg")).map(|m| m.len()).unwrap_or(0);
    log::trace!("magick output file size: {} bytes", size); 


    // Cleanup temp dir
    let _ = fs::remove_dir_all(&tmp_dir);
    result
}

pub fn generate_magick_preview(file_path: &str) -> Option<String> {
    log::info!("Generating preview for: {file_path}");

    let cache_key = generate_cache_key(file_path);

    // First try magick-based extraction
    match magick_extract_preview(file_path)
        .and_then(|bytes| scale_jpeg_bytes(&bytes, 1980, 60))
    {
        Ok(jpeg_bytes) => {
            if let Err(e) = save_preview_to_cache(&cache_key, &jpeg_bytes) {
                log::warn!("Failed to cache magick preview: {e}");
            }
            let base64_result = BASE64.encode(&jpeg_bytes);
            log::info!("Successfully generated preview, base64 length: {}", base64_result.len());
            Some(base64_result)
        }
        Err(e) => {
            log::error!("magick preview failed for {file_path}: {e}");
            None
        }
    }
}

pub fn generate_magick_thumbnail(file_path: &str) -> Option<String> {
    log::info!("Generating thumbnail for: {file_path}");

    let cache_key = generate_cache_key(file_path);

    // First try exif2-based extraction
    match magick_extract_preview(file_path)
        .and_then(|bytes| scale_jpeg_bytes(&bytes, 200, 50))
    {
        Ok(jpeg_bytes) => {
            if let Err(e) = save_thumbnail_to_cache(&cache_key, &jpeg_bytes) {
                log::warn!("Failed to cache magick thumbnail: {e}");
            }
            let base64_result = BASE64.encode(&jpeg_bytes);
            log::info!("Successfully generated thumbnail via magick, base64 length: {}", base64_result.len());
            Some(base64_result)
        }
        Err(e) => {
            log::error!("magick thumbnail failed for {file_path}: {e}");
            None
        }
    }
}

// Scale JPEG bytes to max_dimension and re-encode with given quality
fn scale_jpeg_bytes(jpeg: &[u8], max_dimension: u32, jpeg_quality: u8) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(jpeg).map_err(|e| format!("Failed to load JPEG bytes: {}", e))?;
    let scaled = img.resize(max_dimension, max_dimension, image::imageops::FilterType::CatmullRom);
    let mut out = Vec::new();
    scaled
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, jpeg_quality))
        .map_err(|e| format!("Failed to encode JPEG: {}", e))?;
    Ok(out)
}