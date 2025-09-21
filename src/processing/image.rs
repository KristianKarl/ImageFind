use std::path::Path;
use image;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::processing::raw::generate_raw_preview;

use super::cache::{generate_cache_key, get_cached_thumbnail, get_cached_preview, save_thumbnail_to_cache};
use super::raw::generate_raw_thumbnail;
use super::tiff::{generate_tiff_thumbnail,generate_tiff_preview};
use super::video::generate_video_thumbnail;

// Function to generate a JPEG thumbnail from an image file
pub fn generate_thumbnail(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    
    log::debug!("Generating thumbnail for: {}", file_path);
    
    // Check if file exists
    if !path.exists() {
        log::warn!("File does not exist for thumbnail generation: {}", file_path);
        return None;
    }
    
    // Generate cache key
    let cache_key = generate_cache_key(file_path);
    log::trace!("Generated cache key for thumbnail: {}", cache_key);
    
    // Check disk cache first
    if let Some(cached) = get_cached_thumbnail(&cache_key) {
        log::debug!("Using cached thumbnail for: {}", file_path);
        return Some(cached);
    }
    
    log::debug!("No cached thumbnail found, generating new one for: {}", file_path);
    
    // Check file extension for supported formats
    if let Some(extension) = path.extension() {
        let ext_str = extension.to_string_lossy().to_lowercase();
        log::trace!("File extension detected: {}", ext_str);
        
        match ext_str.as_str() {
            // RAW files - use rawloader crate with RGB demosaicing
            "nef" | "cr2" | "cr3" | "arw" | "orf" | "rw2" | "raf" | "dng" => {
                log::info!("Processing RAW file thumbnail: {}", file_path);
                
                if let Some(result) = generate_raw_thumbnail(file_path) {
                    log::info!("Successfully generated RAW thumbnail using rawloader");
                    return Some(result);
                } else {
                    log::error!("RAW thumbna processing failed: {}", file_path);
                    return None;
                }
            }
            // TIFF files - use specialized tiff crate
            "tiff" | "tif" => {
                log::info!("Processing TIFF file thumbnail: {}", file_path);
                
                // Try the specialized TIFF handler first
                if let Some(result) = generate_tiff_thumbnail(file_path) {
                    log::info!("Successfully generated TIFF thumbnail using specialized handler");
                    return Some(result);
                }

                None
            }
            // Standard image formats
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" |
            // Other RAW formats not fully supported by rawloader
            "3fr" | "ari" | "bay" | "crw" | "dcr" | "erf" | "fff" | "iiq" | 
            "k25" | "kdc" | "mdc" | "mos" | "mrw" | "pef" | "ptx" | "pxn" | 
            "r3d" | "rwl" | "sr2" | "srf" | "srw" | "x3f" => {
                log::debug!("Processing standard/other RAW format thumbnail: {}", file_path);
                
                // Try to load and resize the image
                match image::open(path) {
                    Ok(img) => {
                        // Get original dimensions for optimization
                        let (original_width, original_height) = (img.width(), img.height());
                        log::debug!("Original image dimensions: {}x{}", original_width, original_height);
                        
                        // Early check: if image is very small, use it directly
                        if original_width <= 400 && original_height <= 400 {
                            log::trace!("Very small image, using direct conversion");
                            // Very small image: convert to base64
                            let mut jpeg_bytes = Vec::new();
                            if img.write_with_encoder(
                                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 50)
                            ).is_ok() {
                                let base64_result = BASE64.encode(&jpeg_bytes);
                                let _ = save_thumbnail_to_cache(&cache_key, &jpeg_bytes);
                                log::debug!("Successfully processed small image thumbnail");
                                return Some(base64_result);
                            }
                        }

                        // Optimize thumbnail generation based on image size
                        let thumbnail = if original_width > 2000 || original_height > 2000 {
                            log::trace!("Large image, using progressive scaling");
                            // Large image: use progressive scaling for better performance
                            let intermediate = img.resize(
                                800, 
                                800, 
                                image::imageops::FilterType::Triangle // Fast first pass
                            );
                            intermediate.resize(
                                200, 
                                200, 
                                image::imageops::FilterType::CatmullRom // High quality final pass
                            )
                        } else {
                            log::trace!("Medium image, using direct scaling");
                            // Smaller image: direct scaling with high quality
                            img.resize(
                                200, 
                                200, 
                                image::imageops::FilterType::CatmullRom
                            )
                        };

                        // Convert to JPEG and encode as base64
                        let mut jpeg_bytes = Vec::new();
                        if thumbnail.write_with_encoder(
                            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 50)
                        ).is_ok() {
                            let base64_result = BASE64.encode(&jpeg_bytes);
                            // Save to disk cache
                            let _ = save_thumbnail_to_cache(&cache_key, &jpeg_bytes);
                            log::info!("Successfully generated standard image thumbnail");
                            return Some(base64_result);
                        }
                        
                        log::error!("JPEG encoding failed for thumbnail: {}", file_path);
                        // If JPEG encoding failed, return None
                        None
                    }
                    Err(e) => {
                        // Log the error for debugging
                        log::warn!("Failed to process image with standard method {}: {:?}", file_path, e);
                        
                        // For RAW formats that might not be supported by the image crate,
                        // try rawloader as a fallback
                        match e {
                            image::ImageError::Unsupported(_) => {
                                log::info!("Unsupported format for {}: {}. Trying rawloader fallback...", file_path, ext_str);
                                
                                // Try rawloader for RAW formats
                                match ext_str.as_str() {
                                    "nef" | "cr2" | "cr3" | "arw" | "orf" | "rw2" | "raf" | "dng" | 
                                    "3fr" | "ari" | "bay" | "crw" | "dcr" | "erf" | "fff" | "iiq" | 
                                    "k25" | "kdc" | "mdc" | "mos" | "mrw" | "pef" | "ptx" | "pxn" | 
                                    "r3d" | "rwl" | "sr2" | "srf" | "srw" | "x3f" => {
                                        log::debug!("Attempting rawloader fallback for unsupported RAW format");
                                        if let Some(result) = generate_raw_thumbnail(file_path) {
                                            log::info!("Successfully generated thumbnail using rawloader fallback");
                                            return Some(result);
                                        }
                                        log::warn!("Rawloader fallback also failed for: {}", file_path);
                                    }
                                    _ => {
                                        log::debug!("No fallback available for unsupported format: {}", ext_str);
                                    }
                                }
                                
                                // If rawloader failed, no other options
                                log::error!("All processing methods failed for: {}", file_path);
                                return None;
                            }
                            _ => {
                                // For other errors, no fallback available
                                log::error!("Image processing error for {}: {:?}", file_path, e);
                                None
                            }
                        }
                    }
                }
            }
            // Video formats - generate thumbnail from first frame
            "mp4" | "avi" | "mov" | "wmv" | "flv" | "webm" | "mkv" | "m4v" | "3gp" | "ogv" => {
                log::info!("Processing video thumbnail: {}", file_path);
                
                if let Some(thumbnail_base64) = generate_video_thumbnail(file_path) {
                    // Decode base64 to get JPEG bytes for caching
                    if let Ok(jpeg_bytes) = BASE64.decode(&thumbnail_base64) {
                        // Save to disk cache
                        if let Err(e) = save_thumbnail_to_cache(&cache_key, &jpeg_bytes) {
                            log::warn!("Failed to cache video thumbnail: {}", e);
                        } else {
                            log::trace!("Successfully cached video thumbnail");
                        }
                    }
                    log::info!("Successfully generated video thumbnail");
                    Some(thumbnail_base64)
                } else {
                    log::warn!("Failed to generate video thumbnail for: {}", file_path);
                    None
                }
            }
            _ => {
                log::debug!("Unsupported file extension for thumbnail: {}", ext_str);
                None
            },
        }
    } else {
        log::warn!("No file extension found for: {}", file_path);
        None
    }
}

