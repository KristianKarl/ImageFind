use actix_web::{web, HttpResponse, Responder};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use urlencoding;
use crate::cli::get_cli_args;
use base64::{Engine as _, engine::{general_purpose}};

use crate::processing::{
    image::{generate_thumbnail, generate_preview},
};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use once_cell::sync::Lazy;

#[derive(Deserialize)]
pub struct IndexQuery {
    pub search: Option<String>,
}

// Struct to hold each result row
#[derive(Serialize)]
pub struct SearchResult {
    pub file_path: String,
    pub value: String,
    pub thumbnail_base64: Option<String>,
}

// Global flag to indicate if user requests are active
pub static USER_REQUEST_ACTIVE: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

// Helper to wrap user request handlers and set/unset the busy flag
async fn with_user_activity<F, Fut, R>(f: F) -> R
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = R>,
{
    USER_REQUEST_ACTIVE.store(true, Ordering::SeqCst);
    let result = f().await;
    USER_REQUEST_ACTIVE.store(false, Ordering::SeqCst);
    result
}

// Function to escape HTML characters
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

// Function to highlight search terms in text
fn highlight_search_terms(text: &str, search_term: &str) -> String {
    if search_term.is_empty() {
        return html_escape(text);
    }
    
    // Escape the original text first
    let mut escaped_text = html_escape(text);
    
    // Parse search terms using the same logic as the search query
    let terms_to_highlight = parse_search_terms(search_term);
    
    // Highlight each term
    for term in terms_to_highlight {
        if !term.is_empty() {
            let term_lower = term.to_lowercase();
            let mut result = String::new();
            let mut remaining = escaped_text.as_str();
            
            while let Some(pos) = remaining.to_lowercase().find(&term_lower) {
                // Add text before the match
                result.push_str(&remaining[..pos]);
                
                // Add highlighted match
                let match_text = &remaining[pos..pos + term.len()];
                result.push_str(&format!("<mark style=\"background-color: lightgreen; padding: 1px 2px; border-radius: 2px;\">{}</mark>", match_text));
                
                // Move to text after the match
                remaining = &remaining[pos + term.len()..];
            }
            
            // Add remaining text
            result.push_str(remaining);
            escaped_text = result;
        }
    }
    
    escaped_text
}

// Function to parse search query and handle cross-field search
fn parse_search_query(search_term: &str) -> (String, Vec<String>) {
    if search_term.trim().is_empty() {
        return ("WHERE key_value.value LIKE ?1".to_string(), vec![format!("%{}%", search_term)]);
    }
    
    // Parse search terms, handling quoted strings
    let terms = parse_search_terms(search_term);
    
    if terms.is_empty() {
        return ("WHERE key_value.value LIKE ?1".to_string(), vec![format!("%{}%", search_term)]);
    }
    
    if terms.len() == 1 {
        // Single term, use original single-term logic
        return ("WHERE key_value.value LIKE ?1".to_string(), vec![format!("%{}%", terms[0])]);
    }
    
    // Build WHERE clause that searches across all metadata fields for each file
    // Each term must be found in at least one metadata field of the same file
    let mut where_conditions = Vec::new();
    let mut parameters = Vec::new();
    
    for (i, term) in terms.iter().enumerate() {
        let param_num = i + 1;
        where_conditions.push(format!(
            "file.id IN (SELECT DISTINCT kv{}.file_id FROM key_value kv{} WHERE kv{}.value LIKE ?{})",
            param_num, param_num, param_num, param_num
        ));
        parameters.push(format!("%{}%", term.trim()));
    }
    
    let where_clause = format!("WHERE {}", where_conditions.join(" AND "));
    (where_clause, parameters)
}

