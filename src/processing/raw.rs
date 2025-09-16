use image;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use super::cache::{generate_cache_key, save_thumbnail_to_cache, save_full_image_to_cache};

// Function to find all JPEGs over 50KB in a byte array (returns Vec<(start, end, size)>)
fn find_jpegs(data: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut candidates = Vec::new();
    let mut i = 0;
    
    while i < data.len() - 1 {
        if data[i] == 0xFF && data[i + 1] == 0xD8 {
            let start = i;
            let mut end = i + 2;
            let mut found_end = false;
            
            // Look for the JPEG end marker
            while end < data.len() - 1 {
                if data[end] == 0xFF && data[end + 1] == 0xD9 {
                    end += 2;
                    found_end = true;
                    break;
                }
                end += 1;
            }
            
            // If we didn't find a proper end marker, skip this candidate
            if !found_end {
                i = start + 2;
                continue;
            }
            
            let size = end - start;
            if size > 50_000 {
                candidates.push((start, end, size));
                log::debug!("Found JPEG candidate: {} bytes at offset {}", size, start);
            }
            
            i = end;
        } else {
            i += 1;
        }
    }
    
    // Sort by size descending to try larger JPEGs first
    candidates.sort_by(|a, b| b.2.cmp(&a.2));
    log::info!("Found {} JPEG candidates", candidates.len());
    candidates
}

// RAF-specific JPEG extraction - RAF files store preview JPEG in a specific location
fn find_raf_jpegs(data: &[u8]) -> Vec<(usize, usize, usize)> {
    log::debug!("Attempting RAF-specific JPEG extraction");
    
    let mut candidates = Vec::new();
    
    // RAF files have a specific structure - look for RAF signature first
    if data.len() > 16 && &data[0..16] == b"FUJIFILMCCD-RAW " {
        log::debug!("Confirmed RAF file format");
        
        // RAF files typically have JPEG preview data after the header
        // Look for JPEG markers starting from offset 100 to skip header
        let mut i = 100;
        while i < data.len() - 1 {
            if data[i] == 0xFF && data[i + 1] == 0xD8 {
                let start = i;
                let mut end = i + 2;
                let mut found_end = false;
                
                // More aggressive search for JPEG end in RAF files
                while end < data.len() - 1 {
                    if data[end] == 0xFF {
                        if end + 1 < data.len() && data[end + 1] == 0xD9 {
                            end += 2;
                            found_end = true;
                            break;
                        }
                        // Skip over other FF markers
                        if end + 1 < data.len() && (data[end + 1] >= 0xE0 && data[end + 1] <= 0xEF) {
                            // Application-specific marker, skip length
                            if end + 3 < data.len() {
                                let marker_len = ((data[end + 2] as u16) << 8) | (data[end + 3] as u16);
                                end += 2 + marker_len as usize;
                                continue;
                            }
                        }
                    }
                    end += 1;
                }
                
                if found_end {
                    let size = end - start;
                    if size > 10_000 { // Lower threshold for RAF files
                        candidates.push((start, end, size));
                        log::debug!("Found RAF JPEG candidate: {} bytes at offset {}", size, start);
                    }
                }
                
                i = if found_end { end } else { start + 2 };
            } else {
                i += 1;
            }
        }
    }
    
    // Sort by size descending
    candidates.sort_by(|a, b| b.2.cmp(&a.2));
    log::info!("Found {} RAF-specific JPEG candidates", candidates.len());
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

    // Check if this is a RAF file and try RAF-specific extraction first
    let is_raf = file_path.to_lowercase().ends_with(".raf");
    let candidates = if is_raf {
        log::info!("Detected RAF file, trying RAF-specific extraction first");
        let raf_candidates = find_raf_jpegs(&file_data);
        if !raf_candidates.is_empty() {
            raf_candidates
        } else {
            log::warn!("RAF-specific extraction found no candidates, falling back to generic method");
            find_jpegs(&file_data)
        }
    } else {
        find_jpegs(&file_data)
    };
    
    if candidates.is_empty() {
        log::error!("No suitable embedded JPEG found in RAW file {}", file_path);
        return Err(format!("No JPEG found in RAW file {}", file_path));
    }

    for (idx, (start, end, size)) in candidates.iter().enumerate() {
        log::info!("Trying embedded JPEG candidate #{}: {} bytes at offset {}", idx + 1, size, start);
        let jpeg_data = &file_data[*start..*end];
        
        // Validate JPEG header more thoroughly
        if jpeg_data.len() < 10 {
            log::warn!("JPEG candidate #{} too small ({} bytes), skipping", idx + 1, jpeg_data.len());
            continue;
        }
        
        if jpeg_data[0] != 0xFF || jpeg_data[1] != 0xD8 {
            log::warn!("JPEG candidate #{} has invalid header, skipping", idx + 1);
            continue;
        }
        
        match image::load_from_memory(jpeg_data) {
            Ok(img) => {
                let width = img.width();
                let height = img.height();
                log::debug!("Embedded JPEG dimensions: {}x{}", width, height);
                
                // Skip very small images (likely thumbnails we don't want)
                if width < 200 || height < 200 {
                    log::debug!("JPEG candidate #{} too small ({}x{}), trying next", idx + 1, width, height);
                    continue;
                }
                
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