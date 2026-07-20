use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audio::{self, convert_reference, validate_generated_wav, MAX_UPLOAD_BYTES};
use crate::config::Settings;
use crate::database::Database;
use crate::engine::SpeechEngine;
use crate::paths::{resolve_under_root, safe_owned_file};
use crate::subtitles::SubtitleExtractor;
use crate::{MAX_TEXT_LENGTH, PRODUCT};

pub struct Studio {
    pub settings: Settings,
    pub database: Database,
    pub engine: Arc<dyn SpeechEngine>,
    pub subtitles: Arc<dyn SubtitleExtractor>,
}

impl Studio {
    pub fn new(
        settings: Settings,
        database: Database,
        engine: Arc<dyn SpeechEngine>,
        subtitles: Arc<dyn SubtitleExtractor>,
    ) -> Self {
        Self {
            settings,
            database,
            engine,
            subtitles,
        }
    }

    pub fn status_payload(&self, authenticated: bool) -> Result<Value> {
        Ok(json!({
            "product": PRODUCT,
            "service": "Video Work API",
            "status": "ready",
            "configured": self.database.configured()?,
            "authenticated": authenticated,
            "model_loaded": self.engine.loaded(),
            "mcp": {
                "path": "/mcp",
                "configured": self.settings.mcp_token.is_some(),
            },
            "funclip_ready": self.subtitles.ready(),
            "limits": {
                "max_text_length": MAX_TEXT_LENGTH,
                "min_speed": 0.75,
                "max_speed": 1.25,
                "max_upload_bytes": MAX_UPLOAD_BYTES,
            },
        }))
    }