// Function to parse search terms, handling quoted strings and whitespace splitting
fn parse_search_terms(input: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current_term = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();
    
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    // End of quoted string
                    if !current_term.trim().is_empty() {
                        terms.push(current_term.trim().to_string());
                        current_term.clear();
                    }
                    in_quotes = false;
                } else {
                    // Start of quoted string
                    // If we have accumulated non-quoted content, save it first
                    if !current_term.trim().is_empty() {
                        terms.push(current_term.trim().to_string());
                        current_term.clear();
                    }
                    in_quotes = true;
                }
            }
            ' ' | '\t' | '\n' | '\r' => {
                if in_quotes {
                    // Inside quotes, preserve whitespace
                    current_term.push(ch);
                } else {
                    // Outside quotes, whitespace is a separator
                    if !current_term.trim().is_empty() {
                        terms.push(current_term.trim().to_string());
                        current_term.clear();
                    }
                }
            }
            _ => {
                current_term.push(ch);
            }
        }
    }
    
    // Add any remaining term
    if !current_term.trim().is_empty() {
        terms.push(current_term.trim().to_string());
    }
    
    // Filter out empty terms
    terms.into_iter().filter(|t| !t.is_empty()).collect()
}

pub async fn index(query: web::Query<IndexQuery>) -> HttpResponse {
    log::debug!("Index endpoint called with query: {:?}", query.search);
    
    // If there's a search query, show search results
    if let Some(search_term) = &query.search {
        if !search_term.is_empty() {
            log::info!("Redirecting to search page for term: {}", search_term);
            return search_page(query).await;
        }
    }
    
    log::debug!("Serving index page");
    let html = include_str!("../templates/index.html");

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

pub async fn health_check() -> impl Responder {
    log::trace!("Health check endpoint called");
    HttpResponse::Ok().body("Healthy")
}

pub async fn api_search(query: web::Query<IndexQuery>) -> impl Responder {
    let search_term = query.search.as_deref().unwrap_or("");
    log::info!("API search called with term: '{}'", search_term);
    
    let (where_clause, parameters) = parse_search_query(search_term);
    log::debug!("Generated SQL where clause: {}", where_clause);
    log::debug!("Parameters: {:?}", parameters);

    let args = get_cli_args();
    let conn = match Connection::open(&args.db_path) {
        Ok(c) => {
            log::debug!("Successfully opened database: {}", args.db_path);
            c
        },
        Err(e) => {
            log::error!("Failed to open database {}: {}", args.db_path, e);
            return HttpResponse::InternalServerError().body(format!("DB open error: {}", e));
        },
    };

    let mut stmt = match conn.prepare(
        &format!("SELECT file.path, key_value.value \
         FROM key_value \
         JOIN file ON key_value.file_id = file.id \
         {} \
         ORDER BY file.path ASC", where_clause)
    ) {
        Ok(s) => s,
        Err(e) => {
            log::error!("SQL preparation error: {}", e);
            return HttpResponse::InternalServerError().body(format!("Prepare error: {}", e));
        },
    };

    let rows = stmt
        .query_map(rusqlite::params_from_iter(parameters.iter()), |row| {
            let file_path: String = row.get(0)?;
            let value: String = row.get(1)?;
            // Remove ".xmp" suffix if present
            let file_path = file_path.strip_suffix(".xmp").unwrap_or(&file_path).to_string();
            
            log::trace!("Processing result: {}", file_path);
            // Generate thumbnail for the image
            let thumbnail_base64 = generate_thumbnail(&file_path);
            
            Ok(SearchResult { file_path, value, thumbnail_base64 })
        });

    let mut results = Vec::new();
    match rows {
        Ok(mapped) => {
            for row in mapped {
                match row {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        log::error!("Row processing error: {}", e);
                        return HttpResponse::InternalServerError().body(format!("Row error: {}", e));
                    },
                }
            }
        }
        Err(e) => {
            log::error!("Query execution error: {}", e);
            return HttpResponse::InternalServerError().body(format!("Query error: {}", e));
        },
    }

    log::info!("API search completed, found {} results", results.len());

    // Return as JSON
    match serde_json::to_string(&results) {
        Ok(json) => HttpResponse::Ok().content_type("application/json").body(json),
        Err(e) => {
            log::error!("JSON serialization error: {}", e);
            HttpResponse::InternalServerError().body(format!("Serialization error: {}", e))
        },
    }
}

