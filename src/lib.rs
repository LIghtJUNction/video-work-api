//! Video Work API — authorized zero-shot voice cloning and FunClip subtitles.

pub mod audio;
pub mod config;
pub mod database;
pub mod engine;
pub mod error;
pub mod filenames;
pub mod http;
pub mod importer;
pub mod mcp;
pub mod paths;
pub mod security;
pub mod studio;
pub mod subtitles;

pub use config::Settings;
pub use error::{AppError, AppResult};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PRODUCT: &str = "video-work-api";
pub const COOKIE_NAME: &str = "vwa_session";
pub const MAX_TEXT_LENGTH: usize = 1200;
