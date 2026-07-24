use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CoverSpec {
    pub source_video: String,
    pub frame_timestamp: f64,
    pub layout_profile: LayoutProfile,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutProfile {
    SmokeGlass,
    BannerCard,
    Diagonal,
    EditorialBlackGold,
}

pub fn validate_render_cover(
    spec: &CoverSpec,
    stem: &str,
    video_input_root: &Path,
) -> Result<PathBuf> {
    if stem.is_empty()
        || !stem
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("cover stem must contain only ASCII letters, digits, '-' or '_'");
    }
    if !spec.frame_timestamp.is_finite() || spec.frame_timestamp < 0.0 {
        bail!("frame_timestamp must be finite and nonnegative");
    }
    if spec.title.trim().is_empty() || spec.title.chars().count() > 160 {
        bail!("cover title must contain 1 through 160 characters");
    }
    if spec.subtitle.chars().count() > 240 {
        bail!("cover subtitle must contain at most 240 characters");
    }
    crate::sampling::resolve_media_input_no_symlink(&spec.source_video, video_input_root)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn validates_supported_create_only_cover_request() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("source.mp4"), b"video").unwrap();
        let spec = CoverSpec {
            source_video: "source.mp4".into(),
            frame_timestamp: 1.25,
            layout_profile: LayoutProfile::EditorialBlackGold,
            title: "A title".into(),
            subtitle: "Subtitle".into(),
        };
        assert_eq!(
            validate_render_cover(&spec, "hero", root.path()).unwrap(),
            root.path().join("source.mp4").canonicalize().unwrap()
        );
        assert!(validate_render_cover(&spec, "../hero", root.path()).is_err());
    }
}