pub async fn search_page(query: web::Query<IndexQuery>) -> HttpResponse {
    let search_term = query.search.as_deref().unwrap_or("");
    log::info!("Search page called with term: '{}'", search_term);
    
    let (where_clause, parameters) = parse_search_query(search_term);
    log::debug!("Generated SQL where clause: {}", where_clause);

    let args = get_cli_args();
    let conn = match Connection::open(&args.db_path) {
        Ok(c) => {
            log::debug!("Successfully opened database for search: {}", args.db_path);
            c
        },
        Err(e) => {
            log::error!("Failed to open database {}: {}", args.db_path, e);
            return HttpResponse::InternalServerError().body(format!("DB open error: {}", e));
        },
    };

    // First, get the matching file IDs
    let mut stmt = match conn.prepare(
        &format!("SELECT DISTINCT file.id, file.path \
         FROM key_value \
         JOIN file ON key_value.file_id = file.id \
         {} \
         ORDER BY file.path ASC", where_clause)
    ) {
        Ok(s) => s,
        Err(e) => {
            log::error!("SQL preparation error for search: {}", e);
            return HttpResponse::InternalServerError().body(format!("Prepare error: {}", e));
        },
    };

    let file_rows = stmt
        .query_map(rusqlite::params_from_iter(parameters.iter()), |row| {
            let file_id: i64 = row.get(0)?;
            let file_path: String = row.get(1)?;
            Ok((file_id, file_path))
        });

    let mut file_results = Vec::new();
    match file_rows {
        Ok(mapped) => {
            for row in mapped {
                match row {
                    Ok((file_id, file_path)) => {
                        // Remove ".xmp" suffix if present
                        let clean_path = file_path.strip_suffix(".xmp").unwrap_or(&file_path).to_string();
                        file_results.push((file_id, clean_path));
                    },
                    Err(e) => {
                        log::error!("Row processing error in search: {}", e);
                        return HttpResponse::InternalServerError().body(format!("Row error: {}", e));
                    },
                }
            }
        }
        Err(e) => {
            log::error!("Query execution error in search: {}", e);
            return HttpResponse::InternalServerError().body(format!("Query error: {}", e));
        },
    }

    log::info!("Search page found {} unique files", file_results.len());

    // Now get all metadata for each file
    let mut results_with_metadata = Vec::new();
    for (file_id, file_path) in file_results {
        // Get all metadata values for this file
        let mut metadata_stmt = match conn.prepare(
            "SELECT value FROM key_value WHERE file_id = ?1 ORDER BY key"
        ) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to prepare metadata query: {}", e);
                continue;
            }
        };

        let metadata_rows = metadata_stmt.query_map(rusqlite::params![file_id], |row| {
            let value: String = row.get(0)?;
            Ok(value)
        });

        let mut all_metadata = Vec::new();
        match metadata_rows {
            Ok(mapped) => {
                for row in mapped {
                    match row {
                        Ok(value) => {
                            // Skip empty values and very long values that might be binary data
                            if !value.trim().is_empty() && value.len() < 500 {
                                all_metadata.push(value);
                            }
                        },
                        Err(e) => {
                            log::warn!("Error reading metadata value for file_id {}: {}", file_id, e);
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("Metadata query error for file_id {}: {}", file_id, e);
            }
        }

        results_with_metadata.push((file_path, all_metadata));
    }

    // Generate HTML efficiently
    let mut html_parts = Vec::new();
    
    // HTML header with search term
    let mut header_html = include_str!("../templates/search_header.html").to_string();
    // Replace the placeholder in the search input with the actual search term
    let escaped_search_term = html_escape(search_term);
    header_html = header_html.replace(
        r#"<input type="text" name="search" class="search-input" placeholder="Search images..." value="" />"#,
        &format!(r#"<input type="text" name="search" class="search-input" placeholder="Search images..." value="{}" />"#, escaped_search_term)
    );
    html_parts.push(header_html);

    // Generate result items with placeholder thumbnails and all metadata
    for (file_path, all_metadata) in results_with_metadata {
        let escaped_file_path = html_escape(&file_path);
        
        // Create highlighted metadata values
        let mut highlighted_metadata = Vec::new();
        for metadata_value in &all_metadata {
            let highlighted_value = highlight_search_terms(metadata_value, search_term);
            highlighted_metadata.push(highlighted_value);
        }
        
        // Join all metadata values with line breaks
        let combined_metadata = highlighted_metadata.join("<br>");
        
        // Escape for JavaScript (replace single quotes)
        let js_safe_path = file_path.replace('\'', "\\'");
        let js_safe_value = all_metadata.join(" ").replace('\'', "\\'").replace('\n', "\\n").replace('\r', "");
        let encoded_path = urlencoding::encode(&file_path);
        
        let item_html = format!(r#"
        <div class="result-item" data-file-path="{}">
            <div>
                <div class="thumbnail-container">
                    <div class="thumbnail-placeholder">
                        <div class="loading-spinner"></div>
                        <div class="loading-text">Loading...</div>
                    </div>
                    <img class="thumbnail" style="display: none;" alt="{}" onclick="openModal('/image/{}', '{}')" />
                </div>
            </div>
            <div class="file-path">{}</div>
            <div class="value-text">{}</div>
        </div>
"#, encoded_path, escaped_file_path, js_safe_path, js_safe_value, escaped_file_path, combined_metadata);
        html_parts.push(item_html);
    }

    // HTML footer
    html_parts.push(include_str!("../templates/search_footer.html").to_string());

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html_parts.join(""))
}

// Add a new endpoint for fetching individual thumbnails
pub async fn get_thumbnail(path: web::Path<String>) -> impl Responder {
    with_user_activity(|| async move {
        let image_path = path.into_inner();
        log::debug!("Thumbnail request for: {}", image_path);
        
        // Decode URL-encoded path
        let decoded_path = urlencoding::decode(&image_path).unwrap_or_else(|_| image_path.clone().into());
        let clean_path = decoded_path.to_string();
        
        // Security check - prevent path traversal
        if clean_path.contains("..") {
            log::warn!("Path traversal attempt blocked: {}", clean_path);
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Invalid path: path traversal not allowed"
            }));
        }
        
        // Remove ".xmp" suffix if present
        let file_path = clean_path.strip_suffix(".xmp").unwrap_or(&clean_path).to_string();
        log::trace!("Processing thumbnail for cleaned path: {}", file_path);
        
        // Generate thumbnail in a blocking task
        let thumbnail_result = tokio::task::spawn_blocking(move || {
            generate_thumbnail(&file_path)
        }).await;
        
        match thumbnail_result {
            Ok(Some(thumbnail_base64)) => {
                log::debug!("Successfully generated thumbnail for: {}", clean_path);
                HttpResponse::Ok().json(serde_json::json!({
                    "thumbnail": thumbnail_base64,
                    "file_path": clean_path
                }))
            }
            Ok(None) => {
                log::warn!("Could not generate thumbnail for: {}", clean_path);
                HttpResponse::Ok().json(serde_json::json!({
                    "thumbnail": null,
                    "file_path": clean_path
                }))
            }
            Err(e) => {
                log::error!("Thumbnail generation task failed for {}: {:?}", clean_path, e);
                HttpResponse::InternalServerError().json(serde_json::json!({
                    "error": "Failed to generate thumbnail",
                    "file_path": clean_path
                }))
            }
        }
    }).await
}

