use image;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use super::cache::{generate_cache_key, save_thumbnail_to_cache, save_full_image_to_cache};

// Function to find all JPEGs over 50KB in a byte array (returns Vec<(start, end, size)>)
fn find_jpegs(data: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut candidates = Vec::new();
    let mut i = 0;
    while i < data.len() - 1 {
        if data[i] == 0xFF && data[i + 1] == 0xD8 {
            let mut end = i + 2;
            while end < data.len() - 1 {
                if data[end] == 0xFF && data[end + 1] == 0xD9 {
                    let size = end + 2 - i;
                    if size > 50_000 {
                        candidates.push((i, end + 2, size));
                    }
                    break;
                }
                end += 1;
            }
            i = end + 2;
        } else {
            i += 1;
        }
    }
    // Sort by size descending
    candidates.sort_by(|a, b| b.2.cmp(&a.2));
    candidates
}

// Shared function for RAW to RGB JPEG (for both thumbnail and preview)
pub fn convert_raw_to_rgb_jpeg(
    file_path: &str,
    max_dimension: u32,
    jpeg_quality: u8,
    cache_key: Option<&str>,
    save_to_cache: Option<fn(&str, &[u8]) -> std::io::Result<()>>,
) -> Result<Vec<u8>, String> {
    log::info!("Processing RAW file: {}", file_path);
    let file_data = std::fs::read(file_path)
        .map_err(|e| {
            log::error!("Failed to read RAW file {}: {}", file_path, e);
            format!("Failed to read file {}: {}", file_path, e)
        })?;
    log::debug!("Successfully read RAW file, size: {} bytes", file_data.len());

    let candidates = find_jpegs(&file_data);
    if candidates.is_empty() {
        log::error!("No suitable embedded JPEG found in RAW file {}", file_path);
        return Err(format!("No JPEG found in RAW file {}", file_path));
    }
    for (idx, (start, end, size)) in candidates.iter().enumerate() {
        log::info!("Trying embedded JPEG candidate #{}: {} bytes at offset {}", idx + 1, size, start);
        let jpeg_data = &file_data[*start..*end];
        match image::load_from_memory(jpeg_data) {
            Ok(img) => {
                let width = img.width();
                let height = img.height();
                log::debug!("Embedded JPEG dimensions: {}x{}", width, height);
                let scaled_img = if width > max_dimension || height > max_dimension {
                    log::debug!("Large embedded JPEG ({}x{}), using progressive scaling to {}", width, height, max_dimension);
                    let intermediate = img.resize(800, 800, image::imageops::FilterType::Triangle);
                    intermediate.resize(max_dimension, max_dimension, image::imageops::FilterType::CatmullRom)
                } else {
                    log::debug!("Small embedded JPEG ({}x{}), direct scaling to {}", width, height, max_dimension);
                    img.resize(max_dimension, max_dimension, image::imageops::FilterType::CatmullRom)
                };
                log::trace!("Image scaling completed");
                let mut jpeg_bytes = Vec::new();
                scaled_img.write_with_encoder(
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, jpeg_quality)
                ).map_err(|e| {
                    log::error!("Failed to encode processed RAW as JPEG for {}: {}", file_path, e);
                    format!("Failed to encode JPEG: {}", e)
                })?;
                log::debug!("Successfully encoded RAW result as JPEG, size: {} bytes, quality: {}", jpeg_bytes.len(), jpeg_quality);
                if let (Some(key), Some(save_fn)) = (cache_key, save_to_cache) {
                    match save_fn(key, &jpeg_bytes) {
                        Ok(_) => log::trace!("Saved RAW processing result to cache"),
                        Err(e) => log::warn!("Failed to save RAW result to cache: {}", e),
                    }
                }
                return Ok(jpeg_bytes);
            }
            Err(e) => {
                let head = jpeg_data.iter().take(8).map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                let tail = jpeg_data.iter().rev().take(8).map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                log::warn!("Failed to decode embedded JPEG candidate #{} from {}: {}. First 8 bytes: [{}], Last 8 bytes: [{}]", idx + 1, file_path, e, head, tail);
            }
        }
    }
    log::error!("All embedded JPEG candidates failed to decode in RAW file {}", file_path);
    Err(format!("No valid embedded JPEG could be decoded in {}", file_path))
}

pub fn generate_raw_preview(file_path: &str, cache_key: &str) -> Result<Vec<u8>, String> {
    log::info!("Generating RAW preview for: {}", file_path);
    
    let result = convert_raw_to_rgb_jpeg(
        file_path,
        1980,
        60,
        Some(cache_key),
        Some(save_full_image_to_cache),
    );
    
    match &result {
        Ok(bytes) => log::info!("Successfully generated RAW preview, size: {} bytes", bytes.len()),
        Err(e) => log::error!("Failed to generate RAW preview: {}", e),
    }
    
    result
}

pub fn generate_raw_thumbnail(file_path: &str) -> Option<String> {
    log::info!("Generating RAW thumbnail for: {}", file_path);
    
    let cache_key = generate_cache_key(file_path);
    
    match convert_raw_to_rgb_jpeg(
        file_path,
        200,
        50,
        Some(&cache_key),
        Some(save_thumbnail_to_cache),
    ) {
        Ok(jpeg_bytes) => {
            log::debug!("RAW thumbnail generation successful, encoding as base64");
            
            let base64_result = BASE64.encode(&jpeg_bytes);
            log::info!("Successfully generated RAW thumbnail, base64 length: {}", base64_result.len());
            Some(base64_result)
        }
        Err(e) => {
            log::error!("RAW thumbnail generation failed for {}: {}", file_path, e);
            None
        }
    }
}