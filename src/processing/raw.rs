use image;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use super::cache::{generate_cache_key, save_thumbnail_to_cache, save_full_image_to_cache};

// Function to find largest JPEG in a byte array (typically from a RAW file)
fn find_largest_jpeg(data: &[u8]) -> Option<(usize, usize)> {
    log::trace!("Searching for embedded JPEG in {} bytes of RAW data", data.len());
    
    let mut largest_jpeg = None;
    let mut largest_size = 0;
    let mut i = 0;
    let mut jpeg_count = 0;
    
    while i < data.len() - 1 {
        if data[i] == 0xFF && data[i + 1] == 0xD8 {  // JPEG start marker
            jpeg_count += 1;
            log::trace!("Found JPEG start marker #{} at position {}", jpeg_count, i);
            
            let mut end = i + 2;
            let mut valid_jpeg = false;
            
            // Look for JPEG end marker
            while end < data.len() - 1 {
                if data[end] == 0xFF && data[end + 1] == 0xD9 {  // JPEG end marker
                    valid_jpeg = true;
                    log::trace!("Found JPEG end marker for JPEG #{} at position {}", jpeg_count, end);
                    break;
                }
                end += 1;
            }

            if valid_jpeg {
                let size = end + 2 - i;
                log::debug!("Found valid JPEG #{}: {} bytes", jpeg_count, size);
                
                // Only consider JPEGs larger than 500KB
                if size > 500_000 && size > largest_size {
                    log::info!("New largest JPEG found: {} bytes (previous: {} bytes)", size, largest_size);
                    largest_size = size;
                    largest_jpeg = Some((i, end + 2));
                } else if size <= 500_000 {
                    log::trace!("JPEG #{} too small ({} bytes), skipping", jpeg_count, size);
                }
            } else {
                log::trace!("JPEG #{} has no end marker, skipping", jpeg_count);
            }
            i = end + 2;
        } else {
            i += 1;
        }
    }
    
    match largest_jpeg {
        Some((start, end)) => {
            log::info!("Found {} JPEG(s) in RAW data, using largest: {} bytes at position {}-{}", 
                     jpeg_count, end - start, start, end);
        },
        None => {
            log::warn!("No suitable JPEG found in RAW data ({} total JPEGs found, none > 500KB)", jpeg_count);
        }
    }
    
    largest_jpeg
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
    
    // Read the file and try to find embedded JPEG
    let file_data = std::fs::read(file_path)
        .map_err(|e| {
            log::error!("Failed to read RAW file {}: {}", file_path, e);
            format!("Failed to read file {}: {}", file_path, e)
        })?;
    
    log::debug!("Successfully read RAW file, size: {} bytes", file_data.len());
    
    let (start, size) = find_largest_jpeg(&file_data)
        .ok_or_else(|| {
            log::error!("No suitable embedded JPEG found in RAW file {}", file_path);
            format!("No JPEG found in RAW file {}", file_path)
        })?;
    
    log::info!("Extracting embedded JPEG from RAW file: {} bytes at offset {}", size, start);
    let jpeg_data = &file_data[start..(start + size)];
    
    // Load and process the JPEG
    let img = image::load_from_memory(jpeg_data)
        .map_err(|e| {
            log::error!("Failed to decode embedded JPEG from {}: {}", file_path, e);
            format!("Invalid JPEG data in {}: {}", file_path, e)
        })?;
    
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
    
    Ok(jpeg_bytes)
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