pub async fn get_preview(path: web::Path<String>) -> impl Responder {
    with_user_activity(|| async move {
        let image_path = path.into_inner();
        log::info!("Image serve request for: {}", image_path);
        
        // Decode URL-encoded path
        let decoded_path = urlencoding::decode(&image_path).unwrap_or_else(|_| image_path.clone().into());
        let clean_path = decoded_path.to_string();
        log::debug!("Decoded path: {}", clean_path);
        
        let safe_path = Path::new(&clean_path);
        
        // Security check - prevent path traversal but allow absolute paths in safe directories
        if clean_path.contains("..") {
            log::warn!("Path traversal attempt blocked for image: {}", clean_path);
            return HttpResponse::BadRequest().body("Invalid path: path traversal not allowed");
        }
        
        // Additional security: ensure the path exists and is a file
        if !safe_path.exists() {
            log::warn!("Image file not found: {}", clean_path);
            return HttpResponse::NotFound().body("Image file not found");
        }
        
        if !safe_path.is_file() {
            log::warn!("Path is not a file: {}", clean_path);
            return HttpResponse::BadRequest().body("Path is not a file");
        }

        let image_path_for_closure = clean_path.clone();
        
        // Generate preview in a blocking task
        let preview_result = tokio::task::spawn_blocking(move || {
            generate_preview(&image_path_for_closure)
        }).await;
        
        match preview_result {
            Ok(Some(preview_base64)) => {
                log::debug!("Successfully generated preview for: {}", clean_path);
                // Decode base64 to bytes before returning as image/jpeg
                match general_purpose::STANDARD.decode(&preview_base64) {
                    Ok(jpeg_bytes) => {
                        HttpResponse::Ok()
                            .content_type("image/jpeg")
                            .body(jpeg_bytes)
                    }
                    Err(e) => {
                        log::error!("Failed to decode base64 preview for {}: {:?}", clean_path, e);
                        HttpResponse::InternalServerError().body("Failed to decode preview image")
                    }
                }
            }
            Ok(None) => {
                log::warn!("Could not generate preview for: {}", clean_path);
                HttpResponse::Ok().json(serde_json::json!({
                    "preview": null,
                    "file_path": clean_path
                }))
            }
            Err(e) => {
                log::error!("Preview generation task failed for {}: {:?}", clean_path, e);
                HttpResponse::InternalServerError().json(serde_json::json!({
                    "error": "Failed to generate preview",
                    "file_path": clean_path
                }))
            }
        }

    }).await
}

