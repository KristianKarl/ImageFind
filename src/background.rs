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
            let mut interrupted = false;
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
                        interrupted = true;
                        break; // Pause if user becomes active
                    }
                    if let Ok(file_path) = file_path_res {
                        let file_path = file_path.strip_suffix(".xmp").unwrap_or(&file_path).to_string();
                        let cache_key = crate::processing::cache::generate_cache_key(&file_path);
                        if !crate::processing::cache::thumbnail_exists_in_cache(&cache_key) {
                            log::info!("Background worker: generating thumbnail for {}", file_path);
                            let result = crate::processing::image::generate_thumbnail(&file_path);
                            if result.is_none() {
                                log::error!("Failed to generate thumbnail for {}", file_path);
                            } else {
                                log::debug!("Successfully generated thumbnail for {}", file_path);
                            }
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            }
            // Only set the flag if the scan was not interrupted
            if !interrupted {
                exhausted_flag.store(true, Ordering::SeqCst);
                return;
            }
            // Sleep before next full scan
            thread::sleep(Duration::from_secs(10));
        }
    });
}

// Example: start a second worker when thumbnails are done
pub fn start_background_preview_worker() {
    let user_active = crate::routes::USER_REQUEST_ACTIVE.clone();
    let exhausted_flag = THUMBNAIL_WORKER_EXHAUSTED.clone();
    std::thread::spawn(move || {
        log::info!("Background preview worker started");
        loop {
            // Wait until thumbnail worker is exhausted
            if !exhausted_flag.load(Ordering::SeqCst) {
                log::trace!("Preview worker waiting for thumbnail worker to finish...");
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
            // Pause if user requests are active
            if user_active.load(Ordering::SeqCst) {
                log::trace!("Preview worker pausing due to user activity");
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
            log::debug!("Preview worker starting full-size preview scan");
            let args = get_cli_args();
            let conn = match rusqlite::Connection::open(&args.db_path) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Preview worker: failed to open DB: {}", e);
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    continue;
                }
            };
            let mut stmt = match conn.prepare("SELECT path FROM file") {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Preview worker: failed to prepare statement: {}", e);
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    continue;
                }
            };
            let file_iter = stmt.query_map([], |row| row.get::<_, String>(0));
            if let Ok(iter) = file_iter {
                for file_path_res in iter {
                    if user_active.load(Ordering::SeqCst) {
                        log::trace!("Preview worker interrupted by user activity");
                        break;
                    }
                    if let Ok(file_path) = file_path_res {
                        let file_path = file_path.strip_suffix(".xmp").unwrap_or(&file_path);
                        let cache_key = crate::processing::cache::generate_cache_key(file_path);
                        // Only generate if not already cached
                        if crate::processing::cache::get_cached_preview(&cache_key).is_none() {
                            log::info!("Background worker: generating preview for {}", file_path);
                            let result = crate::processing::image::generate_preview(&file_path);
                            if result.is_none() {
                                log::error!("Failed to generate preview for {}", file_path);
                            } else {
                                log::debug!("Successfully generated preview for {}", file_path);
                            }
                            thread::sleep(Duration::from_millis(100));
                        } else {
                            log::trace!("Preview already cached for {}", file_path);
                        }
                    }
                }
                log::warn!("Preview worker: Done with full scan.");
                return;
            } else {
                log::warn!("Preview worker: failed to query file paths");
            }
            log::debug!("Preview worker sleeping before next scan");
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
}

