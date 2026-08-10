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

/// ASR checkpoint used by a single FunClip stage-1 recognition run.
///
/// Paraformer remains the compatibility default for subtitle extraction and
/// recordings. SenseVoice is opt-in because it is the multilingual model and
/// has different recognition characteristics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsrModel {
    #[default]
    #[serde(rename = "paraformer")]
    Paraformer,
    #[serde(rename = "sensevoice")]
    SenseVoice,
}

impl AsrModel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Paraformer => "paraformer",
            Self::SenseVoice => "sensevoice",
        }
    }
}

pub const MAX_VIDEO_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Formats accepted by FunClip stage-1 for standalone recording transcription.
/// Keep this narrower than voice-reference uploads: it mirrors the upstream
/// Paraformer recording path exactly.
pub fn audio_transcription_extension_allowed(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    matches!(ext.as_deref(), Some("wav" | "mp3" | "aac" | "m4a" | "flac"))
}

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
        model: AsrModel,
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
        Self::with_python_and_cancellation(root, timeout_seconds, python, cancellation)
    }

    pub fn with_python_and_cancellation(
        root: Option<PathBuf>,
        timeout_seconds: u64,
        python: PathBuf,
        cancellation: Arc<AtomicBool>,
    ) -> Self {
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

    /// Use the project-owned streaming helper for standalone recordings when
    /// it is present.  The helper keeps FunASR loaded once and feeds it bounded
    /// ffmpeg-decoded chunks instead of asking librosa to materialize a whole
    /// multi-hour recording in one array.
    fn audio_transcriber(&self) -> Option<PathBuf> {
        let project_root = std::env::var_os("VWA_PROJECT_ROOT")
            .map(PathBuf::from)
            .or_else(|| {
                self.root
                    .as_ref()?
                    .parent()?
                    .parent()
                    .map(Path::to_path_buf)
            })?;
        let script = project_root.join("scripts").join("funclip_transcribe.py");
        script.is_file().then_some(script)
    }

    fn run_funclip(&self, mut command: Command) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
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
            FunClipWaitOutcome::Exited(status) if status.success() => Ok(()),
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
    }

    fn read_outputs(
        &self,
        output_dir: &Path,
        model: AsrModel,
    ) -> Result<(Vec<SubtitleSegment>, String, Vec<WordTimestamp>)> {
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
        // SenseVoice is multilingual but does not guarantee Paraformer's
        // token-level timestamp contract. Its SRT remains usable through the
        // fallback segment emitted by FunClip, while word timings are absent.
        let words = match model {
            AsrModel::Paraformer => parse_funclip_words(output_dir)?,
            AsrModel::SenseVoice => Vec::new(),
        };
        Ok((segments, srt, words))
    }
}

impl SubtitleExtractor for FunClipExtractor {
    fn ready(&self) -> bool {
        self.videoclipper().is_some()
    }

