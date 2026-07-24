//! Model weight download and presence checks.
//!
//! Both voice (CosyVoice3) and translation (MADLAD-400-3B) use the same mechanism:
//! optional HF CLI download into a fixed directory, required-file presence checks,
//! single-flight download managers, and lazy load on first inference (in engines).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::config::Settings;

/// Which offline weight set is being managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelKind {
    /// Fun-CosyVoice3 zero-shot speech.
    Voice,
    /// google/madlad400-3b-mt machine translation.
    Translation,
}

impl ModelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Voice => "voice",
            Self::Translation => "translation",
        }
    }

    pub fn hub_id(self) -> &'static str {
        match self {
            Self::Voice => "FunAudioLLM/Fun-CosyVoice3-0.5B-2512",
            Self::Translation => "google/madlad400-3b-mt",
        }
    }

    pub fn download_path(self) -> &'static str {
        match self {
            Self::Voice => "/api/model/download",
            Self::Translation => "/api/model/download-translation",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "voice" | "cosyvoice" | "speech" => Ok(Self::Voice),
            "translation" | "translate" | "madlad" => Ok(Self::Translation),
            other => bail!("unknown model kind {other:?}; use voice or translation"),
        }
    }
}

const REQUIRED_VOICE_FILES: &[&str] = &[
    "config.json",
    "cosyvoice3.yaml",
    "campplus.onnx",
    "speech_tokenizer_v3.onnx",
    "llm.pt",
    "flow.pt",
    "hift.pt",
    "CosyVoice-BlankEN/config.json",
    "CosyVoice-BlankEN/model.safetensors",
    "CosyVoice-BlankEN/tokenizer_config.json",
    "CosyVoice-BlankEN/merges.txt",
    "CosyVoice-BlankEN/vocab.json",
];

const REQUIRED_TRANSLATION_FILES: &[&str] = &[
    "config.json",
    "model.safetensors",
    "spiece.model",
    "tokenizer_config.json",
];

fn required_files(kind: ModelKind) -> &'static [&'static str] {
    match kind {
        ModelKind::Voice => REQUIRED_VOICE_FILES,
        ModelKind::Translation => REQUIRED_TRANSLATION_FILES,
    }
}

fn download_helper(settings: &Settings, kind: ModelKind) -> PathBuf {
    match kind {
        ModelKind::Voice => settings.project_root.join("scripts/download_model.py"),
        ModelKind::Translation => settings
            .project_root
            .join("scripts/download_madlad_model.py"),
    }
}

pub fn model_dir(settings: &Settings, kind: ModelKind) -> &Path {
    match kind {
        ModelKind::Voice => settings.model_dir.as_path(),
        ModelKind::Translation => settings.translation_model_dir.as_path(),
    }
}

/// Download pinned weights for `kind` via the matching Python helper + HF CLI.
pub fn download_model_kind(settings: &Settings, kind: ModelKind) -> Result<()> {
    let helper = download_helper(settings, kind);
    if !helper.is_file() {
        bail!(
            "{} missing at {}",
            helper
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("download helper"),
            helper.display()
        );
    }
    if which_bin("hf").is_none() && which_bin("huggingface-cli").is_none() {
        bail!(
            "Hugging Face CLI (`hf`) not found on PATH. Install it \
             (Arch: pacman -S python-huggingface-hub) and retry."
        );
    }
    let output = model_dir(settings, kind);
    std::fs::create_dir_all(output)
        .with_context(|| format!("create model dir {}", output.display()))?;
    let status = Command::new("python3")
        .arg(&helper)
        .arg("--output")
        .arg(output)
        .status()
        .with_context(|| format!("run {} (hf download)", helper.display()))?;
    if !status.success() {
        bail!(
            "{} model download failed (hf download exited {status})",
            kind.as_str()
        );
    }
    if !model_kind_files_present(settings, kind) {
        bail!(
            "{} model download completed but required inference files are missing",
            kind.as_str()
        );
    }
    Ok(())
}

/// Backward-compatible alias: download CosyVoice voice weights.
pub fn download_model(settings: &Settings) -> Result<()> {
    download_model_kind(settings, ModelKind::Voice)
}

/// Backward-compatible alias: download MADLAD translation weights.
pub fn download_translation_model(settings: &Settings) -> Result<()> {
    download_model_kind(settings, ModelKind::Translation)
}

pub fn model_kind_files_present(settings: &Settings, kind: ModelKind) -> bool {
    model_assets_present(model_dir(settings, kind), required_files(kind))
}

/// Voice (CosyVoice) weights on disk.
pub fn model_files_present(settings: &Settings) -> bool {
    model_kind_files_present(settings, ModelKind::Voice)
}

/// Translation (MADLAD) weights on disk.
pub fn translation_model_files_present(settings: &Settings) -> bool {
    model_kind_files_present(settings, ModelKind::Translation)
}

