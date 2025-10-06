use std::path::Path;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::processing::magick::generate_magick_preview;

use super::magick::{generate_magick_thumbnail};
use super::cache::{generate_cache_key, get_cached_thumbnail, get_cached_preview, save_thumbnail_to_cache};
use super::video::generate_video_thumbnail;


// Function to generate a JPEG thumbnail from an image file
pub fn generate_thumbnail(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    
    log::debug!("Generating thumbnail for: {file_path}");
    
    // Check if file exists
    if !path.exists() {
        log::warn!("File does not exist for thumbnail generation: {file_path}");
        return None;
    }
    
    // Generate cache key
    let cache_key = generate_cache_key(file_path);
    log::trace!("Generated cache key for thumbnail: {cache_key}");
    
    // Check disk cache first
    if let Some(cached) = get_cached_thumbnail(&cache_key) {
        log::debug!("Using cached thumbnail for: {file_path}");
        return Some(cached);
    }
    
    log::debug!("No cached thumbnail found, generating new one for: {file_path}");
    
    // Check file extension for supported formats
    if let Some(extension) = path.extension() {
        let ext_str = extension.to_string_lossy().to_lowercase();
        log::trace!("File extension detected: {ext_str}");
        
        match ext_str.as_str() {
            // Video formats - generate thumbnail from first frame
            "mp4" | "avi" | "mov" | "wmv" | "flv" | "webm" | "mkv" | "m4v" | "3gp" | "ogv" => {
                log::info!("Processing video thumbnail: {file_path}");
                
                if let Some(thumbnail_base64) = generate_video_thumbnail(file_path) {
                    // Decode base64 to get JPEG bytes for caching
                    if let Ok(jpeg_bytes) = BASE64.decode(&thumbnail_base64) {
                        // Save to disk cache
                        if let Err(e) = save_thumbnail_to_cache(&cache_key, &jpeg_bytes) {
                            log::warn!("Failed to cache video thumbnail: {e}");
                        } else {
                            log::trace!("Successfully cached video thumbnail");
                        }
                    }
                    log::info!("Successfully generated video thumbnail");
                    Some(thumbnail_base64)
                } else {
                    log::warn!("Failed to generate video thumbnail for: {file_path}");
                    None
                }
            }          
            _ => {
                if let Some(result) = generate_magick_thumbnail(file_path) {
                    log::info!("Successfully generated thumbnail for file using magick");
                    return Some(result)
                } else {
                    log::error!("Failed creating a thumbnail for: {file_path}");
                    None                    
                }
            }
        }
    } else {
        log::warn!("No file extension found for: {file_path}");
        None
    }
}

pub fn generate_preview(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);

    log::debug!("Preview requested for: {file_path}");

    // Check if file exists
    if !path.exists() {
        log::warn!("File does not exist in scan dir: {file_path}");
        return None;
    }
    
    // Generate cache key
    let cache_key = generate_cache_key(file_path);
    log::trace!("The cache key: {cache_key}");
    
    // Check disk cache first
    if let Some(cached) = get_cached_preview(&cache_key) {
        log::debug!("Using cached preview for: {file_path}");
        return Some(cached);
    }
    
    log::debug!("No cached preview found, generating new one for: {file_path}");

    if let Some(result) = generate_magick_preview(file_path) {
        log::info!("Successfully generated thumbnail for file using magick");
        return Some(result)
    } else {
        log::error!("Failed creating a thumbnail for: {file_path}");
        None                    
    }  
}
