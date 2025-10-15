#[cfg(test)]
mod tests {
    
    use std::fs;


    // Import the actual processing functions from our codebase
    use image_find::cli::{init_logging, CliArgs, LogLevel, CLI_ARGS};
    use image_find::background::{start_background_thumbnail_worker, start_background_preview_worker};
    use image_find::sidecar_scan;
        
    #[test]
    fn test_run() {
        // Initialize app logging via CliArgs at TRACE level, and set test cache paths
        let _ = {
            let args = CliArgs {
                db_path: "tests/tmp/test.sqlite".to_string(),
                thumbnail_cache: "tests/tmp/thumb_cache".to_string(),
                full_image_cache: "tests/tmp/full_cache".to_string(),
                video_preview_cache: "tests/tmp/video_preview_cache".to_string(),
                scan_dir: "tests/data".to_string(),
                log_level: LogLevel::Trace,
                port: 8080,
            };

            // Clean up and recreate test folders
            let _ = fs::remove_dir_all("tests/tmp");
            let _ = fs::create_dir_all(&args.thumbnail_cache);
            let _ = fs::create_dir_all(&args.full_image_cache);
            let _ = fs::create_dir_all(&args.video_preview_cache);

            // Set CLI args once (ignore error if already set by a prior test run)
            let _ = CLI_ARGS.set(args.clone());
            init_logging(&args);
            Ok::<(), ()>(())
        };

        log::trace!("TRACE logging initialized for tests via CliArgs: {:?}", CLI_ARGS.get().unwrap());
        if let Err(e) = sidecar_scan::scan_and_import_sidecars() {
            eprintln!("Error importing sidecars: {e}");
        }

        start_background_thumbnail_worker();
        start_background_preview_worker();

        // Wait for background workers to process (adjust duration as needed)
        std::thread::sleep(std::time::Duration::from_secs(120));
    }
}