// Add this function near the other endpoints
pub async fn serve_video(path: web::Path<String>) -> impl Responder {
    with_user_activity(|| async move {
        let video_path = path.into_inner();
        log::info!("Video preview request for: {}", video_path);

        // Decode URL-encoded path
        let decoded_path = urlencoding::decode(&video_path).unwrap_or_else(|_| video_path.clone().into());
        let clean_path = decoded_path.to_string();

        // Security check - prevent path traversal
        if clean_path.contains("..") {
            log::warn!("Path traversal attempt blocked for video: {}", clean_path);
            return HttpResponse::BadRequest().body("Invalid path: path traversal not allowed");
        }

        // Get video preview cache directory from CLI args
        let args = get_cli_args();
        let preview_cache_dir = std::path::Path::new(&args.video_preview_cache);

        // Build the _480p preview filename (basename + _480p.mp4)
        let orig_path = std::path::Path::new(&clean_path);
        let stem = orig_path.file_stem();
        let ext = orig_path.extension();

        let transcoded_file_path = if let (Some(stem), Some(_ext)) = (stem, ext) {
            let mut transcoded_file_name = stem.to_os_string();
            transcoded_file_name.push("_480p.mp4");
            preview_cache_dir.join(transcoded_file_name)
        } else {
            log::warn!("Could not construct _480p filename for: {}", clean_path);
            return HttpResponse::NotFound().body("Invalid video path");
        };

        log::info!("Looking for transcoded video file in preview cache: {}", transcoded_file_path.display());

        if !transcoded_file_path.exists() {
            log::warn!("Transcoded video file not found: {}", transcoded_file_path.display());
            return HttpResponse::NotFound().body("Transcoded video file not found");
        }

        match std::fs::File::open(&transcoded_file_path) {
            Ok(mut file) => {
                let mut buf = Vec::new();
                if std::io::Read::read_to_end(&mut file, &mut buf).is_ok() {
                    return HttpResponse::Ok()
                        .content_type("video/mp4")
                        .append_header(("Cache-Control", "public, max-age=3600"))
                        .body(buf);
                }
            }
            Err(e) => {
                log::error!("Failed to open transcoded video file: {}", e);
            }
        }
        HttpResponse::InternalServerError().body("Failed to read transcoded video")
    }).await
}