use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// Trait for zero-shot speech generation backends.
pub trait SpeechEngine: Send + Sync {
    fn loaded(&self) -> bool;
    fn generate(
        &self,
        target_text: &str,
        speed: f64,
        prompt_text: &str,
        prompt_wav: &Path,
        output_path: &Path,
    ) -> Result<()>;
}

/// CosyVoice3 via a small Python helper that loads the vendored runtime.
pub struct CosyVoiceEngine {
    cosyvoice_root: PathBuf,
    model_dir: PathBuf,
    helper: PathBuf,
    python: PathBuf,
    loaded: AtomicBool,
    lock: Mutex<()>,
}

impl CosyVoiceEngine {
    pub fn new(cosyvoice_root: PathBuf, model_dir: PathBuf, project_root: &Path) -> Self {
        let helper = project_root.join("scripts").join("cosyvoice_infer.py");
        let python = std::env::var_os("VWA_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("python3"));
        Self {
            cosyvoice_root,
            model_dir,
            helper,
            python,
            loaded: AtomicBool::new(false),
            lock: Mutex::new(()),
        }
    }
}

impl SpeechEngine for CosyVoiceEngine {
    fn loaded(&self) -> bool {
        self.loaded.load(Ordering::Relaxed)
    }

    fn generate(
        &self,
        target_text: &str,
        speed: f64,
        prompt_text: &str,
        prompt_wav: &Path,
        output_path: &Path,
    ) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("engine lock poisoned"))?;
        if !self.cosyvoice_root.is_dir() || !self.model_dir.is_dir() {
            bail!("CosyVoice source or model is not installed");
        }
        if !self.helper.is_file() {
            bail!("CosyVoice helper missing at {}", self.helper.display());
        }
        let status = Command::new(&self.python)
            .arg(&self.helper)
            .args([
                "--cosyvoice-root",
                self.cosyvoice_root
                    .to_str()
                    .context("cosyvoice root utf-8")?,
                "--model-dir",
                self.model_dir.to_str().context("model dir utf-8")?,
                "--target-text",
                target_text,
                "--prompt-text",
                prompt_text,
                "--prompt-wav",
                prompt_wav.to_str().context("prompt wav utf-8")?,
                "--output",
                output_path.to_str().context("output utf-8")?,
                "--speed",
                &speed.to_string(),
            ])
            .status()
            .context("spawn cosyvoice helper")?;
        if !status.success() {
            bail!("CosyVoice inference failed (exit {status})");
        }
        self.loaded.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// Test / dry-run engine that writes silence.
pub struct FakeEngine {
    pub loaded_flag: AtomicBool,
    pub calls: Mutex<Vec<(String, f64, String, PathBuf)>>,
    delay: Option<Duration>,
}

impl FakeEngine {
    pub fn new() -> Self {
        Self {
            loaded_flag: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            delay: None,
        }
    }
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechEngine for FakeEngine {
    fn loaded(&self) -> bool {
        self.loaded_flag.load(Ordering::Relaxed)
    }

    fn generate(
        &self,
        target_text: &str,
        speed: f64,
        prompt_text: &str,
        prompt_wav: &Path,
        output_path: &Path,
    ) -> Result<()> {
        if let Some(d) = self.delay {
            std::thread::sleep(d);
        }
        self.calls.lock().unwrap().push((
            target_text.to_string(),
            speed,
            prompt_text.to_string(),
            prompt_wav.to_path_buf(),
        ));
        crate::audio::write_silent_wav(output_path, 0.1, 24_000)?;
        self.loaded_flag.store(true, Ordering::Relaxed);
        Ok(())
    }
}
