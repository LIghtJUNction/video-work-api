use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::alignment::WordTimestamp;
use crate::audio::{self, convert_reference, validate_generated_wav, MAX_UPLOAD_BYTES};
use crate::config::Settings;
use crate::database::Database;
use crate::engine::{GenerationMode, SpeechEngine};
use crate::model::{
    model_files_present, model_kind_files_present, translation_model_files_present, ModelKind,
};
use crate::paths::{resolve_under_root, safe_owned_file};
use crate::subtitles::{
    audio_transcription_extension_allowed, AsrModel, SubtitleExtractor, SubtitleSegment,
};
use crate::translation::{
    self, languages_payload, TranslationEngine, MAX_TRANSLATE_SEGMENTS, MAX_TRANSLATE_TEXT_CHARS,
    MAX_TRANSLATE_TOTAL_CHARS,
};
use crate::{MAX_TEXT_LENGTH, PRODUCT};

const XRY_MAX_CAPTION_CHARS: usize = 12;

type ExtractedSubtitleCopy = (
    Vec<SubtitleSegment>,
    String,
    Vec<WordTimestamp>,
    Option<String>,
);

pub struct Studio {
    pub settings: Settings,
    pub database: Database,
    pub engine: Arc<dyn SpeechEngine>,
    pub subtitles: Arc<dyn SubtitleExtractor>,
    pub translation: Arc<dyn TranslationEngine>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use tempfile::tempdir;

    use super::*;
    use crate::config::Settings;
    use crate::database::Database;
    use crate::engine::FakeEngine;
    use crate::subtitles::FakeSubtitles;
    use crate::translation::FakeTranslationEngine;

    fn studio(root: &Path) -> Studio {
        let settings = Settings {
            data_dir: root.join("data"),
            model_dir: root.join("model"),
            translation_model_dir: root.join("translation-model"),
            cosyvoice_root: root.join("source"),
            setup_token_file: root.join("setup-token"),
            host: "127.0.0.1".into(),
            port: 7860,
            ssl_certfile: None,
            ssl_keyfile: None,
            mcp_token: None,
            mcp_token_file: root.join("mcp-token"),
            mcp_token_source: None,
            funclip_root: None,
            video_input_dir: root.join("videos"),
            audio_input_dir: root.join("audio"),
            reference_input_dir: root.join("references"),
            video_projects_dir: root.join("video-projects"),
            receipt_key_file: root.join("receipt.key"),
            subtitle_timeout_seconds: 30,
            translation_timeout_seconds: 30,
            xry_task_root: root.join("xry-tasks"),
            xry_source_root: root.join("xry-sources"),
            xry_renderer: root.join("render.py"),
            xry_python: PathBuf::from("/usr/bin/python3").canonicalize().unwrap(),
            render_timeout_seconds: 30,
            video_project_renderer: root.join("video-project-render.py"),
            video_project_python: PathBuf::from("/usr/bin/python3").canonicalize().unwrap(),
            video_project_render_timeout_seconds: 30,
            project_root: root.to_path_buf(),
        };
        settings.create_data_dirs().unwrap();
        fs::create_dir_all(&settings.xry_source_root).unwrap();
        Studio::new(
            settings,
            Database::open(root.join("studio.sqlite3")).unwrap(),
            Arc::new(FakeEngine::new()),
            Arc::new(FakeSubtitles::default()),
            Arc::new(FakeTranslationEngine::new()),
        )
    }

    #[test]
    fn xry_subtitle_copy_rejects_mismatched_hash_and_cleans_up() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let source = studio.settings.xry_source_root.join("source.mov");
        fs::write(&source, b"frozen source bytes").unwrap();
        let error = studio
            .extract_subtitles_from_copy(&source, Some(&"0".repeat(64)))
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("source_sha256 does not match the copied"));
        let scratch = studio.settings.data_dir.join("subtitle-inputs");
        assert!(fs::read_dir(scratch).unwrap().next().is_none());
    }

    #[test]
    fn xry_subtitle_cache_key_rejects_path_traversal() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let source = studio.settings.xry_source_root.join("source.mov");
        fs::write(&source, b"frozen source bytes").unwrap();
        let error = studio
            .extract_subtitles(source.to_str().unwrap(), Some("../outside"))
            .unwrap_err();
        assert!(error.to_string().contains("source_sha256 must be"));
    }
}

