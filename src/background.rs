use std::sync::{atomic::Ordering};
use std::thread;
use std::time::Duration;
use rusqlite::Connection;
use crate::routes::USER_REQUEST_ACTIVE;
use crate::cli::get_cli_args;
use std::sync::atomic::{AtomicBool};
use std::sync::Arc;
use once_cell::sync::Lazy;

// Add a global flag to indicate thumbnail worker is exhausted
pub static THUMBNAIL_WORKER_EXHAUSTED: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

pub fn start_background_thumbnail_worker() {
    let user_active = USER_REQUEST_ACTIVE.clone();
    let exhausted_flag = THUMBNAIL_WORKER_EXHAUSTED.clone();
    thread::spawn(move || {
        let args = get_cli_args();
        let conn = match Connection::open(&args.db_path) {
            Ok(c) => c,
            Err(e) => {
                log::error!("Background worker: failed to open DB: {}", e);
                return;
            }
        };
        loop {
            let mut all_done = true;
            // Pause if user requests are active
            if user_active.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(500));
                continue;
            }
            // Query all file paths
            let mut stmt = match conn.prepare("SELECT path FROM file") {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Background worker: failed to prepare statement: {}", e);
                    break;
                }
            };
            let file_iter = stmt.query_map([], |row| row.get::<_, String>(0));
            if let Ok(iter) = file_iter {
                for file_path_res in iter {
                    if user_active.load(Ordering::SeqCst) {
                        all_done = false;
                        break; // Pause if user becomes active
                    }
                    if let Ok(file_path) = file_path_res {
                        let file_path = file_path.strip_suffix(".xmp").unwrap_or(&file_path);
                        let cache_key = crate::processing::cache::generate_cache_key(file_path);
                        if !crate::processing::cache::thumbnail_exists_in_cache(&cache_key) {
                            log::info!("Background worker: generating thumbnail for {}", file_path);
                            let _ = crate::processing::image::generate_thumbnail(file_path);
                            all_done = false;
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            }
            // If all thumbnails are done, set the flag
            exhausted_flag.store(all_done, Ordering::SeqCst);
            // Sleep before next full scan
            thread::sleep(Duration::from_secs(10));
        }
    });
}

// Example: start a second worker when thumbnails are done
pub fn start_secondary_worker() {
    let user_active = crate::routes::USER_REQUEST_ACTIVE.clone();
    let exhausted_flag = THUMBNAIL_WORKER_EXHAUSTED.clone();
    std::thread::spawn(move || {
        loop {
            // Wait until thumbnail worker is exhausted
            if !exhausted_flag.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
            // Pause if user requests are active
            if user_active.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
            // Example secondary background work: pre-generate full-size previews for all images
            let args = get_cli_args();
            let conn = match rusqlite::Connection::open(&args.db_path) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Secondary worker: failed to open DB: {}", e);
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    continue;
                }
            };
            let mut stmt = match conn.prepare("SELECT path FROM file") {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Secondary worker: failed to prepare statement: {}", e);
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    continue;
                }
            };
            let file_iter = stmt.query_map([], |row| row.get::<_, String>(0));
            if let Ok(iter) = file_iter {
                for file_path_res in iter {
                    if user_active.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(file_path) = file_path_res {
                        let file_path = file_path.strip_suffix(".xmp").unwrap_or(&file_path);
                        let cache_key = crate::processing::cache::generate_cache_key(file_path);
                        // Only generate if not already cached
                        if crate::processing::cache::get_cached_full_image(&cache_key).is_none() {
                            log::info!("Secondary worker: generating full-size preview for {}", file_path);
                            let _ = crate::processing::image::process_preview_with_image_crate(file_path, &cache_key);
                            std::thread::sleep(std::time::Duration::from_millis(200));
                        }
                    }
                }
            }
            // Sleep before next full scan
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
}

