use actix_web::{web, HttpResponse, Responder};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::collections::HashMap;
use urlencoding;
use crate::cli::get_cli_args;

use crate::processing::{
    cache::{generate_cache_key, get_cached_full_image, save_full_image_to_cache},
    image::{generate_external_preview, generate_thumbnail},
    raw::generate_raw_preview,
    tiff::generate_tiff_preview,
    video::generate_video_thumbnail,
};

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
    
    // Extract all search terms (handle AND logic)
    let and_parts: Vec<&str> = search_term
        .split(" AND ")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    
    // If no AND found, treat as single term
    let terms_to_highlight = if and_parts.len() <= 1 {
        vec![search_term]
    } else {
        and_parts
    };
    
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

// Function to parse search query and handle AND logic
fn parse_search_query(search_term: &str) -> (String, Vec<String>) {
    if search_term.trim().is_empty() {
        return ("WHERE key_value.value LIKE ?1".to_string(), vec![format!("%{}%", search_term)]);
    }
    
    // Split by AND (case-insensitive) while preserving quoted strings
    let and_parts: Vec<&str> = search_term
        .split(" AND ")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    
    if and_parts.len() <= 1 {
        // No AND found, use original single-term logic
        return ("WHERE key_value.value LIKE ?1".to_string(), vec![format!("%{}%", search_term)]);
    }
    
    // Build WHERE clause with multiple AND conditions
    let mut where_parts = Vec::new();
    let mut parameters = Vec::new();
    
    for (i, part) in and_parts.iter().enumerate() {
        let param_num = i + 1;
        where_parts.push(format!("key_value.value LIKE ?{}", param_num));
        parameters.push(format!("%{}%", part.trim()));
    }
    
    let where_clause = format!("WHERE {}", where_parts.join(" AND "));
    (where_clause, parameters)
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

    let mut stmt = match conn.prepare(
        &format!("SELECT file.path, key_value.value \
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

    let rows = stmt
        .query_map(rusqlite::params_from_iter(parameters.iter()), |row| {
            let file_path: String = row.get(0)?;
            let value: String = row.get(1)?;
            // Remove ".xmp" suffix if present
            let file_path = file_path.strip_suffix(".xmp").unwrap_or(&file_path).to_string();
            
            Ok((file_path, value))
        });

    let mut results = Vec::new();
    match rows {
        Ok(mapped) => {
            for row in mapped {
                match row {
                    Ok((file_path, value)) => results.push((file_path, value, None::<String>)),
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

    // Deduplicate results by file_path
    let mut seen = std::collections::HashSet::new();
    let mut unique_results = Vec::new();
    for (file_path, value, thumb) in results {
        if seen.insert(file_path.clone()) {
            unique_results.push((file_path, value, thumb));
        }
    }

    log::info!("Search page completed, found {} unique results", unique_results.len());

    // Generate HTML efficiently
    let mut html_parts = Vec::new();
    
    // HTML header
    html_parts.push(include_str!("../templates/search_header.html").to_string());

    // Generate result items with placeholder thumbnails
    for (file_path, value, _) in unique_results {
        let escaped_file_path = html_escape(&file_path);
        let highlighted_value = highlight_search_terms(&value, search_term);
        // Escape for JavaScript (replace single quotes)
        let js_safe_path = file_path.replace('\'', "\\'");
        let js_safe_value = value.replace('\'', "\\'").replace('\n', "\\n").replace('\r', "");
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
"#, encoded_path, escaped_file_path, js_safe_path, js_safe_value, escaped_file_path, highlighted_value);
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
}

pub async fn serve_image(path: web::Path<String>, query: web::Query<HashMap<String, String>>) -> impl Responder {
    let image_path = path.into_inner();
    log::info!("Image serve request for: {}", image_path);
    
    // Decode URL-encoded path
    let decoded_path = urlencoding::decode(&image_path).unwrap_or_else(|_| image_path.clone().into());
    let clean_path = decoded_path.to_string();
    log::debug!("Decoded path: {}", clean_path);
    
    // Handle absolute paths by making them relative to current directory or accepting them if they're in allowed directories
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
    
    // Generate cache key for this image
    let cache_key = generate_cache_key(&clean_path);
    log::trace!("Generated cache key: {}", cache_key);
    
    // Check if this is a cache-busting request (has timestamp parameter)
    let is_cache_bust = query.contains_key("t");
    if is_cache_bust {
        log::debug!("Cache-busting request detected for: {}", clean_path);
    }
    
    // Check cache first (skip cache if cache-busting)
    if !is_cache_bust {
        if let Some(cached_image) = get_cached_full_image(&cache_key) {
            log::debug!("Serving cached image for: {}", clean_path);
            return HttpResponse::Ok()
                .content_type("image/jpeg")
                .append_header(("Cache-Control", "public, max-age=3600"))
                .body(cached_image);
        }
        log::trace!("No cached image found for: {}", clean_path);
    }
    
    // Determine file type before the closure
    let is_video = match safe_path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => {
            let ext_lower = ext.to_lowercase();
            match ext_lower.as_str() {
                "mp4" | "avi" | "mov" | "wmv" | "flv" | 
                "webm" | "mkv" | "m4v" | "3gp" | "ogv" => true,
                _ => false,
            }
        }
        _ => false,
    };
    
    let is_raw = match safe_path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => {
            let ext_lower = ext.to_lowercase();
            match ext_lower.as_str() {
                "nef" | "cr2" | "cr3" | "arw" | "orf" | "rw2" | "raf" | "dng" | 
                "3fr" | "ari" | "bay" | "crw" | "dcr" | "erf" | "fff" | "iiq" | 
                "k25" | "kdc" | "mdc" | "mos" | "mrw" | "pef" | "ptx" | "pxn" | 
                "r3d" | "rwl" | "sr2" | "srf" | "srw" | "x3f" => true,
                _ => false,
            }
        }
        _ => false,
    };
    
    log::debug!("Processing image file: {} (is_video: {}, is_raw: {})", clean_path, is_video, is_raw);
    
    // Clone the path for the closure
    let image_path_for_closure = clean_path.clone();
    
    // Process the image synchronously to ensure it's ready before the response
    let processed_image = tokio::task::spawn_blocking(move || {
        log::trace!("Starting image processing task for: {}", image_path_for_closure);
        
        // Try to read and process the image file
        match std::fs::read(&image_path_for_closure) {
            Ok(image_data) => {
                log::debug!("Successfully read image data, size: {} bytes", image_data.len());
                
                if is_video {
                    log::debug!("Processing video thumbnail for: {}", image_path_for_closure);
                    // For videos, try to use the existing thumbnail generation
                    if let Some(thumbnail_base64) = generate_video_thumbnail(&image_path_for_closure) {
                        if let Ok(thumbnail_bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &thumbnail_base64) {
                            // Scale up the thumbnail to a reasonable preview size
                            if let Ok(img) = image::load_from_memory(&thumbnail_bytes) {
                                let preview = img.resize(
                                    800, 
                                    800, 
                                    image::imageops::FilterType::CatmullRom // Faster algorithm
                                );
                                let mut jpeg_bytes = Vec::new();
                                if preview.write_with_encoder(
                                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 50) // Lower quality for speed
                                ).is_ok() {
                                    // Save to cache
                                    let _ = save_full_image_to_cache(&cache_key, &jpeg_bytes);
                                    return Ok(jpeg_bytes);
                                }
                            }
                        }
                    }
                    return Err("Video preview not available".to_string());
                }
                
                // Check if this is a RAW file and try rawloader with RGB demosaicing
                let path_lower = image_path_for_closure.to_lowercase();
                if path_lower.ends_with(".nef") || path_lower.ends_with(".cr2") || path_lower.ends_with(".cr3") || 
                   path_lower.ends_with(".arw") || path_lower.ends_with(".orf") || path_lower.ends_with(".rw2") || 
                   path_lower.ends_with(".raf") || path_lower.ends_with(".dng") {
                    log::info!("Detected RAW file for preview processing: {}", image_path_for_closure);
                    
                    // Try rawloader with RGB demosaicing first
                    match generate_raw_preview(&image_path_for_closure, &cache_key) {
                        Ok(jpeg_bytes) => {
                            log::info!("Successfully processed RAW preview with rawloader RGB demosaicing");
                            return Ok(jpeg_bytes);
                        }
                        Err(e) => {
                            log::warn!("RAW rawloader processing failed, falling back to standard: {} - {}", image_path_for_closure, e);
                            // Continue to standard processing below
                        }
                    }
                }
                
                // Check if this is a TIFF file and try specialized handling first
                if path_lower.ends_with(".tiff") || path_lower.ends_with(".tif") {
                    log::info!("Detected TIFF file for preview processing: {}", image_path_for_closure);
                    match generate_tiff_preview(&image_path_for_closure, &cache_key) {
                        Ok(jpeg_bytes) => {
                            log::info!("Successfully processed TIFF preview with tiff crate");
                            return Ok(jpeg_bytes);
                        }
                        Err(e) => {
                            log::warn!("TIFF specialized processing failed, falling back to standard: {} - {}", image_path_for_closure, e);
                            // Continue to standard processing below
                        }
                    }
                }
                

                match image_path_for_closure.split('.').last().unwrap_or("").to_lowercase().as_str() {
                    "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" => {
                        generate_external_preview(&image_path_for_closure, &cache_key)
                    }
                    _ => {
                        // If image processing fails completely, try to return original as fallback
                        // but only for smaller files to avoid memory issues
                        if image_data.len() < 50_000_000 { // 50MB limit
                            Ok(image_data)
                        } else {
                            Err("Image too large and processing failed".to_string())
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to read image file {}: {}", image_path_for_closure, e);
                Err("Image file not found".to_string())
            }
        }
    }).await;
    
    match processed_image {
        Ok(Ok(jpeg_bytes)) => {
            log::info!("Successfully processed image: {}, final size: {} bytes", clean_path, jpeg_bytes.len());
            // Add cache headers based on whether this is a cache-busting request
            if is_cache_bust {
                HttpResponse::Ok()
                    .content_type("image/jpeg")
                    .append_header(("Cache-Control", "no-cache, must-revalidate"))
                    .body(jpeg_bytes)
            } else {
                HttpResponse::Ok()
                    .content_type("image/jpeg")
                    .append_header(("Cache-Control", "public, max-age=3600"))
                    .body(jpeg_bytes)
            }
        }
        Ok(Err(error_msg)) => {
            log::error!("Image processing error for {}: {}", clean_path, error_msg);
            HttpResponse::InternalServerError()
                .content_type("text/plain")
                .body(format!("Image processing failed: {}", error_msg))
        }
        Err(e) => {
            log::error!("Task execution error for image {}: {:?}", clean_path, e);
            HttpResponse::InternalServerError()
                .content_type("text/plain")
                .body("Internal processing error".to_string())
        }
    }
}

// Add this function near the other endpoints
pub async fn serve_video(path: web::Path<String>) -> impl Responder {
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
}