pub fn generate_preview(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    
    log::debug!("Generating preview for: {}", file_path);
    
    // Check if file exists
    if !path.exists() {
        log::warn!("File does not exist for preview generation: {}", file_path);
        return None;
    }
    
    // Generate cache key
    let cache_key = generate_cache_key(file_path);
    log::trace!("Generated cache key for preview: {}", cache_key);
    
    // Check disk cache first
    if let Some(cached) = get_cached_preview(&cache_key) {
        log::debug!("Using cached preview for: {}", file_path);
        return Some(cached);
    }
    
    log::debug!("No cached preview found, generating new one for: {}", file_path);
    
    // Check file extension for supported formats
    if let Some(extension) = path.extension() {
        let ext_str = extension.to_string_lossy().to_lowercase();
        log::trace!("File extension detected: {}", ext_str);
        
        match ext_str.as_str() {
            "nef" | "cr2" | "cr3" | "arw" | "orf" | "rw2" | "raf" | "dng" => {
                log::info!("Processing RAW file preview: {}", file_path);
                
                if let Some(result) = generate_raw_preview(file_path) {
                    log::info!("Successfully generated RAW preview using rawloader");
                    return Some(result);
                } else {
                    log::error!("RAW preview processing failed: {}", file_path);
                    return None;
                }
            }
            // TIFF files - use specialized tiff crate
            "tiff" | "tif" => {
                log::info!("Processing TIFF file preview: {}", file_path);
                
                // Try the specialized TIFF handler first
                if let Some(result) = generate_tiff_preview(file_path) {
                    log::info!("Successfully generated TIFF preview using specialized handler");
                    return Some(result);
                }

                None
            }
            // Standard image formats
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" |
            // Other RAW formats not fully supported by rawloader
            "3fr" | "ari" | "bay" | "crw" | "dcr" | "erf" | "fff" | "iiq" | 
            "k25" | "kdc" | "mdc" | "mos" | "mrw" | "pef" | "ptx" | "pxn" | 
            "r3d" | "rwl" | "sr2" | "srf" | "srw" | "x3f" => {
                log::debug!("Processing standard/other RAW format thumbnail: {}", file_path);
                
                // Try to load and resize the image
                match image::open(path) {
                    Ok(img) => {
                        // Get original dimensions for optimization
                        let (original_width, original_height) = (img.width(), img.height());
                        log::debug!("Original image dimensions: {}x{}", original_width, original_height);
                        
                        // Early check: if image is very small, use it directly
                        if original_width <= 400 && original_height <= 400 {
                            log::trace!("Very small image, using direct conversion");
                            // Very small image: convert to base64
                            let mut jpeg_bytes = Vec::new();
                            if img.write_with_encoder(
                                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 50)
                            ).is_ok() {
                                let base64_result = BASE64.encode(&jpeg_bytes);
                                let _ = save_thumbnail_to_cache(&cache_key, &jpeg_bytes);
                                log::debug!("Successfully processed small image thumbnail");
                                return Some(base64_result);
                            }
                        }

                        // Optimize thumbnail generation based on image size
                        let thumbnail = if original_width > 2000 || original_height > 2000 {
                            log::trace!("Large image, using progressive scaling");
                            // Large image: use progressive scaling for better performance
                            let intermediate = img.resize(
                                800, 
                                800, 
                                image::imageops::FilterType::Triangle // Fast first pass
                            );
                            intermediate.resize(
                                200, 
                                200, 
                                image::imageops::FilterType::CatmullRom // High quality final pass
                            )
                        } else {
                            log::trace!("Medium image, using direct scaling");
                            // Smaller image: direct scaling with high quality
                            img.resize(
                                200, 
                                200, 
                                image::imageops::FilterType::CatmullRom
                            )
                        };

                        // Convert to JPEG and encode as base64
                        let mut jpeg_bytes = Vec::new();
                        if thumbnail.write_with_encoder(
                            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 50)
                        ).is_ok() {
                            let base64_result = BASE64.encode(&jpeg_bytes);
                            // Save to disk cache
                            let _ = save_thumbnail_to_cache(&cache_key, &jpeg_bytes);
                            log::info!("Successfully generated standard image thumbnail");
                            return Some(base64_result);
                        }
                        
                        log::error!("JPEG encoding failed for thumbnail: {}", file_path);
                        // If JPEG encoding failed, return None
                        None
                    }
                    Err(e) => {
                        // Log the error for debugging
                        log::warn!("Failed to process image with standard method {}: {:?}", file_path, e);
                        
                        // For RAW formats that might not be supported by the image crate,
                        // try rawloader as a fallback
                        match e {
                            image::ImageError::Unsupported(_) => {
                                log::info!("Unsupported format for {}: {}. Trying rawloader fallback...", file_path, ext_str);
                                
                                // Try rawloader for RAW formats
                                match ext_str.as_str() {
                                    "nef" | "cr2" | "cr3" | "arw" | "orf" | "rw2" | "raf" | "dng" | 
                                    "3fr" | "ari" | "bay" | "crw" | "dcr" | "erf" | "fff" | "iiq" | 
                                    "k25" | "kdc" | "mdc" | "mos" | "mrw" | "pef" | "ptx" | "pxn" | 
                                    "r3d" | "rwl" | "sr2" | "srf" | "srw" | "x3f" => {
                                        log::debug!("Attempting rawloader fallback for unsupported RAW format");
                                        if let Some(result) = generate_raw_thumbnail(file_path) {
                                            log::info!("Successfully generated thumbnail using rawloader fallback");
                                            return Some(result);
                                        }
                                        log::warn!("Rawloader fallback also failed for: {}", file_path);
                                    }
                                    _ => {
                                        log::debug!("No fallback available for unsupported format: {}", ext_str);
                                    }
                                }
                                
                                // If rawloader failed, no other options
                                log::error!("All processing methods failed for: {}", file_path);
                                return None;
                            }
                            _ => {
                                // For other errors, no fallback available
                                log::error!("Image processing error for {}: {:?}", file_path, e);
                                None
                            }
                        }
                    }
                }
            }
            // Video formats - generate thumbnail from first frame
            "mp4" | "avi" | "mov" | "wmv" | "flv" | "webm" | "mkv" | "m4v" | "3gp" | "ogv" => {
                log::info!("Processing video thumbnail: {}", file_path);
                
                if let Some(thumbnail_base64) = generate_video_thumbnail(file_path) {
                    // Decode base64 to get JPEG bytes for caching
                    if let Ok(jpeg_bytes) = BASE64.decode(&thumbnail_base64) {
                        // Save to disk cache
                        if let Err(e) = save_thumbnail_to_cache(&cache_key, &jpeg_bytes) {
                            log::warn!("Failed to cache video thumbnail: {}", e);
                        } else {
                            log::trace!("Successfully cached video thumbnail");
                        }
                    }
                    log::info!("Successfully generated video thumbnail");
                    Some(thumbnail_base64)
                } else {
                    log::warn!("Failed to generate video thumbnail for: {}", file_path);
                    None
                }
            }
            _ => {
                log::debug!("Unsupported file extension for thumbnail: {}", ext_str);
                None
            },
        }
    } else {
        log::warn!("No file extension found for: {}", file_path);
        None
    }
}
