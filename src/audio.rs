use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use serde::Serialize;

pub const ALLOWED_EXTENSIONS: &[&str] = &[".wav", ".mp3", ".m4a", ".flac", ".ogg", ".webm"];
pub const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;

pub fn extension_allowed(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = format!(".{}", e.to_ascii_lowercase());
            ALLOWED_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

pub fn require_regular(path: &Path) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        bail!("File must be a regular non-symlink file");
    }
    Ok(meta)
}

pub fn validate_reference_wav(path: &Path) -> Result<f64> {
    require_regular(path)?;
    let reader = WavReader::open(path).context("open wav")?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != 24_000
        || spec.bits_per_sample != 16
        || spec.sample_format != SampleFormat::Int
    {
        bail!("Converted reference must be mono 24 kHz PCM16 WAV");
    }
    let frames = reader.duration();
    let duration = frames as f64 / 24_000.0;
    if !(5.0..=30.0).contains(&duration) {
        bail!("Reference audio must be between 5 and 30 seconds");
    }
    Ok(duration)
}

pub fn convert_reference(source: &Path, destination: &Path) -> Result<f64> {
    let info = require_regular(source)?;
    if !extension_allowed(source) {
        bail!("Unsupported audio format");
    }
    if info.len() > MAX_UPLOAD_BYTES {
        bail!("Audio exceeds the 50 MiB limit");
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("destination has no parent"))?;
    let temporary = unique_temp(parent, ".convert-", ".wav")?;
    let mut published = false;
    let result = (|| {
        let status = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-y",
                "-i",
                source.to_str().ok_or_else(|| anyhow!("source path"))?,
                "-ac",
                "1",
                "-ar",
                "24000",
                "-c:a",
                "pcm_s16le",
                temporary.to_str().ok_or_else(|| anyhow!("temp path"))?,
            ])
            .status()
            .context("run ffmpeg")?;
        if !status.success() {
            bail!("ffmpeg failed");
        }
        let duration = validate_reference_wav(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        }
        hardlink_or_copy(&temporary, destination)?;
        published = true;
        let _ = fs::remove_file(&temporary);
        Ok(duration)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        if published {
            let _ = fs::remove_file(destination);
        }
    }
    result
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedMeta {
    pub sample_rate: u32,
    pub frames: u32,
    pub duration_seconds: f64,
}

pub fn validate_generated_wav(path: &Path) -> Result<GeneratedMeta> {
    let info = require_regular(path)?;
    if info.len() <= 44 {
        bail!("Generated audio is empty");
    }
    let reader = WavReader::open(path).context("open generated wav")?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != SampleFormat::Int {
        bail!("Generated audio must be PCM16 WAV");
    }
    let rate = spec.sample_rate;
    let frames = reader.duration();
    if rate == 0 || frames == 0 {
        bail!("Generated audio is invalid");
    }
    Ok(GeneratedMeta {
        sample_rate: rate,
        frames,
        duration_seconds: frames as f64 / rate as f64,
    })
}

/// Write a silent PCM16 mono WAV (tests / fakes).
pub fn write_silent_wav(path: &Path, seconds: f64, rate: u32) -> Result<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)?;
    let n = (rate as f64 * seconds) as usize;
    for _ in 0..n {
        writer.write_sample(0i16)?;
    }
    writer.finalize()?;
    Ok(())
}

fn unique_temp(dir: &Path, prefix: &str, suffix: &str) -> Result<PathBuf> {
    let name = format!("{}{}{}", prefix, uuid::Uuid::new_v4(), suffix);
    let path = dir.join(name);
    // Ensure exclusive create then close.
    File::create_new(&path)
        .with_context(|| format!("create temp {}", path.display()))?
        .write_all(&[])?;
    Ok(path)
}

fn hardlink_or_copy(src: &Path, dst: &Path) -> Result<()> {
    match fs::hard_link(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(src, dst)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn silent_wav_validates_as_generated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("g.wav");
        write_silent_wav(&path, 0.1, 24_000).unwrap();
        let meta = validate_generated_wav(&path).unwrap();
        assert!(meta.duration_seconds > 0.0);
    }

    #[test]
    fn extension_checks() {
        assert!(extension_allowed(Path::new("a.WAV")));
        assert!(extension_allowed(Path::new("a.webm")));
        assert!(!extension_allowed(Path::new("a.exe")));
    }
}
