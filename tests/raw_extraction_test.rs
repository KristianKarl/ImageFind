#[cfg(test)]
mod tests {
    use std::path::Path;
    use image;
    
    // Import the actual processing functions from our codebase
    use image_find::processing::raw::{convert_raw_to_rgb_jpeg, generate_raw_thumbnail};

    // Test the problematic NEF file specifically
    #[test]
    fn test_nef_jpeg_extraction_2009_07_14() {
        let test_file = "tests/data/2009-07-14_115409.NEF";
        
        // Fail test if file doesn't exist
        if !Path::new(test_file).exists() {
            panic!("Test file {} not found! Please ensure the test data is in the correct location.", test_file);
        }
        
        println!("Testing JPEG extraction from: {}", test_file);
        
        // Test thumbnail generation using the actual codebase function
        match generate_raw_thumbnail(test_file) {
            Some(thumbnail_base64) => {
                println!("Successfully generated thumbnail, base64 length: {}", thumbnail_base64.len());
                
                // Decode the base64 to verify it's a valid JPEG
                match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &thumbnail_base64) {
                    Ok(jpeg_bytes) => {
                        println!("Decoded thumbnail JPEG: {} bytes", jpeg_bytes.len());
                        
                        // Try to load the JPEG to verify it's valid
                        match image::load_from_memory(&jpeg_bytes) {
                            Ok(img) => {
                                let (w, h) = (img.width(), img.height());
                                println!("Valid thumbnail image: {}x{} pixels", w, h);
                                
                                // Save test output for verification
                                let output_path = "test_output_nef_thumbnail.jpg";
                                if let Err(e) = img.save(output_path) {
                                    println!("Failed to save test output: {}", e);
                                } else {
                                    println!("Saved test output to: {}", output_path);
                                }
                                
                                // Test should pass if we get here
                                assert!(w > 0 && h > 0, "Generated thumbnail has invalid dimensions");
                            }
                            Err(e) => {
                                panic!("Generated thumbnail is not a valid image: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        panic!("Failed to decode base64 thumbnail: {}", e);
                    }
                }
            }
            None => {
                panic!("Failed to generate thumbnail from NEF file using actual codebase functions");
            }
        }
        
        // Also test preview generation
        println!("Testing preview generation...");
        let cache_key = "test_nef_preview";
        match convert_raw_to_rgb_jpeg(test_file, 800, 80, Some(cache_key), None) {
            Ok(preview_bytes) => {
                println!("Successfully generated preview: {} bytes", preview_bytes.len());
                
                // Verify the preview is a valid JPEG
                match image::load_from_memory(&preview_bytes) {
                    Ok(img) => {
                        let (w, h) = (img.width(), img.height());
                        println!("Valid preview image: {}x{} pixels", w, h);
                        
                        // Save preview test output
                        let output_path = "test_output_nef_preview.jpg";
                        if let Err(e) = img.save(output_path) {
                            println!("Failed to save preview test output: {}", e);
                        } else {
                            println!("Saved preview test output to: {}", output_path);
                        }
                        
                        assert!(w > 0 && h > 0, "Generated preview has invalid dimensions");
                    }
                    Err(e) => {
                        panic!("Generated preview is not a valid image: {}", e);
                    }
                }
            }
            Err(e) => {
                panic!("Failed to generate preview from NEF file: {}", e);
            }
        }
        
        println!("All NEF extraction tests passed!");
    }
}