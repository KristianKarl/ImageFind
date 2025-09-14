use std::sync::{atomic::Ordering};
use std::thread;
use std::time::Duration;
use rusqlite::Connection;
use crate::processing::cache::{generate_cache_key, thumbnail_exists_in_cache};
use crate::processing::image::generate_thumbnail;
use crate::routes::USER_REQUEST_ACTIVE;
use crate::cli::get_cli_args;

pub fn start_background_thumbnail_worker() {
    let user_active = USER_REQUEST_ACTIVE.clone();
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
                        break; // Pause if user becomes active
                    }
                    if let Ok(file_path) = file_path_res {
                        let file_path = file_path.strip_suffix(".xmp").unwrap_or(&file_path);
                        let cache_key = generate_cache_key(file_path);
                        if !thumbnail_exists_in_cache(&cache_key) {
                            log::info!("Background worker: generating thumbnail for {}", file_path);
                            let _ = generate_thumbnail(file_path);
                            // Sleep a bit to reduce IO pressure
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            }
            // Sleep before next full scan
            thread::sleep(Duration::from_secs(10));
        }
    });
}
