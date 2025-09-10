use std::fs::File;
use image::{DynamicImage, RgbImage};
use tiff;

use super::cache::save_full_image_to_cache;

// Shared function for TIFF to RGB JPEG (for both thumbnail and preview)
pub fn convert_tiff_to_rgb_jpeg(
    file_path: &str,
    max_dimension: u32,
    jpeg_quality: u8,
    cache_key: Option<&str>,
    save_to_cache: Option<fn(&str, &[u8]) -> std::io::Result<()>>,
) -> Result<Vec<u8>, String> {
    log::info!("Processing TIFF file with tiff crate: {}", file_path);
    
    let file = File::open(file_path)
        .map_err(|e| {
            log::error!("Failed to open TIFF file {}: {:?}", file_path, e);
            format!("Failed to open TIFF file {}: {:?}", file_path, e)
        })?;
    
    log::debug!("Successfully opened TIFF file: {}", file_path);
    
    let mut decoder = tiff::decoder::Decoder::new(file)
        .map_err(|e| {
            log::error!("Failed to create TIFF decoder for {}: {:?}", file_path, e);
            format!("Failed to create TIFF decoder for {}: {:?}", file_path, e)
        })?
        .with_limits(tiff::decoder::Limits::unlimited());
    
    log::trace!("Created TIFF decoder with unlimited limits");
    
    let (width, height) = decoder.dimensions()
        .map_err(|e| {
            log::error!("Failed to get TIFF dimensions for {}: {:?}", file_path, e);
            format!("Failed to get TIFF dimensions for {}: {:?}", file_path, e)
        })?;
    
    log::info!("TIFF dimensions: {}x{}", width, height);
    
    match decoder.read_image() {
        Ok(tiff::decoder::DecodingResult::U8(data)) => {
            // Detect color type
            let color_type = decoder.colortype().unwrap_or(tiff::ColorType::RGB(8));
            log::debug!("TIFF color type: {:?}", color_type);

            let rgb_data = match color_type {
                tiff::ColorType::Gray(nbits) => {
                    log::info!("TIFF is greyscale ({} bits), converting to RGB", nbits);
                    // Convert grayscale to RGB by duplicating each value
                    data.iter().flat_map(|v| std::iter::repeat(*v).take(3)).collect::<Vec<u8>>()
                }
                tiff::ColorType::RGB(_) => {
                    data
                }
                tiff::ColorType::YCbCr(_) => {
                    log::info!("TIFF is YCbCr, converting to RGB");
                    let mut rgb_data = Vec::with_capacity(data.len());
                    for chunk in data.chunks_exact(3) {
                        let y = chunk[0] as f32;
                        let cb = chunk[1] as f32 - 128.0;
                        let cr = chunk[2] as f32 - 128.0;

                        let r = (y + 1.402 * cr).clamp(0.0, 255.0) as u8;
                        let g = (y - 0.344136 * cb - 0.714136 * cr).clamp(0.0, 255.0) as u8;
                        let b = (y + 1.772 * cb).clamp(0.0, 255.0) as u8;

                        rgb_data.push(r);
                        rgb_data.push(g);
                        rgb_data.push(b);
                    }
                    rgb_data
                }
                _ => {
                    log::warn!("TIFF color type not handled: {:?}", color_type);
                    data
                }
            };

            let rgb_width = width;
            let rgb_height = height;
            let rgb_img = RgbImage::from_raw(rgb_width, rgb_height, rgb_data);

            if let Some(rgb_img) = rgb_img {
                log::trace!("Created RGB image from raw data");
                
                let dynamic_img = DynamicImage::ImageRgb8(rgb_img);
                let scaled_img = if width > max_dimension || height > max_dimension {
                    log::debug!("Large TIFF image ({}x{}), using progressive scaling to {}", width, height, max_dimension);
                    let intermediate = dynamic_img.resize(800, 800, image::imageops::FilterType::Triangle);
                    intermediate.resize(max_dimension, max_dimension, image::imageops::FilterType::CatmullRom)
                } else {
                    log::debug!("Small TIFF image ({}x{}), direct scaling to {}", width, height, max_dimension);
                    dynamic_img.resize(max_dimension, max_dimension, image::imageops::FilterType::CatmullRom)
                };
                
                log::trace!("Image scaling completed");
                
                let mut jpeg_bytes = Vec::new();
                match scaled_img.write_with_encoder(
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, jpeg_quality)
                ) {
                    Ok(_) => {
                        log::debug!("Successfully encoded TIFF as JPEG, size: {} bytes, quality: {}", jpeg_bytes.len(), jpeg_quality);
                        
                        if let (Some(key), Some(save_fn)) = (cache_key, save_to_cache) {
                            match save_fn(key, &jpeg_bytes) {
                                Ok(_) => log::trace!("Saved TIFF result to cache"),
                                Err(e) => log::warn!("Failed to save TIFF result to cache: {}", e),
                            }
                        }
                        Ok(jpeg_bytes)
                    },
                    Err(e) => {
                        log::error!("JPEG encoding failed for TIFF {}: {:?}", file_path, e);
                        Err("JPEG encoding failed".to_string())
                    }
                }
            } else {
                log::error!("Failed to create RGB image from TIFF data for {}", file_path);
                Err("Failed to create RGB image from TIFF data for {}".to_string())
            }
        }
        Ok(tiff::decoder::DecodingResult::U16(data)) => {
            let color_type = decoder.colortype().unwrap_or(tiff::ColorType::RGB(16));
            log::debug!("TIFF color type: {:?}", color_type);

            let rgb_data: Vec<u8> = match color_type {
                tiff::ColorType::Gray(_nbits) => {
                    log::info!("TIFF is 16-bit greyscale, converting to 8-bit RGB");
                    // Convert grayscale to RGB by duplicating each value
                    data.iter().flat_map(|x| {
                        let v = (x >> 8) as u8;
                        [v, v, v]
                    }).collect()
                }
                tiff::ColorType::RGB(_) => {
                    data.iter().map(|&x| (x >> 8) as u8).collect()
                }
                tiff::ColorType::YCbCr(_) => {
                    log::info!("TIFF is 16-bit YCbCr, converting to RGB");
                    let mut rgb_data = Vec::with_capacity(data.len());
                    for chunk in data.chunks_exact(3) {
                        let y = (chunk[0] >> 8) as f32;
                        let cb = (chunk[1] >> 8) as f32 - 128.0;
                        let cr = (chunk[2] >> 8) as f32 - 128.0;

                        let r = (y + 1.402 * cr).clamp(0.0, 255.0) as u8;
                        let g = (y - 0.344136 * cb - 0.714136 * cr).clamp(0.0, 255.0) as u8;
                        let b = (y + 1.772 * cb).clamp(0.0, 255.0) as u8;

                        rgb_data.push(r);
                        rgb_data.push(g);
                        rgb_data.push(b);
                    }
                    rgb_data
                }
                _ => {
                    log::warn!("TIFF color type not handled: {:?}", color_type);
                    data.iter().map(|&x| (x >> 8) as u8).collect()
                }
            };

            let rgb_img = RgbImage::from_raw(width, height, rgb_data);
            if let Some(rgb_img) = rgb_img {
                log::trace!("Created RGB image from 16-bit converted data");
                
                let dynamic_img = DynamicImage::ImageRgb8(rgb_img);
                let scaled_img = if width > max_dimension || height > max_dimension {
                    log::debug!("Large 16-bit TIFF image ({}x{}), using progressive scaling", width, height);
                    let intermediate = dynamic_img.resize(800, 800, image::imageops::FilterType::Triangle);
                    intermediate.resize(max_dimension, max_dimension, image::imageops::FilterType::CatmullRom)
                } else {
                    log::debug!("Small 16-bit TIFF image ({}x{}), direct scaling", width, height);
                    dynamic_img.resize(max_dimension, max_dimension, image::imageops::FilterType::CatmullRom)
                };
                
                let mut jpeg_bytes = Vec::new();
                match scaled_img.write_with_encoder(
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, jpeg_quality)
                ) {
                    Ok(_) => {
                        log::debug!("Successfully encoded 16-bit TIFF as JPEG, size: {} bytes", jpeg_bytes.len());
                        
                        if let (Some(key), Some(save_fn)) = (cache_key, save_to_cache) {
                            match save_fn(key, &jpeg_bytes) {
                                Ok(_) => log::trace!("Saved 16-bit TIFF result to cache"),
                                Err(e) => log::warn!("Failed to save 16-bit TIFF result to cache: {}", e),
                            }
                        }
                        Ok(jpeg_bytes)
                    },
                    Err(e) => {
                        log::error!("JPEG encoding failed for 16-bit TIFF {}: {:?}", file_path, e);
                        Err("JPEG encoding failed for 16-bit TIFF".to_string())
                    }
                }
            } else {
                log::error!("Failed to create RGB image from 16-bit TIFF data for {}", file_path);
                Err("Failed to create RGB image from 16-bit TIFF data".to_string())
            }
        }
        Ok(other_format) => {
            log::error!("Unsupported TIFF data format for {}: {:?}", file_path, other_format);
            Err(format!("Unsupported TIFF data format for {}", file_path))
        },
        Err(e) => {
            log::error!("Failed to read TIFF image data for {}: {:?}", file_path, e);
            Err(format!("Failed to read TIFF image data for {}: {:?}", file_path, e))
        },
    }
}

