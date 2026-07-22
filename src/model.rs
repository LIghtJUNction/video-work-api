use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use anyhow::{bail, Context, Result};

use crate::config::Settings;

const REQUIRED_MODEL_FILES: &[&str] = &[
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

pub fn download_model(settings: &Settings) -> Result<()> {
    let helper = settings.project_root.join("scripts/download_model.py");
    if !helper.is_file() {
        bail!("download_model.py missing at {}", helper.display());
    }
    if which_bin("hf").is_none() && which_bin("huggingface-cli").is_none() {
        bail!(
            "Hugging Face CLI (`hf`) not found on PATH. Install it \
             (Arch: pacman -S python-huggingface-hub) and retry."
        );
    }
    std::fs::create_dir_all(&settings.model_dir)
        .with_context(|| format!("create model dir {}", settings.model_dir.display()))?;
    let status = Command::new("python3")
        .arg(&helper)
        .arg("--output")
        .arg(&settings.model_dir)
        .status()
        .context("run download_model.py (hf download)")?;
    if !status.success() {
        bail!("model download failed (hf download exited {status})");
    }
    if !model_assets_present(&settings.model_dir) {
        bail!("model download completed but required inference files are missing");
    }
    Ok(())
}

pub fn model_files_present(settings: &Settings) -> bool {
    model_assets_present(&settings.model_dir)
}

fn model_assets_present(model_dir: &std::path::Path) -> bool {
    model_dir.is_dir()
        && REQUIRED_MODEL_FILES
            .iter()
            .all(|relative| model_dir.join(relative).is_file())
}

fn which_bin(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|dir| {
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
    fn partial_model_assets_are_not_present() {
        let root = tempfile::tempdir().unwrap();
        create_model_files(
            root.path(),
            &REQUIRED_MODEL_FILES[..REQUIRED_MODEL_FILES.len() - 1],
        );
        assert!(!model_assets_present(root.path()));
    }

    #[test]
    fn complete_required_model_assets_are_present_without_demo_assets() {
        let root = tempfile::tempdir().unwrap();
        create_model_files(root.path(), REQUIRED_MODEL_FILES);
        assert!(model_assets_present(root.path()));
        assert!(!root.path().join("asset").exists());
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
}