fn parse_subtitle_timestamp(timestamp: &str) -> Result<f64> {
    let parts: Vec<&str> = timestamp.split(':').collect();
    if parts.len() != 3 {
        bail!("subtitle timestamp is malformed");
    }
    let hours: f64 = parts[0].parse()?;
    let minutes: f64 = parts[1].parse()?;
    let seconds: f64 = parts[2].replace(',', ".").parse()?;
    if !(0.0..60.0).contains(&minutes) || !(0.0..60.0).contains(&seconds) {
        bail!("subtitle timestamp is out of range");
    }
    Ok(hours * 3600.0 + minutes * 60.0 + seconds)
}

fn xry_caption_event_from_tokens(tokens: &[Value]) -> Result<(f64, f64, String, Vec<Value>)> {
    let start = tokens
        .first()
        .and_then(|token| token["start"].as_f64())
        .ok_or_else(|| anyhow!("word token start is missing"))?;
    let end = tokens
        .last()
        .and_then(|token| token["end"].as_f64())
        .ok_or_else(|| anyhow!("word token end is missing"))?;
    let zh = tokens
        .iter()
        .filter_map(|token| token["text"].as_str())
        .collect::<String>();
    if zh.is_empty() || !start.is_finite() || !end.is_finite() || end <= start {
        bail!("word-token caption event is invalid");
    }
    Ok((start, end, zh, tokens.to_vec()))
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("source_sha256 must be exactly 64 lowercase hexadecimal characters");
    }
    Ok(())
}

impl Studio {
    pub fn new(
        settings: Settings,
        database: Database,
        engine: Arc<dyn SpeechEngine>,
        subtitles: Arc<dyn SubtitleExtractor>,
        translation: Arc<dyn TranslationEngine>,
    ) -> Self {
        Self {
            settings,
            database,
            engine,
            subtitles,
            translation,
        }
    }

    pub fn status_payload(&self, authenticated: bool) -> Result<Value> {
        let model_present = model_files_present(&self.settings);
        let runtime_ready =
            python_runtime_ready(&self.settings) && self.settings.cosyvoice_root.is_dir();
        // `model_loaded` historically meant "engine has produced audio at least
        // once in this process" (lazy warm). Keep that meaning, and expose
        // clearer readiness fields for the UI.
        let model_warm = self.engine.loaded();
        let translation_present = translation_model_files_present(&self.settings);
        let translation_ready = self.translation.ready();
        let translation_loaded = self.translation.loaded();
        let voice_ready = model_present && runtime_ready;
        let (render_queued, render_running) = self.database.render_job_counts()?;
        let project_count = self.database.list_video_projects()?.len();
        Ok(json!({
            "product": PRODUCT,
            "service": "Video Work API",
            "status": "ready",
            "configured": self.database.configured()?,
            "authenticated": authenticated,
            "passkey_login_available": self.database.count_passkeys()? > 0,
            // Legacy top-level voice fields (unchanged contract).
            "model_present": model_present,
            "model_runtime_ready": runtime_ready,
            "model_ready": voice_ready,
            // Warm after first successful generation in this process.
            "model_loaded": model_warm,
            // Parallel multi-model view: same fields for every kind.
            "models": {
                "voice": {
                    "kind": ModelKind::Voice.as_str(),
                    "id": ModelKind::Voice.hub_id(),
                    "present": model_present,
                    "runtime_ready": runtime_ready,
                    "ready": voice_ready,
                    "loaded": model_warm,
                    "download_path": ModelKind::Voice.download_path(),
                },
                "translation": {
                    "kind": ModelKind::Translation.as_str(),
                    "id": ModelKind::Translation.hub_id(),
                    "present": translation_present,
                    "runtime_ready": runtime_ready,
                    "ready": translation_ready,
                    "loaded": translation_loaded,
                    "download_path": ModelKind::Translation.download_path(),
                },
            },
            "translation": {
                "model": ModelKind::Translation.hub_id(),
                "present": translation_present,
                "ready": translation_ready,
                "loaded": translation_loaded,
                "path": "/api/translate",
                "languages_path": "/api/translate/languages",
                "download_path": ModelKind::Translation.download_path(),
            },
            "mcp": {
                "path": "/mcp",
                "configured": self.settings.mcp_token.is_some(),
            },
            "funclip_ready": self.subtitles.ready(),
            "video_editor": {
                "path": "/api/editor",
                "projects_root_ready": self.settings.video_projects_dir.is_dir(),
                "projects": project_count,
                "canonical_file": "project.vpe",
            },
            "render_queue": {
                "ready": self.settings.xry_renderer.is_file()
                    && self.settings.xry_task_root.is_dir()
                    && self.settings.xry_source_root.is_dir(),
                "queued": render_queued,
                "running": render_running,
            },
            "limits": {
                "max_text_length": MAX_TEXT_LENGTH,
                "min_speed": 0.75,
                "max_speed": 1.25,
                "max_upload_bytes": MAX_UPLOAD_BYTES,
                "max_translate_text_chars": MAX_TRANSLATE_TEXT_CHARS,
                "max_translate_segments": MAX_TRANSLATE_SEGMENTS,
                "max_translate_total_chars": MAX_TRANSLATE_TOTAL_CHARS,
            },
        }))
    }

