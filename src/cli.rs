use clap::{Parser, ValueEnum};
use std::sync::OnceLock;

/// Log level enum for CLI
#[derive(Debug, Clone, ValueEnum)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn to_log_level_filter(&self) -> log::LevelFilter {
        match self {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        }
    }
}

/// Command line arguments for ImageFind
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct CliArgs {
    /// Path to the SQLite database file
    #[arg(long, required = true)]
    pub db_path: String,

    /// Path to the thumbnail cache directory
    #[arg(long, required = true)]
    pub thumbnail_cache: String,

    /// Path to the full image cache directory
    #[arg(long, required = true)]
    pub full_image_cache: String,

    /// Path to the video preview cache directory
    #[arg(long, required = true)]
    pub video_preview_cache: String,

    /// Directory to scan for XMP sidecar files
    #[arg(long, required = true)]
    pub scan_dir: String,

    /// Set the logging level
    #[arg(long, value_enum, default_value = "info")]
    pub log_level: LogLevel,

    /// Port for the webserver (default: 8080)
    #[arg(long, default_value_t = 8080)]
    pub port: u16,
}

pub static CLI_ARGS: OnceLock<CliArgs> = OnceLock::new();

pub fn get_cli_args() -> &'static CliArgs {
    CLI_ARGS.get().expect("CLI_ARGS not initialized")
}

/// Initialize logging based on CLI arguments
pub fn init_logging(args: &CliArgs) {
    env_logger::Builder::from_default_env()
        .filter_level(args.log_level.to_log_level_filter())
        .init();
    
    log::info!("Logging initialized at level: {:?}", args.log_level);
}