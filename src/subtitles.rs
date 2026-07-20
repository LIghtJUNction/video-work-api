use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleSegment {
    pub index: usize,
    pub start: String,
    pub end: String,
    pub text: String,
}

pub fn parse_srt(content: &str) -> Vec<SubtitleSegment> {
    let time_re = Regex::new(
        r"(?P<start>\d{2}:\d{2}:\d{2}[,.]\d{3})\s*-->\s*(?P<end>\d{2}:\d{2}:\d{2}[,.]\d{3})",
    )
    .expect("srt regex");
    let text = content.trim_start_matches('\u{feff}').trim();
    let mut result = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.len() < 3 {
            continue;
        }
        let Some(caps) = time_re.captures(lines[1]) else {
            continue;
        };
        result.push(SubtitleSegment {
            index: result.len() + 1,
            start: caps["start"].replace(',', "."),
            end: caps["end"].replace(',', "."),
            text: lines[2..].join(" "),
        });
    }
    result
}

pub trait SubtitleExtractor: Send + Sync {
    fn ready(&self) -> bool;
    fn extract(&self, video_path: &Path) -> Result<(Vec<SubtitleSegment>, String)>;
}

pub struct FunClipExtractor {
    root: Option<PathBuf>,
    timeout: Duration,
    python: PathBuf,
}

impl FunClipExtractor {
    pub fn new(root: Option<PathBuf>, timeout_seconds: u64) -> Self {
        let python = std::env::var_os("VWA_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("python3"));
        Self {
            root,
            timeout: Duration::from_secs(timeout_seconds),
            python,
        }
    }

    fn videoclipper(&self) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let script = root.join("funclip").join("videoclipper.py");
        script.is_file().then_some(script)
    }
}

impl SubtitleExtractor for FunClipExtractor {
    fn ready(&self) -> bool {
        self.videoclipper().is_some()
    }

    fn extract(&self, video_path: &Path) -> Result<(Vec<SubtitleSegment>, String)> {
        let root = self
            .root
            .as_ref()
            .context("FunClip is not installed; set VWA_FUNCLIP_ROOT")?;
        let script = self
            .videoclipper()
            .context("FunClip is not installed; set VWA_FUNCLIP_ROOT")?;
        let temp = tempfile::tempdir_in(std::env::temp_dir()).context("temp dir")?;
        // Note: we rely on process-level kill via wait_timeout-like behavior.
        // tokio is used at the HTTP layer; this path is blocking on a worker.
        let output_dir = temp.path();
        let mut child = Command::new(&self.python)
            .arg(&script)
            .args([
                "--stage",
                "1",
                "--file",
                video_path.to_str().context("video path")?,
                "--output_dir",
                output_dir.to_str().context("output dir")?,
            ])
            .current_dir(root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("spawn FunClip")?;

        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        bail!("FunClip subtitle extraction failed");
                    }
                    break;
                }
                Ok(None) => {
                    if start.elapsed() > self.timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        bail!("FunClip subtitle extraction timed out");
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => bail!("wait FunClip: {e}"),
            }
        }

        let mut srt_files: Vec<PathBuf> = Vec::new();
        collect_srt(output_dir, &mut srt_files)?;
        srt_files.sort();
        let srt_path = srt_files
            .first()
            .context("FunClip did not produce an SRT file")?;
        let srt = fs::read_to_string(srt_path).context("read srt")?;
        let segments = parse_srt(&srt);
        if segments.is_empty() {
            bail!("FunClip produced an empty or invalid SRT file");
        }
        Ok((segments, srt))
    }
}

fn collect_srt(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_srt(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("srt") {
            out.push(path);
        }
    }
    Ok(())
}

pub struct FakeSubtitles {
    pub ready_flag: bool,
}

impl Default for FakeSubtitles {
    fn default() -> Self {
        Self { ready_flag: true }
    }
}

impl SubtitleExtractor for FakeSubtitles {
    fn ready(&self) -> bool {
        self.ready_flag
    }

    fn extract(&self, _video_path: &Path) -> Result<(Vec<SubtitleSegment>, String)> {
        let segments = vec![SubtitleSegment {
            index: 1,
            start: "00:00:00.000".into(),
            end: "00:00:01.500".into(),
            text: "hello world".into(),
        }];
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nhello world\n".to_string();
        Ok((segments, srt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_srt() {
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nhello world\n\n2\n00:00:01.500 --> 00:00:02.000\nhi\n";
        let segs = parse_srt(srt);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start, "00:00:00.000");
        assert_eq!(segs[1].text, "hi");
    }
}
