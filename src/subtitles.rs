use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::alignment::WordTimestamp;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleSegment {
    #[serde(default)]
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
    fn extract(
        &self,
        video_path: &Path,
    ) -> Result<(Vec<SubtitleSegment>, String, Vec<WordTimestamp>)>;
}

pub struct FunClipExtractor {
    root: Option<PathBuf>,
    timeout: Duration,
    python: PathBuf,
    cancellation: Arc<AtomicBool>,
}

const MAX_FUNCLIP_STDERR_BYTES: usize = 16 * 1024;

struct CapturedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

enum FunClipWaitOutcome {
    Exited(ExitStatus),
    Cancelled,
    TimedOut,
    WaitFailed(io::Error),
}

fn drain_funclip_stderr(mut stderr: ChildStderr) -> io::Result<CapturedStderr> {
    let mut tail = VecDeque::with_capacity(MAX_FUNCLIP_STDERR_BYTES);
    let mut total_bytes = 0usize;
    let mut buffer = [0u8; 8192];
    loop {
        let bytes_read = stderr.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(bytes_read);
        if bytes_read >= MAX_FUNCLIP_STDERR_BYTES {
            tail.clear();
            tail.extend(&buffer[bytes_read - MAX_FUNCLIP_STDERR_BYTES..bytes_read]);
        } else {
            let overflow = tail
                .len()
                .saturating_add(bytes_read)
                .saturating_sub(MAX_FUNCLIP_STDERR_BYTES);
            if overflow > 0 {
                tail.drain(..overflow);
            }
            tail.extend(&buffer[..bytes_read]);
        }
    }
    Ok(CapturedStderr {
        bytes: tail.into_iter().collect(),
        truncated: total_bytes > MAX_FUNCLIP_STDERR_BYTES,
    })
}

fn join_funclip_stderr(reader: JoinHandle<io::Result<CapturedStderr>>) -> String {
    match reader.join() {
        Ok(Ok(stderr)) => stderr_context(&stderr),
        Ok(Err(error)) => format!("read FunClip stderr: {error}"),
        Err(_) => "join FunClip stderr reader: thread panicked".to_string(),
    }
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

#[cfg(unix)]
fn kill_funclip_process_group(child: &Child) -> Option<String> {
    let process_group = child.id() as i32;
    // SAFETY: the child was spawned into a process group whose ID is its positive PID.
    if unsafe { libc::killpg(process_group, libc::SIGKILL) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Some(format!("kill FunClip process group: {error}"));
        }
    }
    None
}

fn sweep_funclip_descendants(child: &Child) {
    #[cfg(unix)]
    if let Some(error) = kill_funclip_process_group(child) {
        tracing::warn!(%error, "sweep FunClip descendants after main process exit");
    }
}

