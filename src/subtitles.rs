use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
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

pub const MAX_VIDEO_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn video_extension_allowed(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some(
            "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "ts" | "mpg" | "mpeg" | "flv" | "wmv"
        )
    )
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

const MAX_FUNCLIP_STDERR_BYTES: usize = 16 * 1024;

struct CapturedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

enum FunClipWaitOutcome {
    Exited(ExitStatus),
    TimedOut,
    WaitFailed(io::Error),
}

fn read_funclip_stderr(path: &Path) -> io::Result<CapturedStderr> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let truncated = length > MAX_FUNCLIP_STDERR_BYTES as u64;
    if truncated {
        file.seek(SeekFrom::End(-(MAX_FUNCLIP_STDERR_BYTES as i64)))?;
    }
    let mut bytes = Vec::with_capacity(length.min(MAX_FUNCLIP_STDERR_BYTES as u64) as usize);
    file.read_to_end(&mut bytes)?;
    Ok(CapturedStderr { bytes, truncated })
}

fn stderr_context(stderr: &CapturedStderr) -> String {
    let content = String::from_utf8_lossy(&stderr.bytes);
    let content = content.trim();
    let prefix = if stderr.truncated {
        "FunClip stderr (truncated to last 16384 bytes)"
    } else {
        "FunClip stderr"
    };
    if content.is_empty() {
        format!("{prefix}: <empty>")
    } else {
        format!("{prefix}: {content}")
    }
}

fn terminate_funclip(child: &mut Child) -> Vec<String> {
    let mut errors = Vec::new();

    #[cfg(unix)]
    {
        let process_group = child.id() as i32;
        // SAFETY: the child was spawned into a process group whose ID is its positive PID.
        if unsafe { libc::killpg(process_group, libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                errors.push(format!("kill FunClip process group: {error}"));
            }
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = child.kill() {
        errors.push(format!("kill FunClip: {error}"));
    }

    if let Err(error) = child.wait() {
        errors.push(format!("wait after killing FunClip: {error}"));
    }
    errors
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
        let output_dir = temp.path();
        let stderr_path = output_dir.join("funclip.stderr.log");
        let stderr_file = File::create(&stderr_path).context("create FunClip stderr log")?;
        let mut command = Command::new(&self.python);
        command.arg(&script).args([
            "--stage",
            "1",
            "--file",
            video_path.to_str().context("video path")?,
            "--output_dir",
            output_dir.to_str().context("output dir")?,
        ]);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .current_dir(root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(stderr_file))
            .spawn()
            .context("spawn FunClip")?;

        let start = std::time::Instant::now();
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break FunClipWaitOutcome::Exited(status),
                Ok(None) => {
                    if start.elapsed() > self.timeout {
                        break FunClipWaitOutcome::TimedOut;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => break FunClipWaitOutcome::WaitFailed(error),
            }
        };
        let cleanup_errors = match &outcome {
            FunClipWaitOutcome::Exited(_) => Vec::new(),
            FunClipWaitOutcome::TimedOut | FunClipWaitOutcome::WaitFailed(_) => {
                terminate_funclip(&mut child)
            }
        };
        let stderr = match read_funclip_stderr(&stderr_path) {
            Ok(stderr) => stderr_context(&stderr),
            Err(error) => format!("read FunClip stderr log: {error}"),
        };
        let cleanup = if cleanup_errors.is_empty() {
            String::new()
        } else {
            format!("; cleanup errors: {}", cleanup_errors.join("; "))
        };
        match outcome {
            FunClipWaitOutcome::Exited(status) if status.success() => {}
            FunClipWaitOutcome::Exited(_) => {
                bail!("FunClip subtitle extraction failed; {stderr}{cleanup}")
            }
            FunClipWaitOutcome::TimedOut => {
                bail!("FunClip subtitle extraction timed out; {stderr}{cleanup}")
            }
            FunClipWaitOutcome::WaitFailed(error) => {
                bail!("wait FunClip: {error}; {stderr}{cleanup}")
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

    #[cfg(unix)]
    #[test]
    fn funclip_extract_succeeds_when_stderr_exceeds_pipe_capacity() {
        let temp = tempfile::tempdir().expect("temp dir");
        let funclip_dir = temp.path().join("funclip");
        fs::create_dir(&funclip_dir).expect("funclip dir");
        let helper = funclip_dir.join("videoclipper.py");
        fs::write(
            &helper,
            r#"output_dir=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--output_dir' ]; then
    output_dir=$2
    shift 2
  else
    shift
  fi
done
head -c 1048576 /dev/zero >&2
cat > "$output_dir/total.srt" <<'EOF'
1
00:00:00,000 --> 00:00:01,500
hello world
EOF
"#,
        )
        .expect("write helper");
        let video = temp.path().join("video.mp4");
        fs::write(&video, []).expect("write video");
        let extractor = FunClipExtractor {
            root: Some(temp.path().to_path_buf()),
            timeout: Duration::from_secs(1),
            python: PathBuf::from("/bin/sh"),
        };

        let (segments, _) = extractor.extract(&video).expect("extract subtitles");

        assert_eq!(segments[0].text, "hello world");
    }

    #[cfg(unix)]
    #[test]
    fn funclip_timeout_kills_descendants_that_inherit_stderr() {
        let temp = tempfile::tempdir().expect("temp dir");
        let funclip_dir = temp.path().join("funclip");
        fs::create_dir(&funclip_dir).expect("funclip dir");
        let helper = funclip_dir.join("videoclipper.py");
        fs::write(
            &helper,
            r#"video=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--file' ]; then
    video=$2
    shift 2
  else
    shift
  fi
done
(sleep 3) &
printf '%s\n' "$!" > "$video.descendant.pid"
wait
"#,
        )
        .expect("write helper");
        let video = temp.path().join("video.mp4");
        fs::write(&video, []).expect("write video");
        let extractor = FunClipExtractor {
            root: Some(temp.path().to_path_buf()),
            timeout: Duration::from_millis(100),
            python: PathBuf::from("/bin/sh"),
        };

        let start = std::time::Instant::now();
        let result = extractor.extract(&video);
        let elapsed = start.elapsed();
        let pid: i32 = fs::read_to_string(video.with_extension("mp4.descendant.pid"))
            .expect("read descendant pid")
            .trim()
            .parse()
            .expect("parse descendant pid");
        let descendant_gone = (0..20).any(|_| {
            // SAFETY: kill with signal 0 only probes a positive PID written by the test helper.
            let gone = unsafe { libc::kill(pid, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if !gone {
                std::thread::sleep(Duration::from_millis(25));
            }
            gone
        });

        assert!(
            result.is_err() && elapsed < Duration::from_secs(1) && descendant_gone,
            "result={result:?}, elapsed={elapsed:?}, descendant_gone={descendant_gone}"
        );
    }
}
