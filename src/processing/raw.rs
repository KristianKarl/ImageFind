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

// Enhanced NEF-specific JPEG extraction
fn find_nef_jpegs(data: &[u8]) -> Vec<(usize, usize, usize)> {
    log::debug!("Attempting NEF-specific JPEG extraction");
    
    let mut candidates = Vec::new();
    let mut i = 0;
    
    while i < data.len() - 1 {
        if data[i] == 0xFF && data[i + 1] == 0xD8 {
            log::trace!("Found JPEG SOI at offset: {}", i);
            
            let start = i;
            let mut end = i + 2;
            let mut found_end = false;
            let mut in_scan_data = false;
            
            // Enhanced JPEG parsing for NEF files
            while end < data.len() - 1 {
                if data[end] == 0xFF && end + 1 < data.len() {
                    let marker = data[end + 1];
                    match marker {
                        0xD9 => {
                            // End of Image marker
                            end += 2;
                            found_end = true;
                            log::trace!("Found JPEG EOI at offset: {}", end);
                            break;
                        }
                        0xD8 => {
                            // Another SOI - this means we hit another JPEG
                            log::trace!("Found another SOI at offset {}, ending current JPEG", end);
                            break;
                        }
                        0xDA => {
                            // Start of Scan - we're entering compressed image data
                            log::trace!("Found SOS marker at offset: {}", end);
                            end += 2;
                            // Skip scan header
                            if end + 1 < data.len() {
                                let scan_length = ((data[end] as u16) << 8) | (data[end + 1] as u16);
                                end += scan_length as usize;
                                in_scan_data = true;
                            }
                        }
                        0x00 => {
                            // Escaped FF in scan data
                            if in_scan_data {
                                end += 2;
                            } else {
                                end += 1;
                            }
                        }
                        marker if (0xD0..=0xD7).contains(&marker) => {
                            // Restart markers - no length field
                            end += 2;
                        }
                        marker if (0xE0..=0xEF).contains(&marker) => {
                            // Application segments
                            end += 2;
                            if end + 1 < data.len() {
                                let seg_length = ((data[end] as u16) << 8) | (data[end + 1] as u16);
                                end += seg_length as usize;
                                log::trace!("Skipped APP segment, length: {}", seg_length);
                            }
                        }
                        marker if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xCC => {
                            // Start of Frame segments
                            end += 2;
                            if end + 1 < data.len() {
                                let seg_length = ((data[end] as u16) << 8) | (data[end + 1] as u16);
                                end += seg_length as usize;
                                log::trace!("Skipped SOF segment, length: {}", seg_length);
                            }
                        }
                        0xC4 => {
                            // Define Huffman Table
                            end += 2;
                            if end + 1 < data.len() {
                                let seg_length = ((data[end] as u16) << 8) | (data[end + 1] as u16);
                                end += seg_length as usize;
                                log::trace!("Skipped DHT segment, length: {}", seg_length);
                            }
                        }
                        0xDB => {
                            // Define Quantization Table
                            end += 2;
                            if end + 1 < data.len() {
                                let seg_length = ((data[end] as u16) << 8) | (data[end + 1] as u16);
                                end += seg_length as usize;
                                log::trace!("Skipped DQT segment, length: {}", seg_length);
                            }
                        }
                        _ => {
                            // Other markers with length field
                            end += 2;
                            if end + 1 < data.len() {
                                let seg_length = ((data[end] as u16) << 8) | (data[end + 1] as u16);
                                if seg_length >= 2 {
                                    end += seg_length as usize;
                                } else {
                                    // Invalid segment length, skip
                                    end += 1;
                                }
                            }
                        }
                    }
                } else {
                    end += 1;
                }
            }
            
            if found_end {
                let size = end - start;
                log::debug!("Found complete NEF JPEG: {} bytes at offset {}", size, start);
                
                // Include JPEGs larger than 3KB for NEF (they tend to have smaller previews)
                if size > 3_000 {
                    candidates.push((start, end, size));
                } else {
                    log::trace!("NEF JPEG too small ({}), skipping", size);
                }
            } else {
                log::trace!("NEF JPEG at offset {} incomplete, skipping", start);
            }
            
            i = if found_end { end } else { start + 2 };
        } else {
            i += 1;
        }
    }
    
    // Sort by size descending
    candidates.sort_by(|a, b| b.2.cmp(&a.2));
    log::info!("Found {} NEF-specific JPEG candidates", candidates.len());
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

    // Determine file type and use appropriate extraction method
    let file_path_lower = file_path.to_lowercase();
    let candidates = if file_path_lower.ends_with(".nef") {
        log::info!("Detected NEF file, using NEF-specific extraction");
        let nef_candidates = find_nef_jpegs(&file_data);
        if !nef_candidates.is_empty() {
            nef_candidates
        } else {
            log::warn!("NEF-specific extraction found no candidates, falling back to generic method");
            find_jpegs(&file_data)
        }
    } else if file_path_lower.ends_with(".raf") {
        log::info!("Detected RAF file, using RAF-specific extraction");
        let raf_candidates = find_raf_jpegs(&file_data);
        if !raf_candidates.is_empty() {
            raf_candidates
        } else {
            log::warn!("RAF-specific extraction found no candidates, falling back to generic method");
            find_jpegs(&file_data)
        }
    } else {
        log::info!("Using generic JPEG extraction for: {}", file_path);
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