/// Directory-level check used by engines (no Settings needed).
pub fn translation_dir_files_present(model_dir: &Path) -> bool {
    model_assets_present(model_dir, REQUIRED_TRANSLATION_FILES)
}

fn model_assets_present(model_dir: &Path, files: &[&str]) -> bool {
    model_dir.is_dir() && files.iter().all(|relative| model_dir.join(relative).is_file())
}

fn which_bin(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(name);
            candidate.is_file().then_some(candidate)
        })
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModelDownloadState {
    #[default]
    Idle,
    Running,
    Succeeded,
    Failed,
}

impl ModelDownloadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// Single-flight download state for one model kind.
#[derive(Debug, Default)]
pub struct ModelDownloadManager {
    state: Mutex<ModelDownloadState>,
}

impl ModelDownloadManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, ModelDownloadState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn state(&self) -> ModelDownloadState {
        *self.lock()
    }

    pub fn try_start(&self) -> bool {
        let mut state = self.lock();
        if *state == ModelDownloadState::Running {
            return false;
        }
        *state = ModelDownloadState::Running;
        true
    }

    pub fn finish(&self, succeeded: bool) {
        *self.lock() = if succeeded {
            ModelDownloadState::Succeeded
        } else {
            ModelDownloadState::Failed
        };
    }
}

/// Independent single-flight managers so voice and translation downloads never block each other.
#[derive(Debug, Default)]
pub struct ModelDownloadRegistry {
    voice: ModelDownloadManager,
    translation: ModelDownloadManager,
}

impl ModelDownloadRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn manager(&self, kind: ModelKind) -> &ModelDownloadManager {
        match kind {
            ModelKind::Voice => &self.voice,
            ModelKind::Translation => &self.translation,
        }
    }
}

/// Status fragment shared by `/api/status` and download endpoints.
pub fn model_kind_status(
    settings: &Settings,
    kind: ModelKind,
    download_state: ModelDownloadState,
    runtime_ready: bool,
    loaded: bool,
) -> Value {
    let present = model_kind_files_present(settings, kind);
    let ready = match kind {
        ModelKind::Voice => present && runtime_ready && settings.cosyvoice_root.is_dir(),
        ModelKind::Translation => present && runtime_ready,
    };
    json!({
        "kind": kind.as_str(),
        "id": kind.hub_id(),
        "state": download_state.as_str(),
        "model_present": present,
        "present": present,
        "runtime_ready": runtime_ready,
        "ready": ready,
        "loaded": loaded,
        "download_path": kind.download_path(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_model_files(root: &std::path::Path, files: &[&str]) {
        for relative in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"test").unwrap();
        }
    }

    #[test]
    fn partial_voice_assets_are_not_present() {
        let root = tempfile::tempdir().unwrap();
        create_model_files(
            root.path(),
            &REQUIRED_VOICE_FILES[..REQUIRED_VOICE_FILES.len() - 1],
        );
        assert!(!model_assets_present(root.path(), REQUIRED_VOICE_FILES));
    }

    #[test]
    fn complete_required_voice_assets_are_present_without_demo_assets() {
        let root = tempfile::tempdir().unwrap();
        create_model_files(root.path(), REQUIRED_VOICE_FILES);
        assert!(model_assets_present(root.path(), REQUIRED_VOICE_FILES));
        assert!(!root.path().join("asset").exists());
    }

    #[test]
    fn complete_translation_assets_are_detected() {
        let root = tempfile::tempdir().unwrap();
        create_model_files(root.path(), REQUIRED_TRANSLATION_FILES);
        assert!(translation_dir_files_present(root.path()));
    }

    #[test]
    fn manager_is_single_flight() {
        let manager = ModelDownloadManager::new();
        assert!(manager.try_start());
        assert!(!manager.try_start());
    }

    #[test]
    fn manager_records_success_and_allows_retry() {
        let manager = ModelDownloadManager::new();
        assert!(manager.try_start());
        manager.finish(true);
        assert_eq!(manager.state(), ModelDownloadState::Succeeded);
        assert!(manager.try_start());
    }

    #[test]
    fn manager_records_failure() {
        let manager = ModelDownloadManager::new();
        assert!(manager.try_start());
        manager.finish(false);
        assert_eq!(manager.state(), ModelDownloadState::Failed);
        assert!(manager.try_start());
    }

    #[test]
    fn registry_keeps_kinds_independent() {
        let registry = ModelDownloadRegistry::new();
        assert!(registry.manager(ModelKind::Voice).try_start());
        assert!(registry.manager(ModelKind::Translation).try_start());
        assert!(!registry.manager(ModelKind::Voice).try_start());
    }

    #[test]
    fn parses_kind_aliases() {
        assert_eq!(ModelKind::parse("voice").unwrap(), ModelKind::Voice);
        assert_eq!(ModelKind::parse("madlad").unwrap(), ModelKind::Translation);
        assert!(ModelKind::parse("nope").is_err());
    }
}
