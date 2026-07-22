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
pub mod mcp_token;
pub mod model;
pub mod passkeys;
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

pub(crate) fn target_text_is_valid(text: &str) -> bool {
    !text.is_empty() && text.chars().count() <= MAX_TEXT_LENGTH
}

#[cfg(test)]
mod target_text_tests {
    use super::target_text_is_valid;

    #[test]
    fn accepts_1200_chinese_characters() {
        assert!(target_text_is_valid(&"中".repeat(1200)));
    }

    #[test]
    fn rejects_1201_chinese_characters() {
        assert!(!target_text_is_valid(&"中".repeat(1201)));
    }

    #[test]
    fn rejects_whitespace_after_caller_trims_it() {
        assert!(!target_text_is_valid("   \n".trim()));
    }
}
