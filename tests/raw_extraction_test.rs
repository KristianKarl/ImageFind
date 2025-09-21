#[cfg(test)]
mod tests {
    use image;
    use std::fs;
    use std::path::Path;
    use walkdir::WalkDir;

    // Import the actual processing functions from our codebase
    use image_find::cli::{init_logging, CliArgs, LogLevel, CLI_ARGS};
    use image_find::processing::raw::{generate_raw_preview, generate_raw_thumbnail};

    // Test the problematic NEF file specifically
    #[test]
    fn test_jpeg_extraction() {
        // Initialize app logging via CliArgs at TRACE level, and set test cache paths
        let _ = (|| {
            let args = CliArgs {
                db_path: "tests/tmp/test.sqlite".to_string(),
                thumbnail_cache: "tests/tmp/thumb_cache".to_string(),
                full_image_cache: "tests/tmp/full_cache".to_string(),
                video_preview_cache: "tests/tmp/video_preview_cache".to_string(),
                scan_dir: "tests/data".to_string(),
                log_level: LogLevel::Trace,
                port: 8080,
            };

            // Ensure directories exist
            let _ = fs::create_dir_all(&args.thumbnail_cache);
            let _ = fs::create_dir_all(&args.full_image_cache);
            let _ = fs::create_dir_all(&args.video_preview_cache);
            let _ = fs::create_dir_all("tests/tmp");

            // Set CLI args once (ignore error if already set by a prior test run)
            let _ = CLI_ARGS.set(args.clone());
            init_logging(&args);
            Ok::<(), ()>(())
        })();

        log::trace!("TRACE logging initialized for tests via CliArgs");

        let data_dir = Path::new("tests/data");
        assert!(data_dir.exists(), "tests/data directory not found");

        // File extensions to test (RAW-centric; extend as needed)
        let raw_exts = [
            "nef", "NEF", "raf", "RAF", "cr2", "CR2", "cr3", "CR3", "arw", "ARW", "orf", "ORF",
            "rw2", "RW2", "dng", "DNG", "tif", "TIF", "tiff", "TIFF",
        ];

        let mut tested = 0usize;

        for entry in WalkDir::new(data_dir).into_iter().filter_map(Result::ok) {
            if !entry.path().is_file() {
                continue;
            }
            let path = entry.path();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !raw_exts.contains(&ext) {
                continue;
            }

            // Use absolute path for test_file
            let abs_path = match path.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    panic!("Failed to canonicalize test file {}: {}", path.display(), e);
                }
            };
            let test_file = abs_path.to_string_lossy().to_string();
            println!("Testing JPEG extraction from: {}", test_file);

            // Thumbnail generation
            match generate_raw_thumbnail(&test_file) {
                Some(thumbnail_base64) => {
                    println!(
                        "Successfully generated thumbnail, base64 length: {}",
                        thumbnail_base64.len()
                    );

                    // Decode the base64 to verify it's a valid JPEG
                    match base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        &thumbnail_base64,
                    ) {
                        Ok(jpeg_bytes) => {
                            println!("Decoded thumbnail JPEG: {} bytes", jpeg_bytes.len());

                            // Try to load the JPEG to verify it's valid
                            match image::load_from_memory(&jpeg_bytes) {
                                Ok(img) => {
                                    let (w, h) = (img.width(), img.height());
                                    println!("Valid thumbnail image: {}x{} pixels", w, h);

                                    // Save test output for verification per file
                                    let stem =
                                        path.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
                                    let output_path = format!("test_output_{}_thumbnail.jpg", stem);
                                    if let Err(e) = img.save(&output_path) {
                                        println!("Failed to save test output: {}", e);
                                    } else {
                                        println!("Saved test output to: {}", output_path);
                                    }

                                    assert!(
                                        w > 0 && h > 0,
                                        "Generated thumbnail has invalid dimensions"
                                    );
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
                    panic!(
                        "Failed to generate thumbnail from RAW file using actual codebase functions: {}",
                        test_file
                    );
                }
            }

            // Preview generation
            match generate_raw_preview(&test_file, "test_preview_cache_key") {
                Ok(preview_jpeg_bytes) => {
                    println!(
                        "Successfully generated preview JPEG: {} bytes",
                        preview_jpeg_bytes.len()
                    );

                    // Try to load the JPEG to verify it's valid
                    match image::load_from_memory(&preview_jpeg_bytes) {
                        Ok(img) => {
                            let (w, h) = (img.width(), img.height());
                            println!("Valid preview image: {}x{} pixels", w, h);

                            // Save test output for verification per file
                            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
                            let output_path = format!("test_output_{}_preview.jpg", stem);
                            if let Err(e) = img.save(&output_path) {
                                println!("Failed to save test output: {}", e);
                            } else {
                                println!("Saved test output to: {}", output_path);
                            }

                            assert!(w > 0 && h > 0, "Generated preview has invalid dimensions");
                        }
                        Err(e) => {
                            panic!("Generated preview is not a valid image: {}", e);
                        }
                    }
                }
                Err(e) => {
                    panic!(
                        "Failed to generate preview from RAW file using actual codebase functions: {} ({})",
                        test_file, e
                    );
                }
            }

            tested += 1;
        }

        assert!(tested > 0, "No RAW files found in tests/data to test");
        println!("All RAW extraction tests passed for {} files!", tested);
    }
}
