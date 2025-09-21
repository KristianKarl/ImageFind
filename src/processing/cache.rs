use std::fs;
use std::io;
use std::path::Path;
use sha2::{Sha256, Digest};

// Function to get thumbnail cache directory path
pub fn get_cache_dir() -> std::path::PathBuf {
    // Try to get from CLI args if available, otherwise use temp directory for tests
    match std::panic::catch_unwind(|| crate::cli::get_cli_args()) {
        Ok(args) => {
            let cache_dir = Path::new(&args.thumbnail_cache);
            if !cache_dir.exists() {
                log::info!("Creating thumbnail cache directory: {}", cache_dir.display());
                fs::create_dir_all(&cache_dir).expect("Failed to create cache directory");
            } else {
                log::trace!("Thumbnail cache directory exists: {}", cache_dir.display());
            }
            cache_dir.to_path_buf()
        }
        Err(_) => {
            // CLI args not available (probably in tests), use temp directory
            let temp_dir = std::env::temp_dir().join("imagefind_test_thumbnail_cache");
            if !temp_dir.exists() {
                fs::create_dir_all(&temp_dir).expect("Failed to create test cache directory");
            }
            temp_dir
        }
    }
}

// Function to get cache directory path for full images
pub fn get_full_image_cache_dir() -> std::path::PathBuf {
    // Try to get from CLI args if available, otherwise use temp directory for tests
    match std::panic::catch_unwind(|| crate::cli::get_cli_args()) {
        Ok(args) => {
            let cache_dir = Path::new(&args.full_image_cache);
            if !cache_dir.exists() {
                log::info!("Creating full image cache directory: {}", cache_dir.display());
                fs::create_dir_all(&cache_dir).expect("Failed to create full image cache directory");
            } else {
                log::trace!("Full image cache directory exists: {}", cache_dir.display());
            }
            cache_dir.to_path_buf()
        }
        Err(_) => {
            // CLI args not available (probably in tests), use temp directory
            let temp_dir = std::env::temp_dir().join("imagefind_test_full_image_cache");
            if !temp_dir.exists() {
                fs::create_dir_all(&temp_dir).expect("Failed to create test full image cache directory");
            }
            temp_dir
        }
    }
}

// Function to generate cache key from file path
pub fn generate_cache_key(file_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(file_path.as_bytes());
    let key = format!("{:x}", hasher.finalize());
    log::trace!("Generated cache key {} for file: {}", key, file_path);
    key
}

// Function to get cached thumbnail from disk
pub fn get_cached_thumbnail(cache_key: &str) -> Option<String> {
    let cache_dir = get_cache_dir();
    let cache_file = cache_dir.join(format!("{}.jpg", cache_key));
    
    log::trace!("Checking thumbnail cache for key: {}", cache_key);
    
    if cache_file.exists() {
        log::debug!("Found cached thumbnail: {}", cache_file.display());
        match fs::read(&cache_file) {
            Ok(bytes) => {
                log::trace!("Successfully read cached thumbnail, size: {} bytes", bytes.len());
                Some(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes))
            },
            Err(e) => {
                log::warn!("Failed to read cached thumbnail {}: {}", cache_file.display(), e);
                None
            }
        }
    } else {
        log::trace!("No cached thumbnail found for key: {}", cache_key);
        None
    }
}

// Function to save thumbnail to disk cache
pub fn save_thumbnail_to_cache(cache_key: &str, jpeg_bytes: &[u8]) -> io::Result<()> {
    let cache_dir = get_cache_dir();
    let cache_file = cache_dir.join(format!("{}.jpg", cache_key));
    
    log::debug!("Saving thumbnail to cache: {} ({} bytes)", cache_file.display(), jpeg_bytes.len());
    
    match fs::write(&cache_file, jpeg_bytes) {
        Ok(_) => {
            log::trace!("Successfully saved thumbnail to cache: {}", cache_file.display());
            Ok(())
        },
        Err(e) => {
            log::error!("Failed to save thumbnail to cache {}: {}", cache_file.display(), e);
            Err(e)
        }
    }
}

// Function to get cached full image from disk
pub fn get_cached_preview(cache_key: &str) -> Option<String> {
    let cache_dir = get_full_image_cache_dir();
    let cache_file = cache_dir.join(format!("{}.jpg", cache_key));
    
    log::trace!("Checking full image cache for key: {}", cache_key);
    
    if cache_file.exists() {
        log::debug!("Found cached full image: {}", cache_file.display());
        match fs::read(&cache_file) {
            Ok(bytes) => {
                log::debug!("Successfully read cached full image, size: {} bytes", bytes.len());
                Some(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes))
            },
            Err(e) => {
                log::warn!("Failed to read cached full image {}: {}", cache_file.display(), e);
                None
            }
        }
    } else {
        log::trace!("No cached full image found for key: {}", cache_key);
        None
    }
}

// Function to save full image to disk cache
pub fn save_preview_to_cache(cache_key: &str, image_bytes: &[u8]) -> io::Result<()> {
    let cache_dir = get_full_image_cache_dir();
    let cache_file = cache_dir.join(format!("{}.jpg", cache_key));

    log::debug!("Saving preview to cache: {} ({} bytes)", cache_file.display(), image_bytes.len());

    match fs::write(&cache_file, image_bytes) {
        Ok(_) => {
            log::trace!("Successfully saved preview to cache: {}", cache_file.display());
            Ok(())
        },
        Err(e) => {
            log::error!("Failed to save preview to cache {}: {}", cache_file.display(), e);
            Err(e)
        }
    }
}

// Function to check if a thumbnail exists in the cache
pub fn thumbnail_exists_in_cache(cache_key: &str) -> bool {
    let cache_dir = get_cache_dir();
    let cache_file = cache_dir.join(format!("{}.jpg", cache_key));
    cache_file.exists()
}