fn terminate_funclip(child: &mut Child) -> Vec<String> {
    let mut errors = Vec::new();

    #[cfg(unix)]
    if let Some(error) = kill_funclip_process_group(child) {
        errors.push(error);
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
        Self::with_cancellation(root, timeout_seconds, Arc::new(AtomicBool::new(false)))
    }

    pub fn with_cancellation(
        root: Option<PathBuf>,
        timeout_seconds: u64,
        cancellation: Arc<AtomicBool>,
    ) -> Self {
        let python = std::env::var_os("VWA_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("python3"));
        Self {
            root,
            timeout: Duration::from_secs(timeout_seconds),
            python,
            cancellation,
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

    fn extract(
        &self,
        video_path: &Path,
    ) -> Result<(Vec<SubtitleSegment>, String, Vec<WordTimestamp>)> {
        let root = self
            .root
            .as_ref()
            .context("FunClip is not installed; set VWA_FUNCLIP_ROOT")?;
        let script = self
            .videoclipper()
            .context("FunClip is not installed; set VWA_FUNCLIP_ROOT")?;
        let temp = tempfile::tempdir_in(std::env::temp_dir()).context("temp dir")?;
        let output_dir = temp.path();
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
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn FunClip")?;
        let stderr = child.stderr.take().context("capture FunClip stderr")?;
        let stderr_reader = std::thread::spawn(move || drain_funclip_stderr(stderr));

        let start = std::time::Instant::now();
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break FunClipWaitOutcome::Exited(status),
                Ok(None) => {
                    if self.cancellation.load(Ordering::Acquire) {
                        break FunClipWaitOutcome::Cancelled;
                    }
                    if start.elapsed() > self.timeout {
                        break FunClipWaitOutcome::TimedOut;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => break FunClipWaitOutcome::WaitFailed(error),
            }
        };
        let cleanup_errors = match &outcome {
            FunClipWaitOutcome::Exited(_) => {
                sweep_funclip_descendants(&child);
                Vec::new()
            }
            FunClipWaitOutcome::Cancelled
            | FunClipWaitOutcome::TimedOut
            | FunClipWaitOutcome::WaitFailed(_) => terminate_funclip(&mut child),
        };
        let stderr = join_funclip_stderr(stderr_reader);
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
            FunClipWaitOutcome::Cancelled => {
                bail!("FunClip subtitle extraction cancelled during service shutdown; {stderr}{cleanup}")
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
        let words = parse_funclip_words(output_dir)?;
        Ok((segments, srt, words))
    }
}

fn parse_funclip_words(output_dir: &Path) -> Result<Vec<WordTimestamp>> {
    let raw_path = output_dir.join("recog_res_raw");
    let timestamp_path = output_dir.join("timestamp");
    if !raw_path.exists() && !timestamp_path.exists() {
        return Ok(Vec::new());
    }
    if !raw_path.is_file() || !timestamp_path.is_file() {
        bail!("FunClip word timestamp state is incomplete");
    }
    let raw = fs::read_to_string(raw_path).context("read FunClip raw recognition text")?;
    let timestamp_literal =
        fs::read_to_string(timestamp_path).context("read FunClip token timestamps")?;
    // FunClip persists Python list/tuple literals. Timestamp payloads are
    // numeric-only, so normalizing tuple delimiters gives serde a strict,
    // non-executable parser while retaining compatibility with both forms.
    let normalized = timestamp_literal.replace('(', "[").replace(')', "]");
    let timestamps: Vec<[f64; 2]> =
        serde_json::from_str(&normalized).context("parse FunClip token timestamps")?;
    let token_re = Regex::new(r"[\p{Han}]|[\p{L}\p{N}_-]+").expect("token regex");
    let tokens = token_re
        .find_iter(&raw)
        .map(|item| item.as_str().to_string())
        .collect::<Vec<_>>();
    if tokens.len() != timestamps.len() {
        bail!(
            "FunClip token/timestamp cardinality mismatch: {} tokens, {} timestamps",
            tokens.len(),
            timestamps.len()
        );
    }
    let mut previous_end = 0.0;
    tokens
        .into_iter()
        .zip(timestamps)
        .map(|(word, [start_ms, end_ms])| {
            let start = start_ms / 1000.0;
            let end = end_ms / 1000.0;
            if !start.is_finite() || !end.is_finite() || start < previous_end || end <= start {
                bail!("FunClip token timestamps must be finite, positive, and monotonic");
            }
            previous_end = end;
            Ok(WordTimestamp { word, start, end })
        })
        .collect()
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

    fn extract(
        &self,
        _video_path: &Path,
    ) -> Result<(Vec<SubtitleSegment>, String, Vec<WordTimestamp>)> {
        let segments = vec![SubtitleSegment {
            index: 1,
            start: "00:00:00.000".into(),
            end: "00:00:01.500".into(),
            text: "hello world".into(),
        }];
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nhello world\n".to_string();
        Ok((segments, srt, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn process_terminated(pid: i32) -> bool {
        // SAFETY: signal 0 only probes a positive PID written by the test helper.
        if unsafe { libc::kill(pid, 0) } == -1
            && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return true;
        }

        #[cfg(target_os = "linux")]
        {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat"));
            match stat {
                Ok(stat) => {
                    stat.rsplit_once(") ")
                        .and_then(|(_, fields)| fields.chars().next())
                        == Some('Z')
                }
                Err(error) => error.kind() == io::ErrorKind::NotFound,
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    #[test]
    fn parse_basic_srt() {
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nhello world\n\n2\n00:00:01.500 --> 00:00:02.000\nhi\n";
        let segs = parse_srt(srt);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start, "00:00:00.000");
        assert_eq!(segs[1].text, "hi");
    }

    #[test]
    fn parses_genuine_funclip_token_timestamps_without_interpolation() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("recog_res_raw"), "你好 hello-world").unwrap();
        fs::write(
            temp.path().join("timestamp"),
            "[(0, 120), (120, 260), (300, 700)]",
        )
        .unwrap();
        let words = parse_funclip_words(temp.path()).unwrap();
        assert_eq!(
            words,
            vec![
                WordTimestamp {
                    word: "你".into(),
                    start: 0.0,
                    end: 0.12,
                },
                WordTimestamp {
                    word: "好".into(),
                    start: 0.12,
                    end: 0.26,
                },
                WordTimestamp {
                    word: "hello-world".into(),
                    start: 0.3,
                    end: 0.7,
                },
            ]
        );
    }

    #[test]
    fn rejects_funclip_token_timestamp_cardinality_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("recog_res_raw"), "one two").unwrap();
        fs::write(temp.path().join("timestamp"), "[[0, 100]]").unwrap();
        assert!(parse_funclip_words(temp.path())
            .unwrap_err()
            .to_string()
            .contains("cardinality mismatch"));
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
            cancellation: Arc::new(AtomicBool::new(false)),
        };

        let (segments, _, _) = extractor.extract(&video).expect("extract subtitles");

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
            cancellation: Arc::new(AtomicBool::new(false)),
        };

        let start = std::time::Instant::now();
        let result = extractor.extract(&video);
        let elapsed = start.elapsed();
        let pid: i32 = fs::read_to_string(video.with_extension("mp4.descendant.pid"))
            .expect("read descendant pid")
            .trim()
            .parse()
            .expect("parse descendant pid");
        let descendant_terminated = (0..20).any(|_| {
            let terminated = process_terminated(pid);
            if !terminated {
                std::thread::sleep(Duration::from_millis(25));
            }
            terminated
        });

        assert!(
            result.is_err() && elapsed < Duration::from_secs(1) && descendant_terminated,
            "result={result:?}, elapsed={elapsed:?}, descendant_terminated={descendant_terminated}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn funclip_success_sweeps_descendant_that_inherits_stderr() {
        let temp = tempfile::tempdir().expect("temp dir");
        let funclip_dir = temp.path().join("funclip");
        fs::create_dir(&funclip_dir).expect("funclip dir");
        let helper = funclip_dir.join("videoclipper.py");
        fs::write(
            &helper,
            r#"video=''
output_dir=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --file) video=$2; shift 2 ;;
    --output_dir) output_dir=$2; shift 2 ;;
    *) shift ;;
  esac
done
(sleep 3) >&2 &
printf '%s\n' "$!" > "$video.descendant.pid"
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
            timeout: Duration::from_secs(2),
            python: PathBuf::from("/bin/sh"),
            cancellation: Arc::new(AtomicBool::new(false)),
        };

        let start = std::time::Instant::now();
        let result = extractor.extract(&video);
        let elapsed = start.elapsed();
        let pid: i32 = fs::read_to_string(video.with_extension("mp4.descendant.pid"))
            .expect("read descendant pid")
            .trim()
            .parse()
            .expect("parse descendant pid");

        assert!(result.is_ok(), "result={result:?}");
        assert!(elapsed < Duration::from_secs(1), "elapsed={elapsed:?}");
        assert!(process_terminated(pid), "descendant {pid} still running");
    }

    #[cfg(unix)]
    #[test]
    fn funclip_cancellation_terminates_active_process_group() {
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
touch "$video.started"
sleep 3
"#,
        )
        .expect("write helper");
        let video = temp.path().join("video.mp4");
        fs::write(&video, []).expect("write video");
        let cancellation = Arc::new(AtomicBool::new(false));
        let extractor = FunClipExtractor {
            root: Some(temp.path().to_path_buf()),
            timeout: Duration::from_secs(5),
            python: PathBuf::from("/bin/sh"),
            cancellation: cancellation.clone(),
        };
        let video_for_extract = video.clone();
        let worker = std::thread::spawn(move || extractor.extract(&video_for_extract));
        let started = (0..40).any(|_| {
            if video.with_extension("mp4.started").is_file() {
                true
            } else {
                std::thread::sleep(Duration::from_millis(25));
                false
            }
        });
        assert!(started, "FunClip helper did not start");

        let cancel_started = std::time::Instant::now();
        cancellation.store(true, Ordering::Release);
        let error = worker
            .join()
            .expect("join extraction")
            .expect_err("cancellation should fail extraction");

        assert!(
            cancel_started.elapsed() < Duration::from_secs(1),
            "elapsed={:?}",
            cancel_started.elapsed()
        );
        assert!(
            error
                .to_string()
                .contains("cancelled during service shutdown"),
            "error={error:#}"
        );
    }
}
