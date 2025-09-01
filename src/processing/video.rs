use std::process::Command;
use std::env;
use image;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::fs;

use super::cache::{generate_cache_key};

// Function to generate a video thumbnail using ffmpeg binary
pub fn generate_video_thumbnail(file_path: &str) -> Option<String> {
    log::info!("Generating video thumbnail for: {}", file_path);
    
    // Create a temporary file for the thumbnail
    let temp_dir = env::temp_dir();
    let temp_thumbnail = temp_dir.join(format!("thumb_{}.jpg", generate_cache_key(file_path)));
    
    log::debug!("Using temporary file for video thumbnail: {}", temp_thumbnail.display());
    
    // Use ffmpeg to extract the first frame
    let output = Command::new("ffmpeg")
        .args(&[
            "-i", file_path,           // Input file
            "-vf", "scale=200:200:force_original_aspect_ratio=decrease,pad=200:200:(ow-iw)/2:(oh-ih)/2", // Scale and pad to 200x200
            "-vframes", "1",           // Extract only 1 frame
            "-q:v", "2",              // High quality
            "-y",                     // Overwrite output file
            temp_thumbnail.to_str()?  // Output file
        ])
        .output();
    
    match output {
        Ok(result) => {
            if result.status.success() {
                log::debug!("ffmpeg completed successfully for: {}", file_path);
                
                if temp_thumbnail.exists() {
                    log::trace!("Temporary thumbnail file created: {}", temp_thumbnail.display());
                    
                    // Read the generated thumbnail
                    match fs::read(&temp_thumbnail) {
                        Ok(thumbnail_bytes) => {
                            log::debug!("Read thumbnail data, size: {} bytes", thumbnail_bytes.len());
                            
                            // Clean up temp file
                            if let Err(e) = fs::remove_file(&temp_thumbnail) {
                                log::warn!("Failed to clean up temp thumbnail file {}: {}", temp_thumbnail.display(), e);
                            } else {
                                log::trace!("Cleaned up temporary thumbnail file");
                            }
                            
                            // Try to open with image crate
                            match image::load_from_memory(&thumbnail_bytes) {
                                Ok(img) => {
                                    log::trace!("Successfully loaded thumbnail image with image crate");
                                    // Convert back to JPEG bytes
                                    let mut jpeg_bytes = Vec::new();
                                    match img.write_with_encoder(
                                        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 50)
                                    ) {
                                        Ok(_) => {
                                            log::debug!("Successfully processed video thumbnail, final size: {} bytes", jpeg_bytes.len());
                                            return Some(BASE64.encode(&jpeg_bytes));
                                        },
                                        Err(e) => {
                                            log::warn!("Failed to encode video thumbnail as JPEG: {:?}", e);
                                        }
                                    }
                                },
                                Err(e) => {
                                    log::warn!("Failed to load thumbnail with image crate: {:?}", e);
                                }
                            }
                            
                            // If rotation fails, return the original thumbnail
                            log::debug!("Using original ffmpeg output as thumbnail");
                            return Some(BASE64.encode(&thumbnail_bytes));
                        },
                        Err(e) => {
                            log::error!("Failed to read generated thumbnail file {}: {}", temp_thumbnail.display(), e);
                        }
                    }
                } else {
                    log::warn!("ffmpeg completed but thumbnail file was not created: {}", temp_thumbnail.display());
                }
            } else {
                log::error!("ffmpeg failed for video {}: {}", file_path, String::from_utf8_lossy(&result.stderr));
            }
            
            // Clean up temp file if it exists
            if temp_thumbnail.exists() {
                if let Err(e) = fs::remove_file(&temp_thumbnail) {
                    log::warn!("Failed to clean up temp file after error {}: {}", temp_thumbnail.display(), e);
                }
            }
        }
        Err(e) => {
            log::error!("Failed to execute ffmpeg for video {}: {}", file_path, e);
            
            // Clean up temp file if it exists
            if temp_thumbnail.exists() {
                if let Err(e) = fs::remove_file(&temp_thumbnail) {
                    log::warn!("Failed to clean up temp file after execution error {}: {}", temp_thumbnail.display(), e);
                }
            }
        }
    }
    
    log::warn!("Video thumbnail generation failed for: {}", file_path);
    None
}