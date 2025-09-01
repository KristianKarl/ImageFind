use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::Reader;
use rayon::prelude::*;
use rusqlite::{params, Connection, Result};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::sync::{Arc, Mutex};
use walkdir::WalkDir;
use xxhash_rust::xxh3::xxh3_64;

use crate::cli::get_cli_args;

/// Scans the given directory for XMP sidecar files and imports their metadata into the SQLite database.
pub fn scan_and_import_sidecars() -> Result<()> {
    let args = get_cli_args();
    let scan_dir = args.scan_dir.clone();
    let db_path = args.db_path.clone();
    
    log::info!("Starting sidecar scan - Directory: {}, Database: {}", scan_dir, db_path);
    
    let conn = Arc::new(Mutex::new(Connection::open(&db_path)?));
    log::debug!("Successfully opened database connection");

    {
        let conn = conn.lock().unwrap();
        log::debug!("Creating database tables if they don't exist");
        
        // Table file contains all sidecar files with their path and hash
        conn.execute(
            "CREATE TABLE IF NOT EXISTS file (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL,
                hash BIGINT NOT NULL,
                UNIQUE(path, hash)
            )",
            [],
        )?;
        log::trace!("File table created/verified");
        
        // Table key_value contains all key-value pairs extracted from the XMP files
        conn.execute(
            "CREATE TABLE IF NOT EXISTS key_value (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                FOREIGN KEY(file_id) REFERENCES file(id)
            )",
            [],
        )?;
        log::trace!("Key_value table created/verified");
    }

    log::info!("Scanning directory for XMP files: {}", scan_dir);
    
    // Collect all XMP file paths first
    let xmp_files: Vec<_> = WalkDir::new(&scan_dir)
        .into_iter()
        .filter_map(|e| {
            match e {
                Ok(entry) => Some(entry),
                Err(err) => {
                    log::warn!("Error accessing directory entry: {}", err);
                    None
                }
            }
        })
        .filter(|entry| {
            let path = entry.path();
            let is_xmp = path.is_file()
                && path
                    .extension()
                    .map(|ext| ext.eq_ignore_ascii_case("xmp"))
                    .unwrap_or(false);
            
            if is_xmp {
                log::trace!("Found XMP file: {}", path.display());
            }
            is_xmp
        })
        .map(|entry| entry.path().to_owned())
        .collect();

    log::info!("Found {} XMP files to process", xmp_files.len());

    if xmp_files.is_empty() {
        log::warn!("No XMP files found in directory: {}", scan_dir);
        return Ok(());
    }

    let processed_count = Arc::new(Mutex::new(0));
    let error_count = Arc::new(Mutex::new(0));

    // Process each XMP file in parallel
    xmp_files.par_iter().for_each(|path| {
        if let Some(path_str) = path.to_str() {
            log::debug!("Processing XMP file: {}", path_str);

            match extract_key_value(path_str) {
                Some(kv) => {
                    log::trace!("Extracted {} key-value pairs from {}", kv.len(), path_str);

                    // Get hash sum using xxhash for file
                    match std::fs::File::open(path) {
                        Ok(mut file) => {
                            let mut buffer = Vec::new();
                            match file.read_to_end(&mut buffer) {
                                Ok(bytes_read) => {
                                    log::trace!("Read {} bytes from {}", bytes_read, path_str);
                                    let hash = xxh3_64(&buffer) as i64;
                                    log::trace!("Generated hash {} for {}", hash, path_str);

                                    // Acquire the database lock only for the DB operations
                                    let conn_guard = conn.lock();
                                    match conn_guard {
                                        Ok(ref conn) => {
                                            // Check if path exists in table file
                                            match conn.prepare("SELECT id, hash FROM file WHERE path = ?1") {
                                                Ok(mut stmt) => {
                                                    match stmt.query(params![path_str]) {
                                                        Ok(mut rows) => {
                                                            match rows.next() {
                                                                Ok(Some(row)) => {
                                                                    let file_id: i64 = row.get(0).unwrap();
                                                                    let old_hash: i64 = row.get(1).unwrap();
                                                                    if old_hash == hash {
                                                                        // Already up to date, skip
                                                                        log::trace!("File {} is up to date (hash {})", path_str, hash);
                                                                        return;
                                                                    } else {
                                                                        log::info!("File {} has changed, updating (old hash: {}, new hash: {})", path_str, old_hash, hash);
                                                                        // Update hash
                                                                        if let Err(e) = conn.execute(
                                                                            "UPDATE file SET hash = ?1 WHERE id = ?2",
                                                                            params![hash, file_id],
                                                                        ) {
                                                                            log::error!("Failed to update hash for {}: {}", path_str, e);
                                                                            let mut error_count = error_count.lock().unwrap();
                                                                            *error_count += 1;
                                                                            return;
                                                                        }

                                                                        // Delete all old key-values
                                                                        if let Err(e) = conn.execute("DELETE FROM key_value WHERE file_id = ?1", params![file_id]) {
                                                                            log::error!("Failed to delete old key-values for {}: {}", path_str, e);
                                                                            let mut error_count = error_count.lock().unwrap();
                                                                            *error_count += 1;
                                                                            return;
                                                                        }

                                                                        insert_key_values(conn, file_id, &kv);
                                                                        log::info!("Updated file: {} [{}]", path_str, hash);
                                                                    }
                                                                }
                                                                Ok(None) => {
                                                                    log::info!("New file detected: {}", path_str);
                                                                    // Insert new row into table file
                                                                    if let Err(e) = conn.execute(
                                                                        "INSERT INTO file (path, hash) VALUES (?1, ?2)",
                                                                        params![path_str, hash],
                                                                    ) {
                                                                        log::error!("Failed to insert new file {}: {}", path_str, e);
                                                                        let mut error_count = error_count.lock().unwrap();
                                                                        *error_count += 1;
                                                                        return;
                                                                    }
                                                                    let file_id: i64 = conn.last_insert_rowid();

                                                                    insert_key_values(conn, file_id, &kv);
                                                                    log::info!("Inserted file: {} [{}]", path_str, hash);
                                                                }
                                                                Err(e) => {
                                                                    log::error!("Database query error for {}: {}", path_str, e);
                                                                    let mut error_count = error_count.lock().unwrap();
                                                                    *error_count += 1;
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            log::error!("Failed to execute query for {}: {}", path_str, e);
                                                            let mut error_count = error_count.lock().unwrap();
                                                            *error_count += 1;
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    log::error!("Failed to prepare statement for {}: {}", path_str, e);
                                                    let mut error_count = error_count.lock().unwrap();
                                                    *error_count += 1;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            log::error!("Failed to acquire database lock for {}: {:?}", path_str, e);
                                            let mut error_count = error_count.lock().unwrap();
                                            *error_count += 1;
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to read file {}: {}", path_str, e);
                                    let mut error_count = error_count.lock().unwrap();
                                    *error_count += 1;
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to open file {}: {}", path_str, e);
                            let mut error_count = error_count.lock().unwrap();
                            *error_count += 1;
                        }
                    }
                }
                None => {
                    log::warn!("Failed to extract key-value pairs from {}", path_str);
                    let mut error_count = error_count.lock().unwrap();
                    *error_count += 1;
                }
            }

            // Update processed count
            let mut processed_count = processed_count.lock().unwrap();
            *processed_count += 1;

            // Log progress every 100 files
            if *processed_count % 100 == 0 {
                log::info!("Processed {} files so far", *processed_count);
            }
        } else {
            log::error!("Invalid UTF-8 in file path: {:?}", path);
            let mut error_count = error_count.lock().unwrap();
            *error_count += 1;
        }
    });
    
    let final_processed = *processed_count.lock().unwrap();
    let final_errors = *error_count.lock().unwrap();
    
    log::info!("Sidecar scan completed - Processed: {} files, Errors: {} files", final_processed, final_errors);
    
    if final_errors > 0 {
        log::warn!("Scan completed with {} errors", final_errors);
    } else {
        log::info!("Scan completed successfully with no errors");
    }
    
    Ok(())
}

fn insert_key_values(
    conn: &std::sync::MutexGuard<'_, Connection>,
    file_id: i64,
    kv: &HashMap<String, String>,
) {
    log::trace!("Inserting {} key-value pairs for file_id {}", kv.len(), file_id);
    
    // Insert new key-values, start with xmp:ModifyDate
    let modify_date = kv
        .iter()
        .find(|(k, _)| k.ends_with("xmp:ModifyDate"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("");

    log::trace!("Inserting xmp:ModifyDate: {}", modify_date);
    if let Err(e) = conn.execute(
        "INSERT INTO key_value (file_id, key, value) VALUES (?1, ?2, ?3)",
        params![file_id, "xmp:ModifyDate", modify_date],
    ) {
        log::error!("Failed to insert xmp:ModifyDate for file_id {}: {}", file_id, e);
        return;
    }

    let mut inserted_count = 1; // Count the xmp:ModifyDate we just inserted
    
    // Insert the rest of the key-values
    for (key, value) in kv {
        if key.contains("digiKam:TagsList") || key == "dc:title/rdf:Alt" {
            log::trace!("Inserting key: {} = {}", key, value);
            if let Err(e) = conn.execute(
                "INSERT INTO key_value (file_id, key, value) VALUES (?1, ?2, ?3)",
                params![file_id, key, value],
            ) {
                log::error!("Failed to insert key-value {}='{}' for file_id {}: {}", key, value, file_id, e);
            } else {
                inserted_count += 1;
            }
        }
    }
    
    log::debug!("Successfully inserted {} key-value pairs for file_id {}", inserted_count, file_id);
}

fn extract_key_value(path: &str) -> Option<HashMap<String, String>> {
    log::trace!("Extracting key-value pairs from XMP file: {}", path);
    
    let xml = match fs::read_to_string(path) {
        Ok(content) => {
            log::trace!("Successfully read XMP file, size: {} bytes", content.len());
            content
        }
        Err(e) => {
            log::error!("Failed to read XMP file {}: {}", path, e);
            return None;
        }
    };
    
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);

    let mut buf: Vec<u8> = Vec::new();
    let mut kv = HashMap::new();
    let mut tag_stack: Vec<String> = Vec::new();
    let mut in_tagslist = false;
    let mut in_seq = false;
    let mut in_title = false;
    let mut in_alt = false;
    let mut tagslist_items: Vec<String> = Vec::new();
    let mut title_items: Vec<String> = Vec::new();

    let mut element_count = 0;
    let mut text_count = 0;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                element_count += 1;
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                tag_stack.push(tag.clone());
                
                if tag.ends_with("digiKam:TagsList") {
                    in_tagslist = true;
                    log::trace!("Entering digiKam:TagsList section");
                }
                if in_tagslist && tag.ends_with("rdf:Seq") {
                    in_seq = true;
                    log::trace!("Entering rdf:Seq section within TagsList");
                }
                if tag.ends_with("dc:title") {
                    in_title = true;
                    log::trace!("Entering dc:title section");
                }
                if in_title && tag.ends_with("rdf:Alt") {
                    in_alt = true;
                    log::trace!("Entering rdf:Alt section within title");
                }
                
                for attr in e.attributes().flatten() {
                    let key = format!(
                        "{}:{}",
                        tag_stack.join("/"),
                        String::from_utf8_lossy(attr.key.as_ref())
                    );
                    let value = attr.unescape_value().unwrap_or_default().to_string();
                    log::trace!("Found attribute: {} = {}", key, value);
                    kv.insert(key, value);
                }
            }
            Ok(Event::Text(e)) => {
                text_count += 1;
                let lossy = String::from_utf8_lossy(e.as_ref()).into_owned();
                let text = unescape(&lossy).unwrap_or_else(|_| lossy.clone().into());
                if !tag_stack.is_empty() && !text.trim().is_empty() {
                    let key = tag_stack.join("/");
                    // Collect rdf:li items under digiKam:TagsList/rdf:Seq
                    if in_tagslist
                        && in_seq
                        && tag_stack
                            .last()
                            .map(|t| t.ends_with("rdf:li"))
                            .unwrap_or(false)
                    {
                        log::trace!("Found TagsList item: {}", text);
                        tagslist_items.push(text.to_string());
                    // Collect rdf:li items under dc:title/rdf:Alt
                    } else if in_title
                        && in_alt
                        && tag_stack
                            .last()
                            .map(|t| t.ends_with("rdf:li"))
                            .unwrap_or(false)
                    {
                        log::trace!("Found title item: {}", text);
                        title_items.push(text.to_string());
                    } else {
                        log::trace!("Found text content: {} = {}", key, text);
                        kv.insert(key, text.to_string());
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if in_seq && tag.ends_with("rdf:Seq") {
                    in_seq = false;
                    log::trace!("Exiting rdf:Seq section");
                }
                if in_tagslist && tag.ends_with("digiKam:TagsList") {
                    in_tagslist = false;
                    log::trace!("Exiting digiKam:TagsList section");
                    // Store all collected tagslist_items as a single value (joined by semicolon)
                    if !tagslist_items.is_empty() {
                        let combined_tags = tagslist_items.join(";");
                        log::debug!("Collected {} TagsList items: {}", tagslist_items.len(), combined_tags);
                        kv.insert(
                            "digiKam:TagsList/rdf:Seq".to_string(),
                            combined_tags,
                        );
                        tagslist_items.clear();
                    }
                }
                if in_alt && tag.ends_with("rdf:Alt") {
                    in_alt = false;
                    log::trace!("Exiting rdf:Alt section");
                }
                if in_title && tag.ends_with("dc:title") {
                    in_title = false;
                    log::trace!("Exiting dc:title section");
                    // Store all collected title_items as a single value (joined by semicolon)
                    if !title_items.is_empty() {
                        let combined_titles = title_items.join(";");
                        log::debug!("Collected {} title items: {}", title_items.len(), combined_titles);
                        kv.insert("dc:title/rdf:Alt".to_string(), combined_titles);
                        title_items.clear();
                    }
                }
                tag_stack.pop();
            }
            Ok(Event::Eof) => {
                log::trace!("Reached end of XML file");
                break;
            }
            Err(e) => {
                log::error!("XML parsing error in {}: {}", path, e);
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    
    log::debug!("XMP parsing completed for {} - Elements: {}, Text nodes: {}, Key-value pairs: {}", 
              path, element_count, text_count, kv.len());
    
    if kv.is_empty() {
        log::warn!("No key-value pairs extracted from {}", path);
    }
    
    Some(kv)
}