pub fn generate_tiff_preview(file_path: &str, cache_key: &str) -> Result<Vec<u8>, String> {
    log::info!("Generating TIFF preview for: {}", file_path);
    
    let result = convert_tiff_to_rgb_jpeg(
        file_path,
        1980,
        60,
        Some(cache_key),
        Some(save_full_image_to_cache),
    );
    
    match &result {
        Ok(bytes) => log::info!("Successfully generated TIFF preview, size: {} bytes", bytes.len()),
        Err(e) => log::error!("Failed to generate TIFF preview: {}", e),
    }
    
    result
}

pub fn generate_tiff_thumbnail(file_path: &str) -> Option<String> {
    log::info!("Generating TIFF thumbnail for: {}", file_path);
    
    let cache_key = super::cache::generate_cache_key(file_path);
    
    match convert_tiff_to_rgb_jpeg(
        file_path,
        200,
        50,
        Some(&cache_key),
        Some(super::cache::save_thumbnail_to_cache),
    ) {
        Ok(jpeg_bytes) => {
            log::debug!("TIFF thumbnail generation successful, encoding as base64");
            
            let base64_result = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg_bytes);
            log::info!("Successfully generated TIFF thumbnail, base64 length: {}", base64_result.len());
            Some(base64_result)
        }
        Err(e) => {
            log::error!("TIFF thumbnail generation failed for {}: {}", file_path, e);
            None
        }
    }
}