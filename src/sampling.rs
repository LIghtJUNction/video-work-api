use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

fn default_max_frames() -> usize {
    12
}

fn default_resolution() -> (u32, u32) {
    (360, 640)
}

fn default_timestamp_overlay() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractAnalysisFramesRequest {
    pub video_path: String,
    #[serde(default = "default_max_frames")]
    pub max_frames: usize,
    #[serde(default = "default_resolution")]
    pub resolution: (u32, u32),
    #[serde(default = "default_timestamp_overlay")]
    pub add_timestamp_overlay: bool,
    #[serde(default)]
    pub asr_segments: Vec<AnalysisSegment>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSegment {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

impl ExtractAnalysisFramesRequest {
    pub fn validate(&self, video_input_root: &Path) -> Result<PathBuf> {
        if self.max_frames == 0 || self.max_frames > 12 {
            bail!("max_frames must be between 1 and 12");
        }
        if self.resolution.0 == 0
            || self.resolution.1 == 0
            || self.resolution.0 > 1920
            || self.resolution.1 > 1920
        {
            bail!("resolution must be between 1x1 and 1920x1920");
        }
        validate_segments(&self.asr_segments)?;
        resolve_media_input_no_symlink(&self.video_path, video_input_root)
    }
}

fn validate_segments(segments: &[AnalysisSegment]) -> Result<()> {
    let mut previous_end = 0.0;
    for segment in segments {
        if !segment.start_seconds.is_finite()
            || !segment.end_seconds.is_finite()
            || segment.start_seconds < 0.0
            || segment.end_seconds <= segment.start_seconds
            || segment.start_seconds < previous_end
            || segment.text.chars().count() > 2_000
        {
            bail!(
                "ASR segments must be ordered, nonoverlapping, finite, and at most 2000 characters"
            );
        }
        previous_end = segment.end_seconds;
    }
    Ok(())
}

pub fn resolve_media_input_no_symlink(raw: &str, root: &Path) -> Result<PathBuf> {
    let relative = Path::new(raw);
    if raw.is_empty()
        || relative.is_absolute()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("media path must be a safe relative path inside VWA_VIDEO_INPUT_DIR");
    }
    let root = root
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("VWA_VIDEO_INPUT_DIR is unavailable"))?;
    let mut current = root.clone();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|_| anyhow::anyhow!("media input does not exist"))?;
        if metadata.file_type().is_symlink() {
            bail!("media input path must not contain symlinks");
        }
    }
    let metadata = std::fs::symlink_metadata(&current)?;
    if !metadata.is_file() {
        bail!("media input must be a regular file");
    }
    let canonical = current.canonicalize()?;
    if !canonical.starts_with(&root) {
        bail!("media input escaped VWA_VIDEO_INPUT_DIR");
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn defaults_are_bounded_and_unknown_fields_are_rejected() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("clip.mp4"), b"video").unwrap();
        let request: ExtractAnalysisFramesRequest =
            serde_json::from_value(serde_json::json!({"video_path": "clip.mp4"})).unwrap();
        assert_eq!(request.max_frames, 12);
        assert_eq!(request.resolution, (360, 640));
        request.validate(root.path()).unwrap();
        assert!(
            serde_json::from_value::<ExtractAnalysisFramesRequest>(serde_json::json!({
                "video_path": "clip.mp4",
                "duration_seconds": 4.0
            }))
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_media_paths() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("clip.mp4"), b"video").unwrap();
        symlink(outside.path(), root.path().join("linked")).unwrap();
        assert!(resolve_media_input_no_symlink("linked/clip.mp4", root.path()).is_err());
    }
}