    pub fn list_speakers(&self) -> Result<Value> {
        let speakers = self.database.list_speakers()?;
        let mut out = Vec::new();
        for s in speakers {
            let profiles = self.database.list_profiles(&s.id)?;
            let profiles_json: Vec<Value> = profiles
                .into_iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "style_name": p.style_name,
                        "prompt_text": p.prompt_text,
                        "duration_seconds": p.duration_seconds,
                        "created_at": p.created_at,
                    })
                })
                .collect();
            out.push(json!({
                "id": s.id,
                "name": s.name,
                "created_at": s.created_at,
                "profiles": profiles_json,
            }));
        }
        Ok(json!({ "speakers": out }))
    }

    pub fn create_speaker(&self, name: &str) -> Result<Value> {
        let name = name.trim();
        if name.is_empty() || name.len() > 100 {
            bail!("Speaker name must contain 1 to 100 characters");
        }
        let id = Uuid::new_v4().to_string();
        self.database.insert_speaker(&id, name)?;
        Ok(json!({ "id": id, "name": name }))
    }

    pub fn delete_speaker(&self, speaker_id: &str) -> Result<()> {
        if self.database.speaker_has_profiles(speaker_id)? {
            return Err(StudioError::SpeakerHasProfiles.into());
        }
        if !self.database.delete_speaker(speaker_id)? {
            return Err(StudioError::SpeakerNotFound.into());
        }
        Ok(())
    }

    pub fn add_profile_from_file(
        &self,
        speaker_id: &str,
        style_name: &str,
        prompt_text: &str,
        source: &Path,
        consent: bool,
    ) -> Result<Value> {
        if !consent {
            return Err(StudioError::RightsRequired.into());
        }
        let style_name = style_name.trim();
        let prompt_text = prompt_text.trim();
        if style_name.is_empty() || style_name.len() > 100 {
            bail!("Style name must contain 1 to 100 characters");
        }
        if prompt_text.is_empty() || prompt_text.len() > 2000 {
            bail!("Exact transcript is required");
        }
        if self.database.speaker_by_id(speaker_id)?.is_none() {
            return Err(StudioError::SpeakerNotFound.into());
        }
        if !audio::extension_allowed(source) {
            return Err(StudioError::UnsupportedAudio.into());
        }
        let profile_id = Uuid::new_v4().to_string();
        let audio_name = format!("{}.wav", Uuid::new_v4());
        let destination = self.settings.profiles_dir().join(&audio_name);
        let duration = match convert_reference(source, &destination) {
            Ok(d) => d,
            Err(_) => return Err(StudioError::InvalidAudio.into()),
        };
        if let Err(e) = self.database.insert_profile(
            &profile_id,
            speaker_id,
            style_name,
            prompt_text,
            &audio_name,
            duration,
        ) {
            let _ = fs::remove_file(&destination);
            tracing::error!(error = %e, "Profile import failed");
            return Err(StudioError::ProfileFailed.into());
        }
        Ok(json!({
            "id": profile_id,
            "speaker_id": speaker_id,
            "style_name": style_name,
            "duration_seconds": duration,
        }))
    }

    pub fn add_profile_from_sandbox(
        &self,
        speaker_id: &str,
        style_name: &str,
        prompt_text: &str,
        audio_path: &str,
        confirm_rights: bool,
    ) -> Result<Value> {
        let root = self.settings.reference_input_dir.as_path();
        let source = resolve_under_root(audio_path, root).ok_or_else(|| {
            anyhow!("Audio must be inside the configured reference input directory")
        })?;
        self.add_profile_from_file(
            speaker_id,
            style_name,
            prompt_text,
            &source,
            confirm_rights,
        )
    }

    pub fn delete_profile(&self, profile_id: &str) -> Result<()> {
        let profile = self
            .database
            .profile_by_id(profile_id)?
            .ok_or(StudioError::ProfileNotFound)?;
        if self.database.profile_in_use(profile_id)? {
            return Err(StudioError::ProfileInUse.into());
        }
        let path = safe_owned_file(&self.settings.profiles_dir(), &profile.audio_name)
            .ok_or(StudioError::ProfileFileInvalid)?;
        self.database.delete_profile(profile_id)?;
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn generate_speech(
        &self,
        speaker_id: &str,
        profile_id: &str,
        target_text: &str,
        speed: f64,
    ) -> Result<Value> {
        let profile = self
            .database
            .profile_for_speaker(profile_id, speaker_id)?
            .ok_or(StudioError::InvalidProfile)?;
        let prompt_wav = safe_owned_file(&self.settings.profiles_dir(), &profile.audio_name)
            .ok_or(StudioError::ProfileFileInvalid)?;
        let generation_id = Uuid::new_v4().to_string();
        let audio_name = format!("{}.wav", Uuid::new_v4());
        self.database.insert_generation_running(
            &generation_id,
            speaker_id,
            profile_id,
            target_text,
            speed,
        )?;

        let generations = self.settings.generations_dir();
        let temporary = generations.join(format!(".generation-{}.wav", Uuid::new_v4()));
        let destination = generations.join(&audio_name);
        let mut published = false;

        let result = (|| {
            self.engine.generate(
                target_text,
                speed,
                &profile.prompt_text,
                &prompt_wav,
                &temporary,
            )?;
            let metadata = validate_generated_wav(&temporary)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
            }
            match fs::hard_link(&temporary, &destination) {
                Ok(()) => {}
                Err(_) => {
                    fs::copy(&temporary, &destination)?;
                }
            }
            published = true;
            let _ = fs::remove_file(&temporary);
            self.database
                .complete_generation(&generation_id, &audio_name)?;
            Ok(json!({
                "id": generation_id,
                "status": "complete",
                "audio_url": format!("/api/generations/{generation_id}/audio"),
                "audio_path": destination,
                "audio": metadata,
            }))
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            if published {
                let _ = fs::remove_file(&destination);
            }
            let _ = self.database.fail_generation(&generation_id);
            tracing::error!("Generation failed");
            return Err(StudioError::GenerationFailed.into());
        }
        result
    }

    pub fn get_generation(&self, generation_id: &str) -> Result<Value> {
        let row = self
            .database
            .generation_by_id(generation_id)?
            .ok_or(StudioError::GenerationNotFound)?;
        let mut result = json!({
            "id": row.id,
            "status": row.status,
            "audio_name": row.audio_name,
            "target_text": row.target_text,
            "speed": row.speed,
            "created_at": row.created_at,
        });
        if row.status == "complete" {
            if let Some(name) = &row.audio_name {
                let path = safe_owned_file(&self.settings.generations_dir(), name);
                result["audio_url"] = json!(format!("/api/generations/{generation_id}/audio"));
                result["audio_path"] = json!(path.map(|p| p.display().to_string()));
            }
        }
        Ok(result)
    }

    pub fn extract_subtitles(&self, video_path_raw: &str) -> Result<Value> {
        let root = self.settings.video_input_dir.as_path();
        let video_path = resolve_under_root(video_path_raw, root).ok_or_else(|| {
            anyhow!("Video must be inside the configured video input directory")
        })?;
        let (segments, srt) = self.subtitles.extract(&video_path)?;
        Ok(json!({
            "segments": segments,
            "srt": srt,
        }))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    #[error("speaker_has_profiles")]
    SpeakerHasProfiles,
    #[error("speaker_not_found")]
    SpeakerNotFound,
    #[error("rights_required")]
    RightsRequired,
    #[error("unsupported_audio")]
    UnsupportedAudio,
    #[error("invalid_audio")]
    InvalidAudio,
    #[error("profile_failed")]
    ProfileFailed,
    #[error("profile_not_found")]
    ProfileNotFound,
    #[error("profile_in_use")]
    ProfileInUse,
    #[error("profile_file_invalid")]
    ProfileFileInvalid,
    #[error("invalid_profile")]
    InvalidProfile,
    #[error("generation_failed")]
    GenerationFailed,
    #[error("generation_not_found")]
    GenerationNotFound,
}