    pub fn list_translation_languages(&self) -> Value {
        let mut payload = languages_payload();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("ready".into(), json!(self.translation.ready()));
            obj.insert(
                "present".into(),
                json!(model_kind_files_present(
                    &self.settings,
                    ModelKind::Translation
                )),
            );
            obj.insert(
                "download_path".into(),
                json!(ModelKind::Translation.download_path()),
            );
        }
        payload
    }

    pub fn translate(
        &self,
        target_lang: &str,
        text: Option<&str>,
        texts: Option<&[String]>,
        srt: Option<&str>,
        segments: Option<&[SubtitleSegment]>,
    ) -> Result<Value> {
        translation::translate_request(
            self.translation.as_ref(),
            target_lang,
            text,
            texts,
            srt,
            segments,
        )
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

    pub fn rename_speaker(&self, speaker_id: &str, name: &str) -> Result<Value> {
        let name = name.trim();
        if name.is_empty() || name.len() > 100 {
            bail!("Speaker name must contain 1 to 100 characters");
        }
        let current = self
            .database
            .speaker_by_id(speaker_id)?
            .ok_or(StudioError::SpeakerNotFound)?;
        if current.name == name {
            return Ok(json!({ "id": current.id, "name": current.name }));
        }
        if let Some(other) = self.database.speaker_by_name(name)? {
            if other.id != speaker_id {
                return Err(StudioError::NameConflict.into());
            }
        }
        if !self.database.rename_speaker(speaker_id, name)? {
            return Err(StudioError::SpeakerNotFound.into());
        }
        Ok(json!({ "id": speaker_id, "name": name }))
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
        self.add_profile_from_file(speaker_id, style_name, prompt_text, &source, confirm_rights)
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

    pub fn rename_profile(&self, profile_id: &str, style_name: &str) -> Result<Value> {
        let style_name = style_name.trim();
        if style_name.is_empty() || style_name.len() > 100 {
            bail!("Style name must contain 1 to 100 characters");
        }
        let profile = self
            .database
            .profile_by_id(profile_id)?
            .ok_or(StudioError::ProfileNotFound)?;
        if profile.style_name == style_name {
            return Ok(json!({
                "id": profile.id,
                "speaker_id": profile.speaker_id,
                "style_name": profile.style_name,
            }));
        }
        if self
            .database
            .profile_exists_style(&profile.speaker_id, style_name)?
        {
            return Err(StudioError::NameConflict.into());
        }
        if !self.database.rename_profile_style(profile_id, style_name)? {
            return Err(StudioError::ProfileNotFound.into());
        }
        Ok(json!({
            "id": profile_id,
            "speaker_id": profile.speaker_id,
            "style_name": style_name,
        }))
    }

    pub fn generate_speech(
        &self,
        speaker_id: &str,
        profile_id: &str,
        target_text: &str,
        speed: f64,
        generation_mode: GenerationMode,
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
                generation_mode,
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
                "download_name": crate::filenames::download_name_from_text(target_text),
                "generation_mode": generation_mode,
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
            "download_name": crate::filenames::download_name_from_text(&row.target_text),
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

    pub fn extract_subtitles(
        &self,
        video_path_raw: &str,
        source_sha256: Option<&str>,
    ) -> Result<Value> {
        if let Some(source_sha256) = source_sha256 {
            validate_sha256(source_sha256)?;
        }
        let root = self.settings.video_input_dir.as_path();
        let video_path = resolve_under_root(video_path_raw, root)
            .or_else(|| {
                let path = Path::new(video_path_raw);
                let allowed = self.settings.xry_source_root.canonicalize().ok()?;
                let resolved = path.canonicalize().ok()?;
                (path.is_absolute()
                    && resolved.starts_with(&allowed)
                    && fs::symlink_metadata(path).ok()?.file_type().is_file()
                    && !fs::symlink_metadata(path).ok()?.file_type().is_symlink())
                .then_some(resolved)
            })
            .ok_or_else(|| {
                anyhow!(
                    "Video must be inside the configured video input directory or XRY source root"
                )
            })?;
        let cached = match source_sha256 {
            Some(expected) => {
                self.read_frozen_caption_cache(&video_path, expected, video_path_raw)?
            }
            None => None,
        };
        if let Some(payload) = cached {
            return Ok(payload);
        }
        let (segments, srt, words, actual_sha256) =
            self.extract_subtitles_from_copy(&video_path, source_sha256)?;
        if let Some(expected_sha256) = source_sha256 {
            let payload = self.xry_frozen_captions(
                video_path_raw,
                actual_sha256.as_deref().expect("source hash was verified"),
                expected_sha256,
                &segments,
                &words,
            )?;
            self.write_frozen_caption_cache(expected_sha256, &payload)?;
            return Ok(payload);
        }
        Ok(json!({
            "segments": segments,
            "srt": srt,
            "words": words,
        }))
    }

    fn xry_frozen_captions(
        &self,
        source_path: &str,
        actual_sha256: &str,
        expected_sha256: &str,
        segments: &[SubtitleSegment],
        words: &[WordTimestamp],
    ) -> Result<Value> {
        if actual_sha256 != expected_sha256 {
            bail!("source_sha256 does not match the resolved video file");
        }
        if segments.is_empty() || words.is_empty() {
            bail!("XRY frozen captions require non-empty subtitle segments and word timestamps");
        }
        if !self.translation.ready() {
            bail!("Translation model is not ready; cannot attest frozen XRY captions");
        }

        let mut event_bases = Vec::with_capacity(segments.len());
        for segment in segments {
            let start = parse_subtitle_timestamp(&segment.start)?;
            let end = parse_subtitle_timestamp(&segment.end)?;
            if !start.is_finite() || !end.is_finite() || end <= start {
                bail!("subtitle segment has an invalid timestamp range");
            }
            let tokens: Vec<Value> = words
                .iter()
                .filter(|word| word.start >= start && word.end <= end && word.end > word.start)
                .map(|word| json!({ "text": word.word, "start": word.start, "end": word.end }))
                .collect();
            if tokens.is_empty() {
                bail!("subtitle segment is missing trustworthy word timestamps");
            }
            let mut chunk = Vec::new();
            let mut chunk_chars = 0usize;
            for token in tokens {
                let token_chars = token["text"]
                    .as_str()
                    .map(str::chars)
                    .map(Iterator::count)
                    .unwrap_or(0);
                if !chunk.is_empty() && chunk_chars + token_chars > XRY_MAX_CAPTION_CHARS {
                    event_bases.push(xry_caption_event_from_tokens(&chunk)?);
                    chunk.clear();
                    chunk_chars = 0;
                }
                chunk_chars += token_chars;
                chunk.push(token);
            }
            if !chunk.is_empty() {
                event_bases.push(xry_caption_event_from_tokens(&chunk)?);
            }
        }

        let chinese: Vec<String> = event_bases.iter().map(|(_, _, zh, _)| zh.clone()).collect();
        let english = self.translation.translate_texts("en", &chinese)?;
        let russian = self.translation.translate_texts("ru", &chinese)?;
        if english.len() != event_bases.len() || russian.len() != event_bases.len() {
            bail!("Translation provider returned a different number of caption events");
        }

        let mut events = Vec::with_capacity(event_bases.len());
        for (index, (start, end, zh, tokens)) in event_bases.into_iter().enumerate() {
            events.push(json!({
                "start": start,
                "end": end,
                "zh": zh,
                "en": english[index],
                "ru": russian[index],
                "zh_tokens": tokens,
            }));
        }

        let receipt_material = serde_json::to_vec(&json!({
            "provider": "video-work-api/funclip+madlad400-3b-mt",
            "source_sha256": actual_sha256,
            "events": events,
        }))?;
        let receipt_hash = hex::encode(Sha256::digest(receipt_material));
        Ok(json!({
            "status": "PASS",
            "source": { "path": source_path, "sha256": expected_sha256 },
            "captions": { "events": events },
            "attestation": {
                "zh_en_faithful": true,
                "zh_ru_faithful": true,
                "same_semantic_events": true,
                "model_receipt_ref": format!("vwa-frozen-captions:{receipt_hash}"),
            },
        }))
    }

    fn extract_subtitles_from_copy(
        &self,
        video_path: &Path,
        expected_sha256: Option<&str>,
    ) -> Result<ExtractedSubtitleCopy> {
        let scratch_root = self.settings.data_dir.join("subtitle-inputs");
        fs::create_dir_all(&scratch_root)?;
        let extension = video_path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("mp4");
        let scratch_path = scratch_root.join(format!("{}.{}", Uuid::new_v4(), extension));
        let result = (|| {
            fs::copy(video_path, &scratch_path)?;
            let copied_sha256 = expected_sha256
                .map(|expected| {
                    let actual = crate::provenance::sha256_file(&scratch_path)?;
                    if actual != expected {
                        bail!("source_sha256 does not match the copied video file");
                    }
                    Ok(actual)
                })
                .transpose()?;
            let (segments, srt, words) = self
                .subtitles
                .extract(&scratch_path, AsrModel::Paraformer)?;
            Ok((segments, srt, words, copied_sha256))
        })();
        let _ = fs::remove_file(&scratch_path);
        result
    }

    fn frozen_caption_cache_path(&self, source_sha256: &str) -> PathBuf {
        self.settings
            .data_dir
            .join("xry-frozen-captions")
            .join(format!("v3-{source_sha256}.json"))
    }

    fn read_frozen_caption_cache(
        &self,
        video_path: &Path,
        expected_sha256: &str,
        source_path: &str,
    ) -> Result<Option<Value>> {
        if crate::provenance::sha256_file(video_path)? != expected_sha256 {
            bail!("source_sha256 does not match the resolved video file");
        }
        let path = self.frozen_caption_cache_path(expected_sha256);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut cached: Value = serde_json::from_slice(&bytes)?;
        if cached["status"] != "PASS"
            || cached["source"]["sha256"] != expected_sha256
            || !cached["captions"]["events"].is_array()
        {
            bail!("frozen caption cache is malformed");
        }
        cached["source"]["path"] = json!(source_path);
        Ok(Some(cached))
    }

    fn write_frozen_caption_cache(&self, source_sha256: &str, payload: &Value) -> Result<()> {
        let path = self.frozen_caption_cache_path(source_sha256);
        let parent = path.parent().expect("cache path has a parent");
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".{}.tmp", Uuid::new_v4()));
        fs::write(&temporary, serde_json::to_vec(payload)?)?;
        fs::rename(temporary, path)?;
        Ok(())
    }

    /// Extract subtitles from a server-created upload temp file (already trusted,
    /// so the video_input_dir sandbox does not apply).
    pub fn extract_subtitles_from_upload(&self, video_path: &Path) -> Result<Value> {
        let (segments, srt, words) = self.subtitles.extract(video_path, AsrModel::Paraformer)?;
        Ok(json!({
            "segments": segments,
            "srt": srt,
            "words": words,
        }))
    }

    /// Transcribe a recording kept under the dedicated recording input root.
    pub fn transcribe_audio(&self, audio_path_raw: &str, model: AsrModel) -> Result<Value> {
        let raw_path = Path::new(audio_path_raw);
        if raw_path.is_absolute()
            || raw_path
                .components()
                .any(|component| component == Component::ParentDir)
        {
            return Err(StudioError::InvalidAudioTranscriptionPath.into());
        }
        let audio_path =
            resolve_under_root(audio_path_raw, self.settings.audio_input_dir.as_path())
                .ok_or(StudioError::InvalidAudioTranscriptionPath)?;
        self.transcribe_audio_file(&audio_path, model)
    }

    /// Transcribe a server-created temporary upload after its extension has
    /// been retained in the generated filename.
    pub fn transcribe_audio_from_upload(&self, audio_path: &Path) -> Result<Value> {
        self.transcribe_audio_file(audio_path, AsrModel::Paraformer)
    }

    fn transcribe_audio_file(&self, audio_path: &Path, model: AsrModel) -> Result<Value> {
        if !audio_transcription_extension_allowed(audio_path) {
            return Err(StudioError::UnsupportedTranscriptionAudio.into());
        }
        let (segments, srt, words) = self.subtitles.extract(audio_path, model)?;
        // This is a stable, direct rendering of the segments returned by the
        // one FunClip/FunASR run; it intentionally performs no second inference.
        let text = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(json!({
            "text": text,
            "segments": segments,
            "srt": srt,
            "words": words,
        }))
    }
}

/// Python used for CosyVoice/FunClip helpers (venv preferred).
fn python_runtime_ready(settings: &Settings) -> bool {
    if let Ok(p) = std::env::var("VWA_PYTHON") {
        let path = std::path::PathBuf::from(p);
        if path.is_file() {
            return true;
        }
    }
    let venv = settings.data_dir.join(".venv/bin/python");
    venv.is_file()
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
    #[error("name_conflict")]
    NameConflict,
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
    #[error("invalid_audio_transcription_path")]
    InvalidAudioTranscriptionPath,
    #[error("unsupported_transcription_audio")]
    UnsupportedTranscriptionAudio,
}
