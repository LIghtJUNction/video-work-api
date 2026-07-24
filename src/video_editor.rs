use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::database::{PendingVideoProjectWrite, VideoProjectRow};
use crate::render_queue;
use crate::studio::{Studio, StudioError};
use crate::vpe::{self, VpeDocument};
use crate::{
    alignment, cover, lifecycle, provenance, quality, sampling, target_text_is_valid, timeline,
};

const PROJECT_FILE: &str = "project.vpe";
const PROJECT_LOCK_FILE: &str = ".editor.lock";
const MAX_PROJECT_BYTES: usize = 2 * 1024 * 1024;
const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;
const CREATE_INTENTS_DIR: &str = ".create-intents";
const CREATE_OWNER_FILE: &str = ".create-intent";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateIntent {
    id: String,
    slug: String,
    content: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum VideoEditorRequest {
    GetStatus {},
    ListSpeakers {},
    CreateSpeaker {
        name: String,
    },
    DeleteSpeaker {
        speaker_id: String,
    },
    RenameSpeaker {
        speaker_id: String,
        name: String,
    },
    AddVoiceProfile {
        speaker_id: String,
        style_name: String,
        prompt_text: String,
        audio_path: String,
        confirm_rights: bool,
    },
    DeleteVoiceProfile {
        profile_id: String,
    },
    RenameVoiceProfile {
        profile_id: String,
        style_name: String,
    },
    GenerateSpeech {
        speaker_id: String,
        profile_id: String,
        target_text: String,
        #[serde(default = "default_speed")]
        speed: f64,
    },
    GetGeneration {
        generation_id: String,
    },
    ExtractVideoSubtitles {
        video_path: String,
    },
    ListTranslationLanguages {},
    Translate {
        target_lang: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        texts: Option<Vec<String>>,
        #[serde(default)]
        srt: Option<String>,
        #[serde(default)]
        segments: Option<Vec<crate::subtitles::SubtitleSegment>>,
    },
    ListProjects {},
    CreateProject {
        slug: String,
        #[serde(default)]
        content: Option<String>,
    },
    GetTree {
        project: String,
    },
    ReadFile {
        project: String,
        path: String,
    },
    WriteFile {
        project: String,
        path: String,
        content: String,
        expected_revision: i64,
    },
    Validate {
        project: String,
    },
    AllocateVariantIds {
        namespace: String,
        #[serde(default = "default_variant_count")]
        count: u32,
        #[serde(default)]
        languages: Vec<String>,
    },
    Export {
        project: String,
    },
    GetJob {
        job_id: String,
    },
    CancelJob {
        job_id: String,
    },
    ExtractAnalysisFrames {
        request: sampling::ExtractAnalysisFramesRequest,
    },
    AnalyzeSafeTrims {
        request: alignment::AnalyzeSafeTrimsRequest,
    },
    ValidatePhase {
        project: String,
        request: Box<quality::ValidatePhaseRequest>,
    },
    RenderCover {
        project: String,
        variant_key: String,
        spec: cover::CoverSpec,
    },
    CleanupIntermediates {
        project: String,
        request: lifecycle::CleanupRequest,
    },
    ArchiveCompletedSources {
        project: String,
        request: lifecycle::ArchiveRequest,
    },
}

fn default_variant_count() -> u32 {
    1
}

fn default_speed() -> f64 {
    1.0
}

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<std::io::Error> for EditorError {
    fn from(error: std::io::Error) -> Self {
        Self::Internal(error.into())
    }
}

fn map_studio_editor(error: anyhow::Error) -> EditorError {
    if let Some(error) = error.downcast_ref::<StudioError>() {
        return match error {
            StudioError::SpeakerNotFound
            | StudioError::ProfileNotFound
            | StudioError::GenerationNotFound => EditorError::NotFound(error.to_string()),
            StudioError::SpeakerHasProfiles
            | StudioError::NameConflict
            | StudioError::ProfileInUse
            | StudioError::ProfileFileInvalid => EditorError::Conflict(error.to_string()),
            _ => EditorError::Invalid(error.to_string()),
        };
    }
    EditorError::Internal(error)
}

pub fn execute(studio: &Studio, request: VideoEditorRequest) -> Result<Value, EditorError> {
    match request {
        VideoEditorRequest::GetStatus {} => studio.status_payload(true).map_err(map_studio_editor),
        VideoEditorRequest::ListSpeakers {} => studio.list_speakers().map_err(map_studio_editor),
        VideoEditorRequest::CreateSpeaker { name } => {
            studio.create_speaker(&name).map_err(map_studio_editor)
        }
        VideoEditorRequest::DeleteSpeaker { speaker_id } => {
            studio
                .delete_speaker(&speaker_id)
                .map_err(map_studio_editor)?;
            Ok(json!({ "deleted": true, "speaker_id": speaker_id }))
        }
        VideoEditorRequest::RenameSpeaker { speaker_id, name } => studio
            .rename_speaker(&speaker_id, &name)
            .map_err(map_studio_editor),
        VideoEditorRequest::AddVoiceProfile {
            speaker_id,
            style_name,
            prompt_text,
            audio_path,
            confirm_rights,
        } => studio
            .add_profile_from_sandbox(
                &speaker_id,
                &style_name,
                &prompt_text,
                &audio_path,
                confirm_rights,
            )
            .map_err(map_studio_editor),
        VideoEditorRequest::DeleteVoiceProfile { profile_id } => {
            studio
                .delete_profile(&profile_id)
                .map_err(map_studio_editor)?;
            Ok(json!({ "deleted": true, "profile_id": profile_id }))
        }
        VideoEditorRequest::RenameVoiceProfile {
            profile_id,
            style_name,
        } => studio
            .rename_profile(&profile_id, &style_name)
            .map_err(map_studio_editor),
        VideoEditorRequest::GenerateSpeech {
            speaker_id,
            profile_id,
            target_text,
            speed,
        } => {
            let text = target_text.trim();
            if !target_text_is_valid(text) {
                return Err(EditorError::Invalid(
                    "Text must contain 1 to 1200 characters".into(),
                ));
            }
            if !(0.75..=1.25).contains(&speed) || !speed.is_finite() {
                return Err(EditorError::Invalid(
                    "Speed must be between 0.75 and 1.25".into(),
                ));
            }
            studio
                .generate_speech(&speaker_id, &profile_id, text, speed)
                .map_err(map_studio_editor)
        }
        VideoEditorRequest::GetGeneration { generation_id } => studio
            .get_generation(&generation_id)
            .map_err(map_studio_editor),
        VideoEditorRequest::ExtractVideoSubtitles { video_path } => studio
            .extract_subtitles(&video_path)
            .map_err(map_studio_editor),
        VideoEditorRequest::ListTranslationLanguages {} => {
            Ok(studio.list_translation_languages())
        }
        VideoEditorRequest::Translate {
            target_lang,
            text,
            texts,
            srt,
            segments,
        } => studio
            .translate(
                &target_lang,
                text.as_deref(),
                texts.as_deref(),
                srt.as_deref(),
                segments.as_deref(),
            )
            .map_err(map_studio_editor),
        VideoEditorRequest::ListProjects {} => list_projects(studio),
        VideoEditorRequest::CreateProject { slug, content } => {
            create_project(studio, &slug, content.as_deref())
        }
        VideoEditorRequest::GetTree { project } => get_tree(studio, &project),
        VideoEditorRequest::ReadFile { project, path } => read_file(studio, &project, &path),
        VideoEditorRequest::WriteFile {
            project,
            path,
            content,
            expected_revision,
        } => write_file(studio, &project, &path, &content, expected_revision),
        VideoEditorRequest::Validate { project } => validate(studio, &project),
        VideoEditorRequest::AllocateVariantIds {
            namespace,
            count,
            languages,
        } => studio
            .database
            .allocate_variant_ids(&namespace, count, &languages)
            .map(|ids| json!({ "namespace": namespace, "ids": ids }))
            .map_err(|error| EditorError::Invalid(error.to_string())),
        VideoEditorRequest::Export { project } => export(studio, &project),
        VideoEditorRequest::GetJob { job_id } => render_queue::get(studio, &job_id)
            .map_err(EditorError::Internal)?
            .ok_or_else(|| EditorError::NotFound("render job was not found".into())),
        VideoEditorRequest::CancelJob { job_id } => render_queue::cancel(studio, &job_id)
            .map_err(EditorError::Internal)?
            .ok_or_else(|| EditorError::NotFound("render job was not found".into())),
        VideoEditorRequest::ExtractAnalysisFrames { request } => {
            let source = request
                .validate(&studio.settings.video_input_dir)
                .map_err(|error| EditorError::Invalid(error.to_string()))?;
            let mut value = serde_json::to_value(&request).map_err(anyhow::Error::from)?;
            value["kind"] = json!("analysis_frames");
            render_queue::submit_media(
                studio,
                "analysis_frames",
                &request.video_path,
                &source,
                &value,
                None,
            )
            .map_err(EditorError::Internal)
        }
        VideoEditorRequest::AnalyzeSafeTrims { request } => {
            request
                .validate()
                .map_err(|error| EditorError::Invalid(error.to_string()))?;
            let source = sampling::resolve_media_input_no_symlink(
                &request.video_path,
                &studio.settings.video_input_dir,
            )
            .map_err(|error| EditorError::Invalid(error.to_string()))?;
            let mut value = serde_json::to_value(&request).map_err(anyhow::Error::from)?;
            value["kind"] = json!("safe_trims");
            render_queue::submit_media(
                studio,
                "safe_trims",
                &request.video_path,
                &source,
                &value,
                None,
            )
            .map_err(EditorError::Internal)
        }
        VideoEditorRequest::ValidatePhase { project, request } => {
            validate_phase(studio, &project, &request)
        }
        VideoEditorRequest::RenderCover {
            project,
            variant_key,
            spec,
        } => {
            let (directory, row) = load_project(studio, &project)?;
            let _project_lock = acquire_project_lock(&directory)?;
            let document = read_and_parse_current(&directory, &row)?;
            let variant = document
                .timeline
                .variants
                .iter()
                .enumerate()
                .find(|(index, variant)| timeline::variant_key(*index, variant) == variant_key)
                .map(|(_, variant)| variant.clone())
                .ok_or_else(|| {
                    EditorError::Invalid(
                        "variant_key is not declared by the current project revision".into(),
                    )
                })?;
            let stem = variant_key.clone();
            let source =
                cover::validate_render_cover(&spec, &variant_key, &studio.settings.video_input_dir)
                    .map_err(|error| EditorError::Invalid(error.to_string()))?;
            let value = json!({
                "kind": "cover",
                "stem": stem,
                "project_id": project,
                "project_revision": row.current_revision,
                "document_sha256": row.current_sha256,
                "variant_key": variant_key,
                "variant": variant,
                "spec": spec
            });
            render_queue::submit_media(
                studio,
                "cover",
                &format!(
                    "{}:{}:{}:{}",
                    project, row.current_revision, row.current_sha256, variant_key
                ),
                &source,
                &value,
                Some(&project),
            )
            .map_err(EditorError::Internal)
        }
        VideoEditorRequest::CleanupIntermediates { project, request } => {
            let (directory, _) = load_project(studio, &project)?;
            lifecycle::cleanup_intermediates(&directory, &request)
                .and_then(|result| serde_json::to_value(result).map_err(Into::into))
                .map_err(|error| EditorError::Invalid(error.to_string()))
        }
        VideoEditorRequest::ArchiveCompletedSources { project, request } => {
            let (directory, _) = load_project(studio, &project)?;
            lifecycle::archive_completed_sources(
                &directory,
                &request,
                &studio.settings.receipt_key_file,
            )
            .and_then(|result| serde_json::to_value(result).map_err(Into::into))
            .map_err(|error| EditorError::Invalid(error.to_string()))
        }
    }
}

fn validate_phase(
    studio: &Studio,
    project: &str,
    request: &quality::ValidatePhaseRequest,
) -> Result<Value, EditorError> {
    let (directory, row) = load_project(studio, project)?;
    let _project_lock = acquire_project_lock(&directory)?;
    let document = read_and_parse_current(&directory, &row)?;
    let chain = verify_phase_predecessors(
        &directory,
        request.phase,
        row.current_revision,
        project,
        &row.current_sha256,
        &studio.settings.receipt_key_file,
    )?;
    let attestation = if matches!(
        request.phase,
        quality::Phase::PrePackage | quality::Phase::Acceptance
    ) {
        let job_id = request
            .job_id
            .as_deref()
            .ok_or_else(|| EditorError::Invalid("job_id is required for rendered phases".into()))?;
        let job = studio
            .database
            .render_job_by_id(job_id)?
            .ok_or_else(|| EditorError::Invalid("render job was not found".into()))?;
        if job.kind != "video_project"
            || job.status != "succeeded"
            || job.project_id.as_deref() != Some(project)
            || job.project_revision != Some(row.current_revision)
            || job.document_sha.as_deref() != Some(row.current_sha256.as_str())
        {
            return Err(EditorError::Conflict(
                "render job is not a succeeded render for this exact project revision".into(),
            ));
        }
        let value = job.attestation_json.ok_or_else(|| {
            EditorError::Conflict("render job has no trusted worker attestation".into())
        })?;
        let attestation = serde_json::from_str::<quality::TrustedRenderAttestation>(&value)
            .map_err(|error| EditorError::Conflict(error.to_string()))?;
        let expected_output = directory.join("exports").join(job_id);
        let expected_prefix = format!("exports/{job_id}/");
        if job.output_dir.as_deref() != Some(expected_output.to_string_lossy().as_ref())
            || attestation.report_relative != format!("{expected_prefix}render-report.json")
            || attestation.master_relative != format!("{expected_prefix}master.mp4")
            || attestation.bundle_sha256 != job.snapshot_hash
            || !attestation.replay_verified
            || attestation.output_sha256 != attestation.replay_sha256
            || attestation.output_sha256.keys().any(|path| {
                !path.starts_with(&expected_prefix)
                    || !Path::new(path)
                        .components()
                        .all(|component| matches!(component, Component::Normal(_)))
            })
        {
            return Err(EditorError::Conflict(
                "stored render attestation is not authoritative for this job".into(),
            ));
        }
        Some(attestation)
    } else {
        None
    };
    let mut cover_attestations = std::collections::BTreeMap::new();
    if request.phase == quality::Phase::PrePackage {
        let mut job_ids = std::collections::BTreeSet::new();
        for (variant_key, job_id) in &request.cover_jobs {
            if !job_ids.insert(job_id) {
                return Err(EditorError::Conflict(
                    "each declared variant requires a unique cover job".into(),
                ));
            }
            let job = studio
                .database
                .render_job_by_id(job_id)?
                .ok_or_else(|| EditorError::Invalid("cover job was not found".into()))?;
            if job.kind != "cover"
                || job.status != "succeeded"
                || job.project_id.as_deref() != Some(project)
                || job.project_revision != Some(row.current_revision)
                || job.document_sha.as_deref() != Some(row.current_sha256.as_str())
            {
                return Err(EditorError::Conflict(
                    "cover job is foreign, stale, or not succeeded".into(),
                ));
            }
            let value = job.attestation_json.ok_or_else(|| {
                EditorError::Conflict("cover job has no trusted worker attestation".into())
            })?;
            let cover = serde_json::from_str::<quality::TrustedCoverAttestation>(&value).map_err(
                |error| EditorError::Conflict(format!("invalid cover attestation: {error}")),
            )?;
            if cover.variant_key != *variant_key {
                return Err(EditorError::Conflict(
                    "cover job was reused for a different variant key".into(),
                ));
            }
            cover_attestations.insert(variant_key.clone(), cover);
        }
    }
    let context = quality::GateContext {
        project_dir: &directory,
        document: &document,
        project_id: project,
        revision: row.current_revision,
        document_sha256: &row.current_sha256,
        request,
        attestation: attestation.as_ref(),
        cover_attestations: &cover_attestations,
    };
    let report = quality::Registry::built_in().validate(&context);
    let report_value = serde_json::to_value(&report).map_err(anyhow::Error::from)?;
    let report_bytes = serde_json::to_vec_pretty(&report_value).map_err(anyhow::Error::from)?;
    let report_sha256 = bytes_sha256(&report_bytes);
    if !report.passed {
        return Ok(json!({
            "project": project,
            "revision": row.current_revision,
            "report": report,
            "provenance": {
                "capability": "not-created",
                "reason": "failed validation is returned inline and does not consume the canonical PASS path",
                "validation_report_sha256": report_sha256,
            }
        }));
    }
    match provenance::signing_available(&studio.settings.receipt_key_file) {
        Ok(()) => {}
        Err(provenance::ReceiptError::CapabilityUnavailable) => {
            return Ok(json!({
                "project": project,
                "revision": row.current_revision,
                "report": report,
                "provenance": {
                    "capability": "unavailable",
                    "reason": "VWA_RECEIPT_KEY_FILE is not configured; canonical PASS evidence was not consumed",
                    "validation_report_sha256": report_sha256,
                }
            }));
        }
        Err(error) => return Err(EditorError::Invalid(error.to_string())),
    }
    let canonical_scope = format!("rev-{}/{}", row.current_revision, request.phase.as_str());
    let attempt_scope = format!(
        "rev-{}/attempt-{}-{}",
        row.current_revision,
        request.phase.as_str(),
        Uuid::new_v4()
    );
    let attempt_dir = provenance::prepare_receipt_scope(&directory, &attempt_scope)
        .map_err(|error| EditorError::Invalid(error.to_string()))?;
    let report_path = attempt_dir.join("validation_report.json");
    provenance::write_immutable(&report_path, &report_bytes)
        .map_err(|error| EditorError::Conflict(error.to_string()))?;
    let previous_receipt_hash = chain.last().map(|receipt| receipt.receipt_hash.clone());
    let source_hashes = request.input_manifest.clone();
    let output_hashes = report.artifact_sha256.clone();
    let mut bound_hashes = source_hashes.clone();
    bound_hashes.extend(output_hashes.clone());
    bound_hashes.insert(PROJECT_FILE.into(), row.current_sha256.clone());
    let canonical_params = json!({
        "phase": request.phase.as_str(),
        "project_id": project,
        "revision": row.current_revision,
        "document_sha256": row.current_sha256.clone(),
        "validation_report_sha256": report_sha256.clone(),
        "passed": true,
        "gate_results": report.checks.clone(),
        "source_sha256": source_hashes,
        "output_sha256": output_hashes,
        "previous_receipt_hash": previous_receipt_hash.clone(),
    });
    let receipt_result = provenance::create_receipt(
        &directory,
        &attempt_scope,
        bound_hashes,
        canonical_params,
        previous_receipt_hash.clone(),
        &studio.settings.receipt_key_file,
    );
    match receipt_result {
        Ok((receipt, _)) => {
            let canonical_dir = directory.join("receipts").join(&canonical_scope);
            reject_symlink_components(&canonical_dir, true)?;
            fs::rename(&attempt_dir, &canonical_dir)
                .map_err(|error| EditorError::Conflict(error.to_string()))?;
            sync_directory(
                canonical_dir
                    .parent()
                    .ok_or_else(|| anyhow!("canonical receipt has no parent"))?,
            )?;
            let receipt_path = canonical_dir.join("audit_receipt.json");
            Ok(json!({
                "project": project,
                "revision": row.current_revision,
                "report": report,
                "provenance": {
                    "capability": "available",
                    "receipt": receipt_path.strip_prefix(&directory)
                        .unwrap_or(&receipt_path).display().to_string(),
                    "receipt_hash": receipt.receipt_hash,
                }
            }))
        }
        Err(provenance::ReceiptError::CapabilityUnavailable) => Ok(json!({
            "project": project,
            "revision": row.current_revision,
            "report": report,
            "provenance": {
                "capability": "unavailable",
                "reason": "receipt signing key disappeared before canonical evidence was created",
                "validation_report_sha256": report_sha256,
            }
        })),
        Err(error) => Err(EditorError::Invalid(error.to_string())),
    }
}

fn verify_phase_predecessors(
    project_dir: &Path,
    phase: quality::Phase,
    revision: i64,
    project_id: &str,
    document_sha256: &str,
    key_file: &Path,
) -> Result<Vec<provenance::AuditReceipt>, EditorError> {
    let predecessors: &[quality::Phase] = match phase {
        quality::Phase::PreRender => &[],
        quality::Phase::PrePackage => &[quality::Phase::PreRender],
        quality::Phase::Acceptance => &[quality::Phase::PreRender, quality::Phase::PrePackage],
    };
    let paths = predecessors
        .iter()
        .map(|predecessor| {
            project_dir
                .join("receipts")
                .join(format!("rev-{revision}"))
                .join(predecessor.as_str())
                .join("audit_receipt.json")
        })
        .collect::<Vec<_>>();
    if paths.iter().any(|path| !path.exists()) {
        return Err(EditorError::Conflict(format!(
            "{} requires the complete prior phase chain",
            phase.as_str()
        )));
    }
    let receipts = provenance::verify_chain(&paths, key_file)
        .map_err(|error| EditorError::Invalid(error.to_string()))?;
    for (receipt, predecessor) in receipts.iter().zip(predecessors) {
        let params = &receipt.canonical_params;
        if params.get("phase").and_then(Value::as_str) != Some(predecessor.as_str())
            || params.get("project_id").and_then(Value::as_str) != Some(project_id)
            || params.get("revision").and_then(Value::as_i64) != Some(revision)
            || params.get("document_sha256").and_then(Value::as_str) != Some(document_sha256)
            || params.get("passed").and_then(Value::as_bool) != Some(true)
        {
            return Err(EditorError::Conflict(
                "prior receipt chain targets a different project revision".into(),
            ));
        }
        let report = project_dir
            .join("receipts")
            .join(format!("rev-{revision}"))
            .join(predecessor.as_str())
            .join("validation_report.json");
        provenance::verify_report_binding(receipt, &report)
            .map_err(|error| EditorError::Invalid(error.to_string()))?;
    }
    Ok(receipts)
}

fn list_projects(studio: &Studio) -> Result<Value, EditorError> {
    let root = secure_projects_root(studio)?;
    let _root_lock = acquire_project_lock(&root)?;
    recover_create_intents(studio, &root)?;
    recover_all_pending_writes(studio, &root)?;
    let projects = studio
        .database
        .list_video_projects()?
        .into_iter()
        .map(|project| {
            let directory = root.join(&project.id);
            let available = secure_existing_directory(&directory, &root).is_ok();
            json!({
                "slug": project.id,
                "revision": project.current_revision,
                "validated_revision": project.validated_revision,
                "valid": project.validated_revision == Some(project.current_revision),
                "available": available,
                "sha256": project.current_sha256,
                "created_at": project.created_at,
                "updated_at": project.updated_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "projects": projects }))
}

fn create_project(
    studio: &Studio,
    slug: &str,
    content: Option<&str>,
) -> Result<Value, EditorError> {
    validate_slug(slug)?;
    let root = secure_projects_root(studio)?;
    let _root_lock = acquire_project_lock(&root)?;
    recover_create_intents(studio, &root)?;
    recover_all_pending_writes(studio, &root)?;
    let project_dir = root.join(slug);
    match fs::symlink_metadata(&project_dir) {
        Ok(_) => {
            let directory = secure_existing_directory(&project_dir, &root)
                .map_err(|_| EditorError::Conflict("project path already exists".into()))?;
            let _project_lock = acquire_project_lock(&directory)?;
            recover_pending_write(studio, &directory, slug)?;
            return Err(EditorError::Conflict("project path already exists".into()));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let content = content
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_project(slug));
    validate_content_size(&content)?;
    let intent = begin_create_intent(&root, slug, &content)?;
    complete_create_intent(studio, &root, &intent).map(project_payload)
}

fn get_tree(studio: &Studio, project: &str) -> Result<Value, EditorError> {
    let (directory, row) = load_project(studio, project)?;
    let mut entries = Vec::new();
    collect_tree_entries(&directory, &directory, &mut entries)?;
    Ok(json!({
        "project": project,
        "revision": row.current_revision,
        "validated_revision": row.validated_revision,
        "entries": entries,
    }))
}

fn read_file(studio: &Studio, project: &str, path: &str) -> Result<Value, EditorError> {
    let (directory, row) = load_project(studio, project)?;
    let relative = safe_relative_path(path)?;
    if relative.components().next().is_some_and(|component| {
        component.as_os_str() == ".tmp" || component.as_os_str() == PROJECT_LOCK_FILE
    }) {
        return Err(EditorError::NotFound("virtual file was not found".into()));
    }
    let candidate = directory.join(&relative);
    reject_symlink_components(&candidate, false)?;
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("canonicalize virtual file {}", candidate.display()))
        .map_err(|_| EditorError::NotFound("virtual file was not found".into()))?;
    if !canonical.starts_with(&directory) || !canonical.is_file() {
        return Err(EditorError::NotFound("virtual file was not found".into()));
    }
    let metadata = canonical.metadata()?;
    if metadata.len() > MAX_READ_BYTES {
        return Err(EditorError::Invalid(
            "virtual file is too large to display".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(&canonical)?.read_to_end(&mut bytes)?;
    let content = String::from_utf8(bytes)
        .map_err(|_| EditorError::Invalid("virtual file is not UTF-8 text".into()))?;
    Ok(json!({
        "project": project,
        "path": relative_path_string(&relative),
        "content": content,
        "revision": row.current_revision,
        "read_only": relative_path_string(&relative) != PROJECT_FILE,
    }))
}

fn write_file(
    studio: &Studio,
    project: &str,
    path: &str,
    content: &str,
    expected_revision: i64,
) -> Result<Value, EditorError> {
    if path != PROJECT_FILE {
        return Err(EditorError::Invalid(
            "project.vpe is the only writable virtual file".into(),
        ));
    }
    validate_content_size(content)?;
    let (directory, _) = load_project(studio, project)?;
    let _project_lock = acquire_project_lock(&directory)?;
    recover_pending_write(studio, &directory, project)?;
    let current = studio
        .database
        .video_project(project)?
        .ok_or_else(|| EditorError::NotFound("video project was not found".into()))?;
    if current.current_revision != expected_revision {
        return Err(EditorError::Conflict(format!(
            "revision conflict: expected {expected_revision}, current revision is {}",
            current.current_revision
        )));
    }
    verify_current_file(&directory, &current)?;
    let sha256 = content_sha256(content);
    let staged_path = stage_project_file(&directory, content.as_bytes())?;
    let pending = studio
        .database
        .prepare_video_project_write(
            project,
            expected_revision,
            &current.current_sha256,
            &sha256,
            &staged_path,
            false,
        )
        .map_err(map_database_write_error)?;
    let row = publish_and_finalize(studio, &directory, &pending)?;
    Ok(project_payload(row))
}

fn validate(studio: &Studio, project: &str) -> Result<Value, EditorError> {
    let (directory, _) = load_project(studio, project)?;
    let _project_lock = acquire_project_lock(&directory)?;
    recover_pending_write(studio, &directory, project)?;
    let row = studio
        .database
        .video_project(project)?
        .ok_or_else(|| EditorError::NotFound("video project was not found".into()))?;
    let content = read_current_file(&directory)?;
    let sha256 = content_sha256(&content);
    if sha256 != row.current_sha256 {
        return Err(EditorError::Conflict(
            "project.vpe does not match the registered revision".into(),
        ));
    }
    let parsed = vpe::parse(&content).map_err(|error| EditorError::Invalid(error.to_string()))?;
    let row = studio
        .database
        .mark_video_project_validated(project, row.current_revision, &sha256)
        .map_err(map_database_write_error)?;
    Ok(json!({
        "project": project,
        "revision": row.current_revision,
        "valid": true,
        "sha256": row.current_sha256,
        "document": parsed,
    }))
}

fn export(studio: &Studio, project: &str) -> Result<Value, EditorError> {
    let (directory, _) = load_project(studio, project)?;
    let _project_lock = acquire_project_lock(&directory)?;
    recover_pending_write(studio, &directory, project)?;
    let row = studio
        .database
        .video_project(project)?
        .ok_or_else(|| EditorError::NotFound("video project was not found".into()))?;
    if row.validated_revision != Some(row.current_revision) {
        return Err(EditorError::Conflict(
            "the current revision must be validated before export".into(),
        ));
    }
    let document = read_and_parse_current(&directory, &row)?;
    let payload = render_queue::submit_video_project(
        studio,
        &directory,
        project,
        row.current_revision,
        &row.current_sha256,
        &document,
    )
    .map_err(|error| EditorError::Invalid(error.to_string()))?;
    Ok(json!({
        "project": project,
        "revision": row.current_revision,
        "document_sha256": row.current_sha256,
        "render": payload,
    }))
}

fn read_and_parse_current(
    directory: &Path,
    row: &VideoProjectRow,
) -> Result<VpeDocument, EditorError> {
    let content = read_current_file(directory)?;
    if content_sha256(&content) != row.current_sha256 {
        return Err(EditorError::Conflict(
            "project.vpe does not match the registered revision".into(),
        ));
    }
    vpe::parse(&content).map_err(|error| EditorError::Invalid(error.to_string()))
}

fn load_project(studio: &Studio, project: &str) -> Result<(PathBuf, VideoProjectRow), EditorError> {
    validate_slug(project)?;
    let root = secure_projects_root(studio)?;
    {
        let _root_lock = acquire_project_lock(&root)?;
        recover_create_intents(studio, &root)?;
    }
    let directory = secure_existing_directory(&root.join(project), &root)
        .map_err(|_| EditorError::NotFound("video project directory is unavailable".into()))?;
    {
        let _project_lock = acquire_project_lock(&directory)?;
        recover_pending_write(studio, &directory, project)?;
    }
    let row = studio
        .database
        .video_project(project)?
        .ok_or_else(|| EditorError::NotFound("video project was not found".into()))?;
    Ok((directory, row))
}

fn secure_projects_root(studio: &Studio) -> Result<PathBuf, EditorError> {
    let root = &studio.settings.video_projects_dir;
    if !root.is_absolute() {
        return Err(EditorError::Internal(anyhow!(
            "VWA_VIDEO_PROJECTS_DIR must be absolute"
        )));
    }
    reject_symlink_components(root, false)?;
    let canonical = root
        .canonicalize()
        .with_context(|| format!("canonicalize video projects root {}", root.display()))?;
    if !canonical.is_dir() {
        return Err(EditorError::Internal(anyhow!(
            "video projects root is not a directory"
        )));
    }
    Ok(canonical)
}

fn secure_existing_directory(path: &Path, root: &Path) -> anyhow::Result<PathBuf> {
    reject_symlink_components(path, false)?;
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root) || !canonical.is_dir() {
        bail!("project directory escaped the configured root");
    }
    Ok(canonical)
}

fn safe_relative_path(path: &str) -> Result<PathBuf, EditorError> {
    let relative = Path::new(path);
    if path.is_empty() || relative.is_absolute() {
        return Err(EditorError::Invalid(
            "virtual path must be a non-empty relative path".into(),
        ));
    }
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(EditorError::Invalid(
            "virtual path cannot contain '.', '..', roots, or prefixes".into(),
        ));
    }
    Ok(relative.to_path_buf())
}

fn reject_symlink_components(path: &Path, allow_missing_final: bool) -> anyhow::Result<()> {
    if !path.is_absolute() {
        bail!("sandbox path must be absolute");
    }
    let component_count = path.components().count();
    let mut current = PathBuf::new();
    for (index, component) in path.components().enumerate() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            bail!("sandbox path contains a non-canonical component");
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("sandbox path contains symlink component");
            }
            Ok(_) => {}
            Err(error)
                if allow_missing_final
                    && index + 1 == component_count
                    && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn collect_tree_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<Value>,
) -> Result<(), EditorError> {
    let mut children = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(EditorError::Conflict(
                "virtual project contains a symlink".into(),
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| anyhow!("tree entry escaped project root"))?;
        let relative_string = relative_path_string(relative);
        if relative_string == ".tmp" || relative_string == PROJECT_LOCK_FILE {
            continue;
        }
        if metadata.is_dir() {
            entries.push(json!({
                "path": relative_string,
                "kind": "directory",
                "read_only": true,
            }));
            collect_tree_entries(root, &path, entries)?;
        } else if metadata.is_file() {
            entries.push(json!({
                "path": relative_string,
                "kind": "file",
                "read_only": relative_string != PROJECT_FILE,
                "size": metadata.len(),
            }));
        } else {
            return Err(EditorError::Conflict(
                "virtual project contains an unsupported filesystem entry".into(),
            ));
        }
    }
    Ok(())
}

fn verify_current_file(directory: &Path, row: &VideoProjectRow) -> Result<(), EditorError> {
    let content = read_current_file(directory)?;
    if content_sha256(&content) != row.current_sha256 {
        return Err(EditorError::Conflict(
            "project.vpe does not match the registered revision".into(),
        ));
    }
    Ok(())
}

fn read_current_file(directory: &Path) -> Result<String, EditorError> {
    let path = directory.join(PROJECT_FILE);
    reject_symlink_components(&path, false)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() > MAX_PROJECT_BYTES as u64 {
        return Err(EditorError::Invalid("project.vpe is invalid".into()));
    }
    fs::read_to_string(path).map_err(Into::into)
}

fn create_intents_directory(root: &Path) -> Result<PathBuf, EditorError> {
    let directory = root.join(CREATE_INTENTS_DIR);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(EditorError::Conflict(
                "create intent root is not a safe directory".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_private_directory(&directory)?;
            sync_directory(root)?;
        }
        Err(error) => return Err(error.into()),
    }
    secure_existing_directory(&directory, root).map_err(EditorError::Internal)
}

fn begin_create_intent(
    root: &Path,
    slug: &str,
    content: &str,
) -> Result<CreateIntent, EditorError> {
    let intents = create_intents_directory(root)?;
    let intent = CreateIntent {
        id: Uuid::new_v4().to_string(),
        slug: slug.to_string(),
        content: content.to_string(),
        sha256: content_sha256(content),
    };
    let bytes = serde_json::to_vec(&intent).map_err(anyhow::Error::from)?;
    let path = intents.join(format!("{}.json", intent.id));
    write_immutable(&path, &bytes)?;
    sync_directory(&intents)?;
    Ok(intent)
}

fn recover_create_intents(studio: &Studio, root: &Path) -> Result<(), EditorError> {
    let intents = create_intents_directory(root)?;
    let mut paths = fs::read_dir(&intents)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        reject_symlink_components(&path, false)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 3_000_000 {
            return Err(EditorError::Conflict(
                "create intent is not a bounded regular file".into(),
            ));
        }
        let intent: CreateIntent = serde_json::from_slice(&fs::read(&path)?)
            .map_err(|error| EditorError::Conflict(error.to_string()))?;
        complete_create_intent(studio, root, &intent)?;
    }
    // A crash after the intent was durably removed but before its owner marker
    // was removed leaves a completed, registered project. Clean only that exact
    // proven state; unknown directories remain fail-closed.
    for row in studio.database.list_video_projects()? {
        let project = match secure_existing_directory(&root.join(&row.id), root) {
            Ok(project) => project,
            Err(_) => continue,
        };
        let owner = project.join(CREATE_OWNER_FILE);
        if !owner.exists() {
            continue;
        }
        reject_symlink_components(&owner, false)?;
        if read_current_file(&project)
            .map(|content| content_sha256(&content) == row.current_sha256)
            .unwrap_or(false)
        {
            fs::remove_file(&owner)?;
            sync_directory(&project)?;
        }
    }
    Ok(())
}

fn complete_create_intent(
    studio: &Studio,
    root: &Path,
    intent: &CreateIntent,
) -> Result<VideoProjectRow, EditorError> {
    validate_slug(&intent.slug)?;
    validate_content_size(&intent.content)?;
    if content_sha256(&intent.content) != intent.sha256 {
        return Err(EditorError::Conflict(
            "create intent content hash mismatch".into(),
        ));
    }
    let intents = create_intents_directory(root)?;
    let intent_path = intents.join(format!("{}.json", intent.id));
    let staging = intents.join(format!("{}.project", intent.id));
    let project = root.join(&intent.slug);
    if !project.exists() {
        repair_create_staging(&staging, &intents, &intent.id)?;
        fs::rename(&staging, &project)?;
        sync_directory(root)?;
    }
    let project = secure_existing_directory(&project, root)
        .map_err(|_| EditorError::Conflict("intent-owned project directory is unsafe".into()))?;
    let owner = project.join(CREATE_OWNER_FILE);
    reject_symlink_components(&owner, false)?;
    if fs::read_to_string(&owner)? != intent.id {
        return Err(EditorError::Conflict(
            "project directory is not owned by its create intent".into(),
        ));
    }
    let _project_lock = acquire_project_lock(&project)?;
    let row = match studio.database.video_project(&intent.slug)? {
        Some(row) => {
            if row.current_sha256 != intent.sha256 {
                return Err(EditorError::Conflict(
                    "created project database content conflicts with intent".into(),
                ));
            }
            row
        }
        None => {
            let staged_path = stage_project_file(&project, intent.content.as_bytes())?;
            let pending = studio
                .database
                .prepare_video_project_write(
                    &intent.slug,
                    0,
                    "",
                    &intent.sha256,
                    &staged_path,
                    true,
                )
                .map_err(map_database_write_error)?;
            publish_and_finalize(studio, &project, &pending)?
        }
    };
    fs::remove_file(&intent_path)?;
    sync_directory(&intents)?;
    fs::remove_file(&owner)?;
    sync_directory(&project)?;
    Ok(row)
}

fn repair_create_staging(
    staging: &Path,
    intents: &Path,
    intent_id: &str,
) -> Result<(), EditorError> {
    const DIRECTORIES: [&str; 5] = [".history", ".tmp", "assets", "receipts", "exports"];
    match fs::symlink_metadata(staging) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(EditorError::Conflict(
                "create staging path is not a safe directory".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_private_directory(staging)?;
            sync_directory(intents)?;
        }
        Err(error) => return Err(error.into()),
    }
    reject_symlink_components(staging, false)?;
    let allowed = DIRECTORIES
        .into_iter()
        .chain([CREATE_OWNER_FILE])
        .collect::<std::collections::BTreeSet<_>>();
    for entry in fs::read_dir(staging)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| EditorError::Conflict("create staging entry is not UTF-8".into()))?;
        if !allowed.contains(name.as_str()) {
            return Err(EditorError::Conflict(format!(
                "unexpected create staging entry '{name}'"
            )));
        }
    }
    for name in DIRECTORIES {
        let child = staging.join(name);
        match fs::symlink_metadata(&child) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(EditorError::Conflict(format!(
                    "create staging child '{name}' is unsafe"
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                create_private_directory(&child)?;
                sync_directory(staging)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let owner = staging.join(CREATE_OWNER_FILE);
    match fs::symlink_metadata(&owner) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(EditorError::Conflict(
                "create staging owner marker is unsafe".into(),
            ));
        }
        Ok(_) if fs::read(&owner)? != intent_id.as_bytes() => {
            return Err(EditorError::Conflict(
                "create staging owner marker conflicts with intent".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            write_immutable(&owner, intent_id.as_bytes())?;
        }
        Err(error) => return Err(error.into()),
    }
    sync_directory(staging)?;
    sync_directory(intents)?;
    Ok(())
}

fn recover_all_pending_writes(studio: &Studio, root: &Path) -> Result<(), EditorError> {
    for pending in studio.database.list_pending_video_project_writes()? {
        validate_slug(&pending.project_id)?;
        let directory =
            secure_existing_directory(&root.join(&pending.project_id), root).map_err(|_| {
                EditorError::Conflict(format!(
                    "pending write for '{}' has no secure project directory",
                    pending.project_id
                ))
            })?;
        let _project_lock = acquire_project_lock(&directory)?;
        recover_pending_write(studio, &directory, &pending.project_id)?;
    }
    Ok(())
}

fn recover_pending_write(
    studio: &Studio,
    project_dir: &Path,
    project: &str,
) -> Result<(), EditorError> {
    if let Some(pending) = studio.database.pending_video_project_write(project)? {
        publish_and_finalize(studio, project_dir, &pending)?;
    }
    Ok(())
}

fn publish_and_finalize(
    studio: &Studio,
    project_dir: &Path,
    pending: &PendingVideoProjectWrite,
) -> Result<VideoProjectRow, EditorError> {
    let staged_path = pending_staged_path(project_dir, &pending.staged_path)?;
    let history_dir = secure_existing_directory(&project_dir.join(".history"), project_dir)?;
    let history_path = history_dir.join(format!(
        "{:06}-{}.vpe",
        pending.new_revision, pending.new_sha
    ));
    let bytes = recover_pending_bytes(project_dir, &staged_path, &history_path, &pending.new_sha)?;
    publish_history(&history_path, &bytes, &pending.new_sha)?;
    publish_current(project_dir, &bytes, &pending.new_sha)?;
    let row = studio
        .database
        .finalize_video_project_write(pending)
        .map_err(map_database_write_error)?;
    match fs::remove_file(&staged_path) {
        Ok(()) => sync_directory(
            staged_path
                .parent()
                .ok_or_else(|| anyhow!("staged path has no parent"))?,
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(row)
}

fn stage_project_file(project_dir: &Path, bytes: &[u8]) -> anyhow::Result<String> {
    let temp_directory = secure_existing_directory(&project_dir.join(".tmp"), project_dir)?;
    let name = format!("write-{}.vpe", Uuid::new_v4());
    let path = temp_directory.join(&name);
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(&path)
        .with_context(|| format!("create staged project file {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    sync_directory(&temp_directory)?;
    Ok(format!(".tmp/{name}"))
}

fn pending_staged_path(project_dir: &Path, relative: &str) -> Result<PathBuf, EditorError> {
    let path = Path::new(relative);
    let components = path.components().collect::<Vec<_>>();
    if components.len() != 2
        || components[0].as_os_str() != ".tmp"
        || !matches!(components[1], Component::Normal(_))
    {
        return Err(EditorError::Conflict(
            "pending video project write has an invalid staged path".into(),
        ));
    }
    let absolute = project_dir.join(path);
    reject_symlink_components(&absolute, true)?;
    Ok(absolute)
}

fn recover_pending_bytes(
    project_dir: &Path,
    staged_path: &Path,
    history_path: &Path,
    expected_sha: &str,
) -> Result<Vec<u8>, EditorError> {
    for candidate in [
        staged_path.to_path_buf(),
        history_path.to_path_buf(),
        project_dir.join(PROJECT_FILE),
    ] {
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(EditorError::Conflict(format!(
                    "pending write recovery candidate is unsafe: {}",
                    candidate.display()
                )));
            }
            Ok(metadata) if metadata.len() <= MAX_PROJECT_BYTES as u64 => {
                let bytes = fs::read(&candidate)?;
                if bytes_sha256(&bytes) == expected_sha {
                    return Ok(bytes);
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(EditorError::Conflict(
        "pending video project write cannot be recovered from staged, history, or current content"
            .into(),
    ))
}

fn publish_history(path: &Path, bytes: &[u8], expected_sha: &str) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || bytes_sha256(&fs::read(path)?) != expected_sha
            {
                bail!("history revision exists with conflicting content");
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => write_immutable(path, bytes),
        Err(error) => Err(error.into()),
    }
}

fn publish_current(project_dir: &Path, bytes: &[u8], expected_sha: &str) -> anyhow::Result<()> {
    let path = project_dir.join(PROJECT_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && bytes_sha256(&fs::read(&path)?) == expected_sha =>
        {
            sync_directory(project_dir)
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            bail!("project.vpe is not a safe regular file")
        }
        Ok(_) => write_atomic(&path, bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => write_atomic(&path, bytes),
        Err(error) => Err(error.into()),
    }
}

fn write_immutable(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    reject_symlink_components(path, true)?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o400).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("create immutable history {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    }
    sync_directory(
        path.parent()
            .ok_or_else(|| anyhow!("history path has no parent"))?,
    )
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("project file has no parent"))?;
    secure_existing_directory(parent, parent)?;
    let temp_directory = secure_existing_directory(&parent.join(".tmp"), parent)?;
    let temporary = temp_directory.join(format!("project.vpe.{}.tmp", Uuid::new_v4()));
    let mut cleanup = TemporaryFile {
        path: temporary.clone(),
        active: true,
    };
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("create private project temp {}", temporary.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)
        .with_context(|| format!("atomically replace {}", path.display()))?;
    cleanup.active = false;
    sync_directory(parent)
}

struct TemporaryFile {
    path: PathBuf,
    active: bool,
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.active {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn sync_directory(directory: &Path) -> anyhow::Result<()> {
    File::open(directory)?.sync_all().map_err(Into::into)
}

struct ProjectLock {
    _file: File,
}

#[cfg(unix)]
fn acquire_project_lock(directory: &Path) -> Result<ProjectLock, EditorError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let path = directory.join(PROJECT_LOCK_FILE);
    reject_symlink_components(&path, true)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .with_context(|| format!("open project lock {}", path.display()))?;
    // SAFETY: the descriptor is valid and remains owned by ProjectLock until
    // the operation finishes, at which point the OS releases the lock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == -1 {
        return Err(EditorError::Internal(
            std::io::Error::last_os_error().into(),
        ));
    }
    Ok(ProjectLock { _file: file })
}

#[cfg(not(unix))]
fn acquire_project_lock(_directory: &Path) -> Result<ProjectLock, EditorError> {
    Err(EditorError::Internal(anyhow!(
        "project locking is unavailable on this platform"
    )))
}

fn create_private_directory(path: &Path) -> anyhow::Result<()> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .with_context(|| format!("create private directory {}", path.display()))
}

fn validate_slug(slug: &str) -> Result<(), EditorError> {
    if slug.is_empty()
        || slug.len() > 64
        || slug.starts_with('-')
        || slug.ends_with('-')
        || !slug.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(EditorError::Invalid(
            "project slug must use 1 to 64 lowercase ASCII letters, digits, or internal hyphens"
                .into(),
        ));
    }
    Ok(())
}

fn validate_content_size(content: &str) -> Result<(), EditorError> {
    if content.len() > MAX_PROJECT_BYTES {
        return Err(EditorError::Invalid(format!(
            "project.vpe must not exceed {MAX_PROJECT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn content_sha256(content: &str) -> String {
    bytes_sha256(content.as_bytes())
}

fn bytes_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn relative_path_string(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn project_payload(row: VideoProjectRow) -> Value {
    json!({
        "project": row.id,
        "revision": row.current_revision,
        "validated_revision": row.validated_revision,
        "valid": row.validated_revision == Some(row.current_revision),
        "sha256": row.current_sha256,
    })
}

fn map_database_write_error(error: anyhow::Error) -> EditorError {
    let message = error.to_string();
    if message.contains("already exists") || message.contains("revision conflict") {
        EditorError::Conflict(message)
    } else if message.contains("was not found") {
        EditorError::NotFound(message)
    } else {
        EditorError::Internal(error)
    }
}

fn default_project(slug: &str) -> String {
    format!(
        r#"project "{slug}" {{
  canvas 1080x1920 @ 30fps
  source main = "assets/source.mp4"

  timeline {{
    track main {{
      clip main source 00:00:00.000..00:00:01.000 at 00:00:00.000
    }}
  }}
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;
    use crate::config::Settings;
    use crate::database::Database;
    use crate::engine::FakeEngine;
    use crate::subtitles::FakeSubtitles;
    use crate::translation::FakeTranslationEngine;

    fn studio(root: &Path) -> Studio {
        let settings = Settings {
            data_dir: root.to_path_buf(),
            model_dir: root.join("model"),
        translation_model_dir: root.join("translation-model"),
            cosyvoice_root: root.join("source"),
            setup_token_file: root.join("setup-token"),
            host: "127.0.0.1".into(),
            port: 7860,
            ssl_certfile: None,
            ssl_keyfile: None,
            mcp_token: Some("token".into()),
            mcp_token_file: root.join("mcp-token"),
            mcp_token_source: None,
            funclip_root: None,
            video_input_dir: root.join("videos"),
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
            video_project_renderer: root.join("video_project_render.py"),
            video_project_python: PathBuf::from("/usr/bin/python3").canonicalize().unwrap(),
            video_project_render_timeout_seconds: 30,
            project_root: root.to_path_buf(),
        };
        settings.create_data_dirs().unwrap();
        Studio::new(
            settings,
            Database::open(root.join("studio.sqlite3")).unwrap(),
            Arc::new(FakeEngine::new()),
            Arc::new(FakeSubtitles::default()),
            Arc::new(FakeTranslationEngine::new()),
        )
    }

    fn export_project() -> &'static str {
        r#"project "Export Project" {
  canvas 1080x1920 @ 30fps
  source main = "assets/source.mp4"
  timeline {
    track main {
      clip main source 00:00:00.000..00:00:04.000 at 00:00:00.000
    }
  }
  marker "Opening hook" at 00:00:03.000
  gate pre_render require opening_hook, continuous_timeline
  export xry(task_dir = "campaign/batch-01", subject_id = "S01", encoder_profile = "formal-auto")
}
"#
    }

    fn quality_project() -> &'static str {
        r#"project "Quality Project" {
  canvas 1080x1920 @ 30fps
  source main = "assets/source.mp4"
  timeline {
    track main {
      clip main source 00:00:00.000..00:00:04.000 at 00:00:00.000
    }
  }
  marker "Opening hook" at 00:00:03.000
  variant "ZH-EN" aspect 9:16 subtitles "subs.zh-en.ass"
}
"#
    }

    fn prepare_video_project_render(studio: &mut Studio, project: &str) {
        fs::write(
            &studio.settings.video_project_renderer,
            include_bytes!("../scripts/video_project_render.py"),
        )
        .unwrap();
        studio.settings.video_project_python =
            PathBuf::from("/usr/bin/python3").canonicalize().unwrap();
        let source = studio
            .settings
            .video_projects_dir
            .join(project)
            .join("assets/source.mp4");
        let status = std::process::Command::new("/usr/bin/ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "4.1",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-y",
            ])
            .arg(source)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn revisions_are_optimistic_and_history_is_hash_named() {
        let directory = tempdir().unwrap();
        let studio = studio(directory.path());
        let created = create_project(&studio, "aurora-launch", None).unwrap();
        assert_eq!(created["revision"], 1);
        let initial = read_file(&studio, "aurora-launch", PROJECT_FILE).unwrap();
        let content = initial["content"]
            .as_str()
            .unwrap()
            .replace("project \"aurora-launch\"", "project \"Aurora Launch\"");
        let written = write_file(&studio, "aurora-launch", PROJECT_FILE, &content, 1).unwrap();
        assert_eq!(written["revision"], 2);
        assert!(matches!(
            write_file(&studio, "aurora-launch", PROJECT_FILE, &content, 1),
            Err(EditorError::Conflict(_))
        ));
        let history = fs::read_dir(
            studio
                .settings
                .video_projects_dir
                .join("aurora-launch/.history"),
        )
        .unwrap()
        .count();
        assert_eq!(history, 2);
    }

    #[test]
    fn root_create_intent_recovers_before_slug_directory_creation() {
        let directory = tempdir().unwrap();
        let first = studio(directory.path());
        let root = secure_projects_root(&first).unwrap();
        let content = default_project("crash-create");
        let intent = begin_create_intent(&root, "crash-create", &content).unwrap();
        assert!(!root.join("crash-create").exists());
        assert!(root
            .join(CREATE_INTENTS_DIR)
            .join(format!("{}.json", intent.id))
            .is_file());
        drop(first);

        let reopened = studio(directory.path());
        let projects = list_projects(&reopened).unwrap();
        assert_eq!(projects["projects"][0]["slug"], "crash-create");
        assert!(root.join("crash-create/project.vpe").is_file());
        assert!(!root
            .join(CREATE_INTENTS_DIR)
            .join(format!("{}.json", intent.id))
            .exists());
    }

    #[test]
    fn create_intent_repairs_every_partial_staging_boundary() {
        let directory = tempdir().unwrap();
        let initial = studio(directory.path());
        let root = secure_projects_root(&initial).unwrap();
        let children = [".history", ".tmp", "assets", "receipts", "exports"];
        for completed in 0..=children.len() {
            let slug = format!("partial-create-{completed}");
            let content = default_project(&slug);
            let intent = begin_create_intent(&root, &slug, &content).unwrap();
            let intents = create_intents_directory(&root).unwrap();
            let staging = intents.join(format!("{}.project", intent.id));
            create_private_directory(&staging).unwrap();
            for name in children.iter().take(completed) {
                create_private_directory(&staging.join(name)).unwrap();
            }
            if completed == children.len() {
                write_immutable(&staging.join(CREATE_OWNER_FILE), intent.id.as_bytes()).unwrap();
            }
        }
        drop(initial);

        let reopened = studio(directory.path());
        let projects = list_projects(&reopened).unwrap();
        assert_eq!(
            projects["projects"].as_array().unwrap().len(),
            children.len() + 1
        );
        for completed in 0..=children.len() {
            let project = root.join(format!("partial-create-{completed}"));
            assert!(project.join(PROJECT_FILE).is_file());
            assert!(children.iter().all(|name| project.join(name).is_dir()));
            assert!(!project.join(CREATE_OWNER_FILE).exists());
        }
    }

    #[test]
    fn concurrent_writers_commit_exactly_one_revision() {
        let directory = tempdir().unwrap();
        let studio = Arc::new(studio(directory.path()));
        create_project(&studio, "concurrent-project", None).unwrap();
        let handles = ["First", "Second"].map(|name| {
            let studio = studio.clone();
            std::thread::spawn(move || {
                let initial = read_file(&studio, "concurrent-project", PROJECT_FILE).unwrap();
                let content = initial["content"].as_str().unwrap().replace(
                    "project \"concurrent-project\"",
                    &format!("project \"{name}\""),
                );
                write_file(&studio, "concurrent-project", PROJECT_FILE, &content, 1)
            })
        });
        let results = handles.map(|handle| handle.join().unwrap());
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(EditorError::Conflict(_))))
                .count(),
            1
        );
        assert_eq!(
            studio
                .database
                .video_project("concurrent-project")
                .unwrap()
                .unwrap()
                .current_revision,
            2
        );
    }

    fn prepare_pending_revision(
        studio: &Studio,
        project: &str,
        project_name: &str,
    ) -> (PathBuf, PendingVideoProjectWrite, String) {
        let directory = studio.settings.video_projects_dir.join(project);
        let current = studio.database.video_project(project).unwrap().unwrap();
        let content = read_current_file(&directory).unwrap().replace(
            &format!("project \"{project}\""),
            &format!("project \"{project_name}\""),
        );
        let sha = content_sha256(&content);
        let staged = stage_project_file(&directory, content.as_bytes()).unwrap();
        let pending = studio
            .database
            .prepare_video_project_write(
                project,
                current.current_revision,
                &current.current_sha256,
                &sha,
                &staged,
                false,
            )
            .unwrap();
        (directory, pending, content)
    }

    #[test]
    fn orphan_stage_before_intent_does_not_change_revision() {
        let root = tempdir().unwrap();
        let first_studio = studio(root.path());
        create_project(&first_studio, "stage-only", None).unwrap();
        let directory = first_studio.settings.video_projects_dir.join("stage-only");
        stage_project_file(&directory, b"not committed").unwrap();
        drop(first_studio);

        let reopened = studio(root.path());
        let project = reopened
            .database
            .video_project("stage-only")
            .unwrap()
            .unwrap();
        assert_eq!(project.current_revision, 1);
        assert!(reopened
            .database
            .pending_video_project_write("stage-only")
            .unwrap()
            .is_none());
    }

    #[test]
    fn durable_intent_is_forward_completed_after_process_reopen() {
        let root = tempdir().unwrap();
        let first_studio = studio(root.path());
        create_project(&first_studio, "intent-recovery", None).unwrap();
        let (_, pending, content) =
            prepare_pending_revision(&first_studio, "intent-recovery", "Recovered");
        assert_eq!(pending.new_revision, 2);
        drop(first_studio);

        let reopened = studio(root.path());
        let read = read_file(&reopened, "intent-recovery", PROJECT_FILE).unwrap();
        assert_eq!(read["revision"], 2);
        assert_eq!(read["content"], content);
        assert!(reopened
            .database
            .pending_video_project_write("intent-recovery")
            .unwrap()
            .is_none());
    }

    #[test]
    fn matching_existing_history_is_reused_without_overwrite() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        create_project(&studio, "history-recovery", None).unwrap();
        let (directory, pending, content) =
            prepare_pending_revision(&studio, "history-recovery", "History");
        let history = directory.join(format!(
            ".history/{:06}-{}.vpe",
            pending.new_revision, pending.new_sha
        ));
        write_immutable(&history, content.as_bytes()).unwrap();
        recover_pending_write(&studio, &directory, "history-recovery").unwrap();
        assert_eq!(fs::read(&history).unwrap(), content.as_bytes());
        assert_eq!(
            studio
                .database
                .video_project("history-recovery")
                .unwrap()
                .unwrap()
                .current_revision,
            2
        );
    }

    #[test]
    fn conflicting_existing_history_fails_closed() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        create_project(&studio, "history-conflict", None).unwrap();
        let (directory, pending, _) =
            prepare_pending_revision(&studio, "history-conflict", "Conflict");
        let history = directory.join(format!(
            ".history/{:06}-{}.vpe",
            pending.new_revision, pending.new_sha
        ));
        write_immutable(&history, b"conflicting bytes").unwrap();
        let error = recover_pending_write(&studio, &directory, "history-conflict").unwrap_err();
        assert!(error.to_string().contains("conflicting content"));
        assert_eq!(fs::read(&history).unwrap(), b"conflicting bytes");
        assert!(studio
            .database
            .pending_video_project_write("history-conflict")
            .unwrap()
            .is_some());
    }

    #[test]
    fn finalize_commit_failure_recovers_after_current_publish_and_reopen() {
        let root = tempdir().unwrap();
        let first_studio = studio(root.path());
        create_project(&first_studio, "commit-recovery", None).unwrap();
        let (directory, pending, content) =
            prepare_pending_revision(&first_studio, "commit-recovery", "Commit Recovery");
        let injector = Connection::open(first_studio.database.path()).unwrap();
        injector
            .execute_batch(
                "CREATE TRIGGER inject_video_project_finalize_failure
                 BEFORE UPDATE OF current_revision ON video_projects
                 WHEN NEW.id='commit-recovery' AND NEW.current_revision=2
                 BEGIN
                   SELECT RAISE(ABORT, 'injected finalize commit failure');
                 END;",
            )
            .unwrap();
        let error = publish_and_finalize(&first_studio, &directory, &pending).unwrap_err();
        assert!(error
            .to_string()
            .contains("injected finalize commit failure"));
        assert_eq!(
            content_sha256(&fs::read_to_string(directory.join(PROJECT_FILE)).unwrap()),
            pending.new_sha
        );
        assert_eq!(
            first_studio
                .database
                .video_project("commit-recovery")
                .unwrap()
                .unwrap()
                .current_revision,
            1
        );
        assert!(first_studio
            .database
            .pending_video_project_write("commit-recovery")
            .unwrap()
            .is_some());
        injector
            .execute_batch("DROP TRIGGER inject_video_project_finalize_failure;")
            .unwrap();
        drop(injector);
        drop(first_studio);

        let reopened = studio(root.path());
        let read = read_file(&reopened, "commit-recovery", PROJECT_FILE).unwrap();
        assert_eq!(read["revision"], 2);
        assert_eq!(read["content"], content);
    }

    #[test]
    fn only_project_vpe_is_writable_and_parent_paths_are_rejected() {
        let directory = tempdir().unwrap();
        let studio = studio(directory.path());
        create_project(&studio, "safe-project", None).unwrap();
        assert!(matches!(
            write_file(&studio, "safe-project", "receipts/a.json", "{}", 1),
            Err(EditorError::Invalid(_))
        ));
        assert!(matches!(
            read_file(&studio, "safe-project", "../project.vpe"),
            Err(EditorError::Invalid(_))
        ));
    }

    #[test]
    fn export_binds_the_current_validated_video_project_revision() {
        let directory = tempdir().unwrap();
        let mut studio = studio(directory.path());
        create_project(&studio, "export-project", Some(export_project())).unwrap();
        prepare_video_project_render(&mut studio, "export-project");
        assert!(matches!(
            export(&studio, "export-project"),
            Err(EditorError::Conflict(_))
        ));
        validate(&studio, "export-project").unwrap();
        let queued = export(&studio, "export-project").unwrap();
        assert_eq!(queued["render"]["created"], true);
        assert_eq!(queued["render"]["job"]["kind"], "video_project");
        assert_eq!(queued["render"]["job"]["project_id"], "export-project");
        assert_eq!(queued["render"]["job"]["project_revision"], 1);
        assert_eq!(
            queued["render"]["job"]["document_sha"],
            queued["document_sha256"]
        );
        assert!(queued["render"]["job"].get("render_plan").is_none());
        assert!(queued["render"]["job"].get("snapshot_dir").is_none());
        assert!(queued["render"]["job"].get("output_dir").is_none());
        let first_id = queued["render"]["job"]["id"].as_str().unwrap();
        let first_row = studio.database.render_job_by_id(first_id).unwrap().unwrap();
        let plan = first_row.render_plan.unwrap();
        let before = fs::read(&plan).unwrap();
        let changed = export_project()
            .replace("Export Project", "Export Project v2")
            .replace("00:00:04.000", "00:00:03.900");
        write_file(&studio, "export-project", PROJECT_FILE, &changed, 1).unwrap();
        assert_eq!(fs::read(&plan).unwrap(), before);
        assert!(matches!(
            export(&studio, "export-project"),
            Err(EditorError::Conflict(_))
        ));
        validate(&studio, "export-project").unwrap();
        let second = export(&studio, "export-project").unwrap();
        assert_ne!(queued["render"]["job"]["id"], second["render"]["job"]["id"]);
        let second_id = second["render"]["job"]["id"].as_str().unwrap();
        let second_plan = studio
            .database
            .render_job_by_id(second_id)
            .unwrap()
            .unwrap()
            .render_plan
            .unwrap();
        assert_ne!(fs::read(&plan).unwrap(), fs::read(second_plan).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn export_rejects_symlinked_project_asset() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        let mut studio = studio(directory.path());
        create_project(&studio, "symlink-assets", Some(export_project())).unwrap();
        prepare_video_project_render(&mut studio, "symlink-assets");
        validate(&studio, "symlink-assets").unwrap();
        let source = studio
            .settings
            .video_projects_dir
            .join("symlink-assets/assets/source.mp4");
        let outside = directory.path().join("outside.mp4");
        fs::rename(&source, &outside).unwrap();
        symlink(&outside, &source).unwrap();
        let error = export(&studio, "symlink-assets").unwrap_err();
        assert!(error.to_string().contains("open project asset"));
    }

    #[test]
    fn unified_actions_queue_frames_and_fail_closed_without_receipt_key() {
        let directory = tempdir().unwrap();
        let studio = studio(directory.path());
        fs::create_dir_all(directory.path().join("scripts")).unwrap();
        fs::write(
            directory.path().join("scripts/video_media_job.py"),
            include_bytes!("../scripts/video_media_job.py"),
        )
        .unwrap();
        fs::write(studio.settings.video_input_dir.join("clip.mp4"), b"video").unwrap();
        create_project(&studio, "quality-project", Some(quality_project())).unwrap();

        let frame_request: VideoEditorRequest = serde_json::from_value(json!({
            "action": "extract_analysis_frames",
            "request": {
                "video_path": "clip.mp4"
            }
        }))
        .unwrap();
        let queued = execute(&studio, frame_request).unwrap();
        assert_eq!(queued["job"]["kind"], "analysis_frames");
        assert_eq!(queued["job"]["status"], "queued");

        let phase_request: VideoEditorRequest = serde_json::from_value(json!({
            "action": "validate_phase",
            "project": "quality-project",
            "request": {
                "phase": "pre-render",
                "subtitle_overflow": {
                    "checked": true,
                    "overflow_count": 0
                }
            }
        }))
        .unwrap();
        let validation = execute(&studio, phase_request).unwrap();
        assert_eq!(validation["report"]["passed"], false);
        assert_eq!(validation["provenance"]["capability"], "not-created");
        assert!(!studio
            .settings
            .video_projects_dir
            .join("quality-project/receipts/rev-1/pre-render/audit_receipt.json")
            .exists());
        let repeated: VideoEditorRequest = serde_json::from_value(json!({
            "action": "validate_phase",
            "project": "quality-project",
            "request": {
                "phase": "pre-render",
                "subtitle_overflow": {
                    "checked": true,
                    "overflow_count": 0
                }
            }
        }))
        .unwrap();
        let repeated = execute(&studio, repeated).unwrap();
        assert_eq!(repeated["report"]["passed"], false);
        assert_eq!(repeated["provenance"]["capability"], "not-created");
    }

    #[test]
    fn unavailable_signing_key_does_not_consume_retryable_pass_path() {
        let directory = tempdir().unwrap();
        let studio = studio(directory.path());
        let content = quality_project().replace(
            "  variant",
            "  gate pre_render require input_manifest, continuous_timeline, opening_hook, subtitle_overflow\n  variant",
        );
        create_project(&studio, "retry-pass", Some(&content)).unwrap();
        let asset = studio
            .settings
            .video_projects_dir
            .join("retry-pass/assets/source.mp4");
        fs::write(&asset, b"video").unwrap();
        fs::write(
            studio
                .settings
                .video_projects_dir
                .join("retry-pass/subs.zh-en.ass"),
            b"[Script Info]\nPlayResX: 1080\nPlayResY: 1920\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,64,&H00FFFFFF,&H0,&H0,&H0,0,0,0,0,100,100,0,0,1,3,0,2,80,80,140,1\n[Events]\nDialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Hello\n",
        )
        .unwrap();
        let source_hash = provenance::sha256_file(&asset).unwrap();
        let request = || VideoEditorRequest::ValidatePhase {
            project: "retry-pass".into(),
            request: serde_json::from_value(json!({
                "phase": "pre-render",
                "input_manifest": {"main": source_hash},
                "subtitle_overflow": {"checked": true, "overflow_count": 0}
            }))
            .unwrap(),
        };
        let unavailable = execute(&studio, request()).unwrap();
        assert_eq!(
            unavailable["report"]["passed"], true,
            "validation={unavailable:#}"
        );
        assert_eq!(unavailable["provenance"]["capability"], "unavailable");
        let canonical = studio
            .settings
            .video_projects_dir
            .join("retry-pass/receipts/rev-1/pre-render");
        assert!(!canonical.exists());

        fs::write(&studio.settings.receipt_key_file, b"test-signing-key").unwrap();
        let retried = execute(&studio, request()).unwrap();
        assert_eq!(retried["provenance"]["capability"], "available");
        assert!(canonical.join("validation_report.json").is_file());
        assert!(canonical.join("audit_receipt.json").is_file());
    }

    #[test]
    fn quality_phases_cannot_skip_predecessors() {
        let directory = tempdir().unwrap();
        let studio = studio(directory.path());
        create_project(&studio, "phase-chain", Some(quality_project())).unwrap();
        let request: VideoEditorRequest = serde_json::from_value(json!({
            "action": "validate_phase",
            "project": "phase-chain",
            "request": {
                "phase": "pre-package"
            }
        }))
        .unwrap();
        let error = execute(&studio, request).unwrap_err();
        assert!(error.to_string().contains("complete prior phase chain"));
        assert!(!studio
            .settings
            .video_projects_dir
            .join("phase-chain/receipts/rev-1/pre-package/validation_report.json")
            .exists());
    }

    #[test]
    fn migrated_product_actions_are_callable_and_keep_rights_validation() {
        let directory = tempdir().unwrap();
        let studio = studio(directory.path());
        let status = execute(&studio, VideoEditorRequest::GetStatus {}).unwrap();
        assert_eq!(status["service"], "Video Work API");
        let created = execute(
            &studio,
            VideoEditorRequest::CreateSpeaker {
                name: "Narrator".into(),
            },
        )
        .unwrap();
        let speaker_id = created["id"].as_str().unwrap().to_owned();
        let listed = execute(&studio, VideoEditorRequest::ListSpeakers {}).unwrap();
        assert_eq!(listed["speakers"].as_array().unwrap().len(), 1);
        execute(
            &studio,
            VideoEditorRequest::RenameSpeaker {
                speaker_id: speaker_id.clone(),
                name: "Host".into(),
            },
        )
        .unwrap();
        fs::write(
            studio.settings.reference_input_dir.join("missing.wav"),
            b"not-used-without-consent",
        )
        .unwrap();
        let denied = execute(
            &studio,
            VideoEditorRequest::AddVoiceProfile {
                speaker_id: speaker_id.clone(),
                style_name: "formal".into(),
                prompt_text: "Exact transcript".into(),
                audio_path: "missing.wav".into(),
                confirm_rights: false,
            },
        )
        .unwrap_err();
        assert!(denied.to_string().contains("rights_required"));
        execute(&studio, VideoEditorRequest::DeleteSpeaker { speaker_id }).unwrap();
        assert!(serde_json::from_value::<VideoEditorRequest>(json!({
            "action": "get_status",
            "unexpected": true
        }))
        .is_err());
    }

    #[test]
    fn cleanup_action_defaults_to_non_destructive_dry_run() {
        let directory = tempdir().unwrap();
        let studio = studio(directory.path());
        create_project(&studio, "cleanup-project", None).unwrap();
        let temporary = studio
            .settings
            .video_projects_dir
            .join("cleanup-project/proxies/example.proxy.mp4");
        fs::create_dir_all(temporary.parent().unwrap()).unwrap();
        fs::write(&temporary, b"temporary").unwrap();
        let request: VideoEditorRequest = serde_json::from_value(json!({
            "action": "cleanup_intermediates",
            "project": "cleanup-project",
            "request": { "paths": ["proxies/example.proxy.mp4"] }
        }))
        .unwrap();
        let result = execute(&studio, request).unwrap();
        assert_eq!(result["dry_run"], true);
        assert!(temporary.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_project_component_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        let studio = studio(directory.path());
        create_project(&studio, "safe-project", None).unwrap();
        let project = studio.settings.video_projects_dir.join("safe-project");
        fs::create_dir(project.join("outside")).unwrap();
        symlink(directory.path(), project.join("outside/link")).unwrap();
        assert!(matches!(
            read_file(&studio, "safe-project", "outside/link/project.vpe"),
            Err(EditorError::Internal(_)) | Err(EditorError::NotFound(_))
        ));
    }

    #[test]
    fn atomic_replace_never_exposes_partial_content() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let directory = tempdir().unwrap();
        let path = directory.path().join(PROJECT_FILE);
        create_private_directory(&directory.path().join(".tmp")).unwrap();
        let first = "A".repeat(256 * 1024);
        let second = "B".repeat(256 * 1024);
        fs::write(&path, &first).unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let reader_running = running.clone();
        let reader_path = path.clone();
        let reader_first = first.clone();
        let reader_second = second.clone();
        let reader = std::thread::spawn(move || {
            while reader_running.load(Ordering::Acquire) {
                let value = fs::read_to_string(&reader_path).unwrap();
                assert!(value == reader_first || value == reader_second);
            }
        });
        for _ in 0..4 {
            write_atomic(&path, second.as_bytes()).unwrap();
            write_atomic(&path, first.as_bytes()).unwrap();
        }
        running.store(false, Ordering::Release);
        reader.join().unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), first);
    }
}