    fn extract(
        &self,
        video_path: &Path,
        model: AsrModel,
    ) -> Result<(Vec<SubtitleSegment>, String, Vec<WordTimestamp>)> {
        let root = self
            .root
            .as_ref()
            .context("FunClip is not installed; set VWA_FUNCLIP_ROOT")?;
        let is_recording = audio_transcription_extension_allowed(video_path);
        let (script, streaming_audio) = if is_recording {
            if let Some(script) = self.audio_transcriber() {
                (script, true)
            } else {
                (
                    self.videoclipper()
                        .context("FunClip is not installed; set VWA_FUNCLIP_ROOT")?,
                    false,
                )
            }
        } else {
            (
                self.videoclipper()
                    .context("FunClip is not installed; set VWA_FUNCLIP_ROOT")?,
                false,
            )
        };
        let temp = tempfile::tempdir_in(std::env::temp_dir()).context("temp dir")?;
        let output_dir = temp.path();
        let mut command = Command::new(&self.python);
        command.arg(&script).args([
            "--file",
            video_path.to_str().context("video path")?,
            "--output_dir",
            output_dir.to_str().context("output dir")?,
            "--model",
            model.as_str(),
        ]);
        if !streaming_audio {
            command.arg("--stage").arg("1");
        }
        command.current_dir(root);
        self.run_funclip(command)?;
        self.read_outputs(output_dir, model)
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
    let token_re =
        Regex::new(r"[\p{Han}]|[\p{L}\p{N}_-]+(?:['’][\p{L}\p{N}_-]+)*").expect("token regex");
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
        _model: AsrModel,
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
    fn recording_extensions_match_funclip_stage_one() {
        for accepted in [
            "recording.wav",
            "recording.MP3",
            "recording.aac",
            "recording.m4a",
            "recording.flac",
        ] {
            assert!(audio_transcription_extension_allowed(Path::new(accepted)));
        }
        for rejected in [
            "recording.ogg",
            "recording.webm",
            "recording.mp4",
            "recording.txt",
        ] {
            assert!(!audio_transcription_extension_allowed(Path::new(rejected)));
        }
    }

    #[test]
    fn asr_models_are_strict_and_default_to_paraformer() {
        assert_eq!(AsrModel::default(), AsrModel::Paraformer);
        assert_eq!(
            serde_json::from_str::<AsrModel>(r#""sensevoice""#).unwrap(),
            AsrModel::SenseVoice
        );
        assert!(serde_json::from_str::<AsrModel>(r#""unsupported""#).is_err());
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

    #[test]
    fn preserves_contractions_as_one_funclip_word_token() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("recog_res_raw"), "it's").unwrap();
        fs::write(temp.path().join("timestamp"), "[[1000, 1240]]").unwrap();
        let words = parse_funclip_words(temp.path()).unwrap();
        assert_eq!(words[0].word, "it's");
        assert_eq!(words[0].start, 1.0);
        assert_eq!(words[0].end, 1.24);
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

        let (segments, _, _) = extractor
            .extract(&video, AsrModel::Paraformer)
            .expect("extract subtitles");

        assert_eq!(segments[0].text, "hello world");
    }

    #[cfg(unix)]
    #[test]
    fn funclip_extract_forwards_the_selected_asr_model() {
        let temp = tempfile::tempdir().expect("temp dir");
        let funclip_dir = temp.path().join("funclip");
        fs::create_dir(&funclip_dir).expect("funclip dir");
        let helper = funclip_dir.join("videoclipper.py");
        fs::write(
            &helper,
            r#"video=''
output_dir=''
model=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --file) video=$2; shift 2 ;;
    --output_dir) output_dir=$2; shift 2 ;;
    --model) model=$2; shift 2 ;;
    *) shift ;;
  esac
done
printf '%s\n' "$model" >> "$video.models"
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

        extractor
            .extract(&video, AsrModel::Paraformer)
            .expect("default extraction");
        extractor
            .extract(&video, AsrModel::SenseVoice)
            .expect("SenseVoice extraction");

        assert_eq!(
            fs::read_to_string(video.with_extension("mp4.models")).unwrap(),
            "paraformer\nsensevoice\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn long_audio_uses_the_streaming_transcription_helper() {
        let temp = tempfile::tempdir().expect("temp dir");
        let app_root = temp.path();
        let funclip_root = app_root.join("vendor/FunClip");
        let helper_root = app_root.join("scripts");
        fs::create_dir_all(funclip_root.join("funclip")).expect("funclip root");
        fs::create_dir_all(&helper_root).expect("helper root");
        fs::write(
            helper_root.join("funclip_transcribe.py"),
            r#"output_dir=''
audio=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --file) audio=$2; shift 2 ;;
    --output_dir) output_dir=$2; shift 2 ;;
    *) shift ;;
  esac
done
touch "$audio.called"
cat > "$output_dir/total.srt" <<'EOF'
1
00:00:00,000 --> 00:00:01,000
chunked audio
EOF
printf 'chunked audio' > "$output_dir/recog_res_raw"
printf '[[0, 1000], [1000, 2000]]' > "$output_dir/timestamp"
"#,
        )
        .expect("write streaming helper");
        let audio = temp.path().join("recording.wav");
        fs::write(&audio, []).expect("write recording");
        let extractor = FunClipExtractor {
            root: Some(funclip_root),
            timeout: Duration::from_secs(1),
            python: PathBuf::from("/bin/sh"),
            cancellation: Arc::new(AtomicBool::new(false)),
        };

        let (segments, srt, words) = extractor
            .extract(&audio, AsrModel::Paraformer)
            .expect("streaming audio extraction");

        assert_eq!(segments[0].text, "chunked audio");
        assert_eq!(segments[0].start, "00:00:00.000");
        assert_eq!(segments[0].end, "00:00:01.000");
        assert!(srt.contains("00:00:00,000 --> 00:00:01,000"));
        assert_eq!(
            words
                .iter()
                .map(|word| (word.word.as_str(), word.start, word.end))
                .collect::<Vec<_>>(),
            vec![("chunked", 0.0, 1.0), ("audio", 1.0, 2.0)]
        );
        assert!(audio.with_extension("wav.called").is_file());
    }

    #[test]
    fn funclip_explicit_python_does_not_depend_on_process_environment() {
        let python = PathBuf::from("/opt/video-work-api/.venv/bin/python");
        let extractor = FunClipExtractor::with_python_and_cancellation(
            None,
            30,
            python.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(extractor.python, python);
    }

    #[cfg(unix)]
    #[test]
    fn sensevoice_extract_accepts_an_srt_without_token_timestamps() {
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
cat > "$output_dir/total.srt" <<'EOF'
1
00:00:00,000 --> 00:00:01,000
multilingual text
EOF
printf 'multilingual text' > "$output_dir/recog_res_raw"
printf '[]' > "$output_dir/timestamp"
"#,
        )
        .expect("write helper");
        let video = temp.path().join("recording.wav");
        fs::write(&video, []).expect("write recording");
        let extractor = FunClipExtractor {
            root: Some(temp.path().to_path_buf()),
            timeout: Duration::from_secs(1),
            python: PathBuf::from("/bin/sh"),
            cancellation: Arc::new(AtomicBool::new(false)),
        };

        let (segments, _, words) = extractor
            .extract(&video, AsrModel::SenseVoice)
            .expect("SenseVoice extraction without token timestamps");

        assert_eq!(segments[0].text, "multilingual text");
        assert!(words.is_empty());
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
        let result = extractor.extract(&video, AsrModel::Paraformer);
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
        let result = extractor.extract(&video, AsrModel::Paraformer);
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
        let worker =
            std::thread::spawn(move || extractor.extract(&video_for_extract, AsrModel::Paraformer));
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
