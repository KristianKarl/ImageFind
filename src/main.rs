use actix_web::{web, App, HttpServer};
use clap::Parser;
mod routes;
mod cli;
mod sidecar_scan;
mod processing;
mod background;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Parse CLI arguments and initialize global static
    let args = cli::CliArgs::parse();
    cli::init_logging(&args);
    cli::CLI_ARGS.set(args).expect("CLI_ARGS already set");
    

    if let Err(e) = sidecar_scan::scan_and_import_sidecars() {
        eprintln!("Error importing sidecars: {}", e);
    }

    let port = cli::CLI_ARGS.get().unwrap().port;

    background::start_background_thumbnail_worker();
    background::start_background_preview_worker();

    HttpServer::new(|| {
        App::new()
            .route("/", web::get().to(routes::index))
            .route("/health_check", web::get().to(routes::health_check))
            .route("/search", web::get().to(routes::search_page))
            .route("/api", web::get().to(routes::api_search))
            .route("/image/{path:.*}", web::get().to(routes::get_preview))
            .route("/thumbnail/{path:.*}", web::get().to(routes::get_thumbnail))
            .route("/video/{path:.*}", web::get().to(routes::serve_video))
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}