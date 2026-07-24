use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::io::{self, Seek};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Instant};
use uuid::Uuid;

#[cfg(test)]
use crate::database::NewRenderJob;
use crate::database::{
    NewMediaJob, NewVideoProjectRenderJob, PublicationIntentOutcome, RenderJobRow,
};
use crate::project_render;
use crate::studio::Studio;
use crate::vpe::VpeDocument;
use crate::{alignment, provenance, quality};

const EXECUTOR_WRAPPER: &str = r#"import json,os,signal,sys,time
fd=int(sys.argv[1]); path=sys.argv[2]; root=sys.argv[3]
handshake=sys.argv[4]; job_id=sys.argv[5]; executor_identity=sys.argv[6]
pid=os.getpid()
stat=open('/proc/self/stat','r',encoding='ascii').read()
starttime=int(stat.rsplit(')',1)[1].split()[19])
record=json.dumps({'pid':pid,'starttime':starttime,'job_id':job_id,'executor_identity':executor_identity},sort_keys=True,separators=(',',':')).encode('ascii')+b'\n'
directory=os.path.dirname(handshake); final_name=os.path.basename(handshake)
dfd=os.open(directory,os.O_RDONLY|os.O_DIRECTORY)
temp_name='.launch-handshake.'+str(pid)+'.'+os.urandom(8).hex()+'.tmp'
flags=os.O_WRONLY|os.O_CREAT|os.O_EXCL
if hasattr(os,'O_NOFOLLOW'): flags|=os.O_NOFOLLOW
hfd=os.open(temp_name,flags,0o600,dir_fd=dfd)
try:
 offset=0
 pause_after=int(os.environ.get('VWA_TEST_HANDSHAKE_PAUSE_AFTER_BYTES','0'))
 while offset<len(record):
  limit=len(record)
  if pause_after>offset: limit=min(limit,pause_after)
  written=os.write(hfd,record[offset:limit])
  if written<=0: raise OSError('short launch handshake write')
  offset+=written
  if pause_after and offset>=pause_after:
   os.fsync(hfd); pause_after=0; os.kill(pid,signal.SIGSTOP)
 os.fsync(hfd)
 os.link(temp_name,final_name,src_dir_fd=dfd,dst_dir_fd=dfd,follow_symlinks=False)
 os.fsync(dfd)
finally:
 os.close(hfd)
 try: os.unlink(temp_name,dir_fd=dfd)
 except FileNotFoundError: pass
 os.fsync(dfd); os.close(dfd)
delay_before_stop=int(os.environ.get('VWA_TEST_HANDSHAKE_DELAY_BEFORE_STOP_MS','0'))
if delay_before_stop: time.sleep(delay_before_stop/1000)
if os.environ.get('VWA_TEST_HANDSHAKE_EXIT_BEFORE_STOP')=='1': os._exit(72)
os.kill(pid,signal.SIGSTOP)
sys.path.insert(0,root); sys.argv=[path]+sys.argv[7:]
data=b''
while True:
 b=os.read(fd,1048576)
 if not b: break
 data+=b
exec(compile(data,path,'exec'),{'__name__':'__main__','__file__':path})
"#;

#[cfg(test)]
#[derive(Clone, Copy, Default)]
struct TestLaunchHook {
    delay_before_stop_ms: u64,
    exit_before_stop: bool,
}

#[cfg(test)]
static TEST_LAUNCH_HOOKS: OnceLock<Mutex<HashMap<String, TestLaunchHook>>> = OnceLock::new();

#[cfg(test)]
fn set_test_launch_hook(job_id: &str, hook: TestLaunchHook) {
    TEST_LAUNCH_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("test launch hook mutex")
        .insert(job_id.to_owned(), hook);
}

#[cfg(test)]
fn take_test_launch_hook(job_id: &str) -> Option<TestLaunchHook> {
    TEST_LAUNCH_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("test launch hook mutex")
        .remove(job_id)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchHandshake {
    pid: u32,
    starttime: u64,
    job_id: String,
    executor_identity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationIntent {
    schema_version: u32,
    job_id: String,
    kind: String,
    project_id: Option<String>,
    project_revision: Option<i64>,
    document_sha256: Option<String>,
    attestation: Option<Value>,
    files: Vec<PublicationFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationFile {
    source_relative: String,
    destination_relative: String,
    sha256: String,
    size: u64,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitRenderRequest {
    pub task_dir: String,
    pub subject_id: String,
    #[serde(default = "default_encoder_profile")]
    pub encoder_profile: String,
}

#[cfg(test)]
fn default_encoder_profile() -> String {
    "formal-auto".into()
}

#[cfg(test)]
#[derive(Debug)]
struct PreparedRender {
    task_dir: PathBuf,
    subject_id: String,
    encoder_profile: String,
    source_dir: PathBuf,
    input_manifest: PathBuf,
    edl: PathBuf,
    subtitle_ze: PathBuf,
    subtitle_re: PathBuf,
    renderer_hash: String,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
struct SourceIdentity {
    filename: String,
    sha256: String,
    size: u64,
}

#[cfg(test)]
#[derive(Debug)]
struct FrozenInputs {
    snapshot_dir: PathBuf,
    snapshot_hash: String,
}

#[derive(Debug)]
struct ExecutionRender {
    renderer: PathBuf,
    python: PathBuf,
    snapshot_dir: PathBuf,
    timeout_seconds: u64,
    kind: ExecutionKind,
}

#[derive(Debug)]
enum ExecutionKind {
    LegacyXry {
        edl: PathBuf,
        source_dir: PathBuf,
        input_manifest: PathBuf,
        work_dir: PathBuf,
    },
    VideoProject {
        canonical_edl: PathBuf,
        assets_root: PathBuf,
        output_dir: PathBuf,
        replay_output_dir: PathBuf,
    },
    Media {
        request: PathBuf,
        input: PathBuf,
        output_dir: PathBuf,
        funclip_root: Option<PathBuf>,
    },
}

#[cfg(test)]
fn submit(studio: &Studio, request: SubmitRenderRequest) -> Result<Value> {
    let prepared = prepare(studio, &request)?;
    let id = Uuid::new_v4().to_string();
    let frozen = freeze_inputs(studio, &id, &prepared)?;
    let render_key = frozen_render_key(&prepared, &frozen);
    let log_path = studio
        .settings
        .render_jobs_dir()
        .join(&id)
        .join("render.log");
    let inserted = studio.database.insert_or_get_render_job(NewRenderJob {
        id: &id,
        render_key: &render_key,
        task_dir: &prepared.task_dir.to_string_lossy(),
        subject_id: &prepared.subject_id,
        encoder_profile: &prepared.encoder_profile,
        log_path: &log_path.to_string_lossy(),
        snapshot_dir: &frozen.snapshot_dir.to_string_lossy(),
        snapshot_hash: &frozen.snapshot_hash,
        renderer_hash: &prepared.renderer_hash,
    });
    let (row, created) = match inserted {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(studio.settings.render_jobs_dir().join(&id));
            return Err(error);
        }
    };
    if !created {
        fs::remove_dir_all(studio.settings.render_jobs_dir().join(&id))
            .context("remove unused deduplicated render snapshot")?;
    }
    Ok(job_payload(row, created))
}

pub fn submit_video_project(
    studio: &Studio,
    project_dir: &Path,
    project_id: &str,
    revision: i64,
    document_sha: &str,
    document: &VpeDocument,
) -> Result<Value> {
    let renderer_root = studio
        .settings
        .video_project_renderer
        .parent()
        .ok_or_else(|| anyhow!("configured video project renderer has no parent"))?;
    let renderer_root =
        canonical_directory_without_symlinks(renderer_root, "video project renderer root")?;
    let renderer = canonical_file_under_without_symlinks(
        &studio.settings.video_project_renderer,
        &renderer_root,
        "video project renderer",
    )?;
    let renderer_hash = hash_file(&renderer)?;
    let python_root = studio
        .settings
        .video_project_python
        .parent()
        .ok_or_else(|| anyhow!("configured video project Python has no parent"))?;
    let python_root =
        canonical_directory_without_symlinks(python_root, "video project Python root")?;
    canonical_file_under_without_symlinks(
        &studio.settings.video_project_python,
        &python_root,
        "video project Python",
    )?;

    let id = Uuid::new_v4().to_string();
    let compiled = match project_render::compile(
        studio,
        &id,
        project_dir,
        project_id,
        revision,
        document_sha,
        document,
    ) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(studio.settings.render_jobs_dir().join(&id));
            return Err(error);
        }
    };
    let mut key = Sha256::new();
    key.update(b"video_project\0");
    key.update(project_id.as_bytes());
    key.update([0]);
    key.update(revision.to_le_bytes());
    key.update(document_sha.as_bytes());
    key.update([0]);
    key.update(compiled.bundle_hash.as_bytes());
    key.update([0]);
    key.update(renderer_hash.as_bytes());
    let render_key = format!("{:x}", key.finalize());
    let log_path = studio
        .settings
        .render_jobs_dir()
        .join(&id)
        .join("render.log");
    let inserted =
        studio
            .database
            .insert_or_get_video_project_render_job(NewVideoProjectRenderJob {
                id: &id,
                render_key: &render_key,
                project_id,
                project_revision: revision,
                document_sha,
                output_dir: &compiled.output_dir.to_string_lossy(),
                log_path: &log_path.to_string_lossy(),
                snapshot_dir: &compiled.snapshot_dir.to_string_lossy(),
                snapshot_hash: &compiled.bundle_hash,
                renderer_hash: &renderer_hash,
                render_plan: &compiled.canonical_edl.to_string_lossy(),
            });
    let (row, created) = match inserted {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(studio.settings.render_jobs_dir().join(&id));
            return Err(error);
        }
    };
    if !created {
        fs::remove_dir_all(studio.settings.render_jobs_dir().join(&id))
            .context("remove unused deduplicated video project bundle")?;
    }
    Ok(job_payload(row, created))
}

pub fn submit_media(
    studio: &Studio,
    kind: &str,
    subject: &str,
    source: &Path,
    request: &Value,
    project_id: Option<&str>,
) -> Result<Value> {
    if !matches!(kind, "analysis_frames" | "safe_trims" | "cover") {
        bail!("unsupported media job kind");
    }
    let renderer = studio
        .settings
        .project_root
        .join("scripts/video_media_job.py");
    let renderer_root = canonical_directory_without_symlinks(
        renderer
            .parent()
            .context("media renderer has no parent directory")?,
        "media renderer root",
    )?;
    let renderer =
        canonical_file_under_without_symlinks(&renderer, &renderer_root, "media renderer")?;
    let renderer_hash = hash_file(&renderer)?;
    let python = canonical_file_under_without_symlinks(
        &studio.settings.video_project_python,
        studio
            .settings
            .video_project_python
            .parent()
            .context("configured Python has no parent")?,
        "media Python",
    )?;
    let _ = python;
    let id = Uuid::new_v4().to_string();
    let job_dir = studio.settings.render_jobs_dir().join(&id);
    let snapshot = job_dir.join("snapshot");
    let output_dir = job_dir.join("attempt-1").join("output");
    let log_path = job_dir.join("render.log");
    let result = (|| -> Result<(String, String, PathBuf)> {
        create_private_directory(&job_dir)?;
        create_private_directory(&snapshot)?;
        let input = snapshot.join("input.media");
        copy_regular_nofollow(source, &input)?;
        let source_sha256 = hash_file(&input)?;
        let request_path = snapshot.join("request.json");
        write_new_synced(&request_path, &serde_json::to_vec(request)?)?;
        sync_directory(&snapshot)?;
        sync_directory(&job_dir)?;
        let snapshot_hash = hash_media_snapshot(&snapshot)?;
        Ok((source_sha256, snapshot_hash, request_path))
    })();
    let (source_sha256, snapshot_hash, request_path) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&job_dir);
            return Err(error);
        }
    };
    let mut key = Sha256::new();
    key.update(kind.as_bytes());
    key.update([0]);
    key.update(subject.as_bytes());
    key.update([0]);
    key.update(snapshot_hash.as_bytes());
    key.update([0]);
    key.update(renderer_hash.as_bytes());
    let render_key = format!("{:x}", key.finalize());
    let project_revision = request.get("project_revision").and_then(Value::as_i64);
    let bound_document_sha = request
        .get("document_sha256")
        .and_then(Value::as_str)
        .unwrap_or(&source_sha256);
    let inserted = studio.database.insert_or_get_media_job(NewMediaJob {
        id: &id,
        render_key: &render_key,
        kind,
        subject,
        project_id,
        project_revision,
        document_sha: bound_document_sha,
        output_dir: &output_dir.to_string_lossy(),
        log_path: &log_path.to_string_lossy(),
        snapshot_dir: &snapshot.to_string_lossy(),
        snapshot_hash: &snapshot_hash,
        renderer_hash: &renderer_hash,
        request_path: &request_path.to_string_lossy(),
    });
    let (row, created) = match inserted {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&job_dir);
            return Err(error);
        }
    };
    if !created {
        fs::remove_dir_all(&job_dir).context("remove deduplicated media job snapshot")?;
    }
    Ok(job_payload(row, created))
}

pub fn get(studio: &Studio, id: &str) -> Result<Option<Value>> {
    Ok(studio
        .database
        .render_job_by_id(id)?
        .map(|row| job_payload(row, false)))
}

pub fn cancel(studio: &Studio, id: &str) -> Result<Option<Value>> {
    Ok(studio
        .database
        .request_cancel_render_job(id)?
        .map(|row| job_payload(row, false)))
}

pub(crate) fn list_public(studio: &Studio) -> Result<Value> {
    let jobs = studio
        .database
        .recent_render_jobs()?
        .into_iter()
        .map(|row| job_payload(row, false)["job"].clone())
        .collect::<Vec<_>>();
    Ok(json!({ "jobs": jobs }))
}

fn job_payload(row: RenderJobRow, created: bool) -> Value {
    let public_error = match row.status.as_str() {
        "failed" => Some(
            if row
                .error
                .as_deref()
                .is_some_and(|error| error.to_ascii_lowercase().contains("timeout"))
            {
                json!({"code": "timeout", "message": "Render timed out."})
            } else {
                json!({"code": "failed", "message": "Render failed."})
            },
        ),
        "canceled" => Some(json!({"code": "canceled", "message": "Render was canceled."})),
        _ => None,
    };
    let result = row
        .output_dir
        .as_deref()
        .map(Path::new)
        .map(|directory| directory.join("result.json"))
        .filter(|path| path.is_file())
        .and_then(|path| provenance::read_regular_with_sha256(&path).ok())
        .and_then(|(bytes, _)| serde_json::from_slice::<Value>(&bytes).ok())
        .or_else(|| {
            (row.kind == "cover")
                .then_some(row.attestation_json.as_deref())
                .flatten()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
        });
    let deliverables = row
        .attestation_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<quality::TrustedRenderAttestation>(value).ok())
        .map(|attestation| {
            json!({
                "master": attestation.master_relative,
                "variants": attestation.variants,
                "render_report": attestation.report_relative,
            })
        });
    let public_job = json!({
        "id": row.id,
        "kind": row.kind,
        "status": row.status,
        "project_id": row.project_id,
        "project_revision": row.project_revision,
        "document_sha": row.document_sha,
        "enqueue_seq": row.enqueue_seq,
        "cancel_requested": row.cancel_requested,
        "exit_code": row.exit_code,
        "error": public_error,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "started_at": row.started_at,
        "finished_at": row.finished_at,
        "deliverables": deliverables,
    });
    json!({
        "job": public_job,
        "created": created,
        "deduplicated": !created,
        "result": result,
    })
}

#[cfg(test)]
fn prepare(studio: &Studio, request: &SubmitRenderRequest) -> Result<PreparedRender> {
    let subject = request.subject_id.trim();
    let encoder_profile = request.encoder_profile.trim();
    if !matches!(
        encoder_profile,
        "formal-cpu" | "formal-auto" | "formal-vaapi"
    ) {
        bail!("encoder_profile must be formal-cpu, formal-auto, or formal-vaapi");
    }
    if subject.len() < 3
        || subject.len() > 4
        || !subject.starts_with('S')
        || !subject[1..].chars().all(|ch| ch.is_ascii_digit())
    {
        bail!("subject_id must match S followed by 2 or 3 digits");
    }

    let task_root = canonical_directory_without_symlinks(
        &studio.settings.xry_task_root,
        "configured XRY task root",
    )?;
    let candidate = Path::new(request.task_dir.trim());
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        studio.settings.xry_task_root.join(candidate)
    };
    let lexical_relative = candidate
        .strip_prefix(&studio.settings.xry_task_root)
        .map_err(|_| anyhow!("task_dir must be inside the configured XRY task root"))?;
    if lexical_relative.components().count() != 2
        || !lexical_relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("task_dir must identify one exact <group>/<batch> directory");
    }
    let task_dir =
        canonical_directory_under_without_symlinks(&candidate, &task_root, "task directory")?;
    let relative = task_dir
        .strip_prefix(&task_root)
        .context("canonical task escaped configured XRY task root")?;

    let source_root = canonical_directory_without_symlinks(
        &studio.settings.xry_source_root,
        "configured XRY source root",
    )?;
    let source_dir = canonical_directory_under_without_symlinks(
        &studio.settings.xry_source_root.join(relative),
        &source_root,
        "derived source directory",
    )?;

    let production_dir = canonical_directory_under_without_symlinks(
        &task_dir.join(".pipeline/production").join(subject),
        &task_dir,
        "subject production directory",
    )?;
    let renderer_root = studio
        .settings
        .xry_renderer
        .parent()
        .ok_or_else(|| anyhow!("configured XRY renderer has no parent directory"))?;
    let renderer_root =
        canonical_directory_without_symlinks(renderer_root, "official renderer root")?;
    let renderer = canonical_file_under_without_symlinks(
        &studio.settings.xry_renderer,
        &renderer_root,
        "official XRY renderer",
    )?;
    let python_root = studio
        .settings
        .xry_python
        .parent()
        .ok_or_else(|| anyhow!("configured XRY Python has no parent directory"))?;
    let python_root = canonical_directory_without_symlinks(python_root, "official Python root")?;
    let _python = canonical_file_under_without_symlinks(
        &studio.settings.xry_python,
        &python_root,
        "official XRY Python",
    )?;
    let edl = canonical_file_under_without_symlinks(
        &production_dir.join("edl.json"),
        &task_dir,
        "frozen EDL",
    )?;
    let subtitle_ze = canonical_file_under_without_symlinks(
        &production_dir.join("subs.zh-en.ass"),
        &task_dir,
        "frozen ZE subtitles",
    )?;
    let subtitle_re = canonical_file_under_without_symlinks(
        &production_dir.join("subs.ru-en.ass"),
        &task_dir,
        "frozen RE subtitles",
    )?;
    let input_manifest = canonical_file_under_without_symlinks(
        &task_dir.join(".pipeline/input_manifest.tsv"),
        &task_dir,
        "frozen input manifest",
    )?;
    let work_dir = production_dir.join("render");
    reject_symlink_components(&work_dir, true, "render work directory")?;
    if work_dir.exists() {
        let canonical_work = work_dir
            .canonicalize()
            .context("canonicalize render work directory")?;
        if !canonical_work.starts_with(&task_dir) || !canonical_work.is_dir() {
            bail!("render work directory escaped exact task directory");
        }
    }
    let renderer_hash = hash_file(&renderer)?;

    Ok(PreparedRender {
        task_dir,
        subject_id: subject.to_string(),
        encoder_profile: encoder_profile.to_string(),
        source_dir,
        input_manifest,
        edl,
        subtitle_ze,
        subtitle_re,
        renderer_hash,
    })
}

#[cfg(test)]
fn freeze_inputs(studio: &Studio, id: &str, prepared: &PreparedRender) -> Result<FrozenInputs> {
    let job_dir = studio.settings.render_jobs_dir().join(id);
    reject_symlink_components(&job_dir, true, "render job directory")?;
    fs::create_dir(&job_dir).context("create private render job directory")?;
    #[cfg(unix)]
    fs::set_permissions(&job_dir, fs::Permissions::from_mode(0o700))?;
    let snapshot_dir = job_dir.join("snapshot");
    fs::create_dir(&snapshot_dir).context("create render input snapshot")?;
    #[cfg(unix)]
    fs::set_permissions(&snapshot_dir, fs::Permissions::from_mode(0o700))?;

    let result = (|| {
        for (source, name) in [
            (&prepared.edl, "edl.json"),
            (&prepared.subtitle_ze, "subs.zh-en.ass"),
            (&prepared.subtitle_re, "subs.ru-en.ass"),
            (&prepared.input_manifest, "input_manifest.tsv"),
        ] {
            copy_regular_nofollow(source, &snapshot_dir.join(name))?;
        }
        let identities = validate_manifest(
            &snapshot_dir.join("input_manifest.tsv"),
            &prepared.source_dir,
        )?;
        validate_edl_sources(&snapshot_dir.join("edl.json"), &identities)?;
        let identities = serde_json::to_vec(&identities)?;
        write_new_synced(&snapshot_dir.join("source-identities.json"), &identities)?;
        sync_directory(&snapshot_dir)?;
        sync_directory(&job_dir)?;
        let snapshot_hash = hash_snapshot(&snapshot_dir)?;
        Ok(FrozenInputs {
            snapshot_dir: snapshot_dir.clone(),
            snapshot_hash,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&job_dir);
    }
    result
}

#[cfg(test)]
fn frozen_render_key(prepared: &PreparedRender, frozen: &FrozenInputs) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prepared.task_dir.as_os_str().as_encoded_bytes());
    hasher.update([0]);
    hasher.update(prepared.subject_id.as_bytes());
    hasher.update([0]);
    hasher.update(prepared.encoder_profile.as_bytes());
    hasher.update([0]);
    hasher.update(prepared.renderer_hash.as_bytes());
    hasher.update([0]);
    hasher.update(frozen.snapshot_hash.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn copy_regular_nofollow(source: &Path, destination: &Path) -> Result<()> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut input = options
        .open(source)
        .with_context(|| format!("open frozen input {}", source.display()))?;
    if !input.metadata()?.is_file() {
        bail!("frozen input is not a regular file: {}", source.display());
    }
    let mut output_options = OpenOptions::new();
    output_options.write(true).create_new(true);
    #[cfg(unix)]
    output_options.mode(0o600);
    let mut output = output_options
        .open(destination)
        .with_context(|| format!("create snapshot {}", destination.display()))?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn write_new_synced(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

fn hash_media_snapshot(snapshot: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    for name in ["input.media", "request.json"] {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        let (bytes, hash) = provenance::read_regular_with_sha256(&snapshot.join(name))?;
        hasher.update(hash.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_snapshot(snapshot: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    for name in [
        "edl.json",
        "subs.zh-en.ass",
        "subs.ru-en.ass",
        "input_manifest.tsv",
        "source-identities.json",
    ] {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(snapshot.join(name))?);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_file(path: &Path) -> Result<String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        bail!("input is not a regular file: {}", path.display());
    }
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_manifest(path: &Path, source_dir: &Path) -> Result<Vec<SourceIdentity>> {
    let contents = fs::read_to_string(path).context("read XRY input manifest")?;
    let mut lines = contents.lines();
    let headers: Vec<_> = lines
        .next()
        .ok_or_else(|| anyhow!("input manifest is empty"))?
        .split('\t')
        .collect();
    for required in [
        "filename",
        "size",
        "duration",
        "video_codec",
        "width",
        "height",
        "audio_codec",
        "audio_rate",
    ] {
        if !headers.contains(&required) {
            bail!("input manifest is missing required field {required}");
        }
    }
    let index = |name: &str| headers.iter().position(|header| *header == name);
    let hash_index = ["sha256", "source_sha256", "fingerprint"]
        .iter()
        .find_map(|name| index(name));
    let mut identities = Vec::new();
    let mut filenames = std::collections::HashSet::new();
    for (line_number, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != headers.len() {
            bail!(
                "input manifest row {} has the wrong field count",
                line_number + 2
            );
        }
        let value = |name: &str| fields[index(name).expect("required manifest header")];
        let filename = value("filename");
        let path_value = Path::new(filename);
        if path_value.components().count() != 1
            || !matches!(path_value.components().next(), Some(Component::Normal(_)))
            || !filenames.insert(filename.to_owned())
        {
            bail!("input manifest filename must be a unique basename");
        }
        let declared_size: u64 = value("size").parse().context("invalid manifest size")?;
        let duration: f64 = value("duration")
            .parse()
            .context("invalid manifest duration")?;
        let width: u32 = value("width").parse().context("invalid manifest width")?;
        let height: u32 = value("height").parse().context("invalid manifest height")?;
        let audio_rate: u32 = value("audio_rate")
            .parse()
            .context("invalid manifest audio_rate")?;
        if duration < 0.0
            || !duration.is_finite()
            || width == 0
            || height == 0
            || audio_rate == 0
            || value("video_codec").is_empty()
            || value("audio_codec").is_empty()
        {
            bail!(
                "input manifest row {} has invalid media metadata",
                line_number + 2
            );
        }
        let source = canonical_file_under_without_symlinks(
            &source_dir.join(filename),
            source_dir,
            "manifest source",
        )?;
        let metadata = source.metadata()?;
        if metadata.len() != declared_size {
            bail!("manifest source size does not match: {filename}");
        }
        let sha256 = hash_file(&source)?;
        if let Some(index) = hash_index {
            let declared = fields[index].trim();
            if !declared.is_empty() && !declared.eq_ignore_ascii_case(&sha256) {
                bail!("manifest source hash does not match: {filename}");
            }
        }
        identities.push(SourceIdentity {
            filename: filename.to_owned(),
            sha256,
            size: declared_size,
        });
    }
    if identities.is_empty() {
        bail!("input manifest contains no sources");
    }
    Ok(identities)
}

fn validate_edl_sources(path: &Path, identities: &[SourceIdentity]) -> Result<()> {
    let edl: Value = serde_json::from_slice(&fs::read(path)?).context("parse frozen EDL")?;
    let object = edl
        .as_object()
        .ok_or_else(|| anyhow!("EDL must be a JSON object"))?;
    for track in ["video_segments", "b_roll_overlays"] {
        if let Some(value) = object.get(track) {
            if !value.is_array() {
                bail!("EDL {track} must be an array");
            }
        }
    }
    let allowed: std::collections::HashSet<_> = identities
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    fn walk(value: &Value, allowed: &std::collections::HashSet<&str>) -> Result<()> {
        match value {
            Value::Object(object) => {
                if let Some(source) = object.get("source").and_then(Value::as_str) {
                    let path = Path::new(source);
                    let basename = path.file_name().and_then(|name| name.to_str());
                    if path.components().count() != 1
                        || basename.is_none_or(|v| !allowed.contains(v))
                    {
                        bail!("EDL source is not a manifest basename: {source}");
                    }
                }
                for child in object.values() {
                    walk(child, allowed)?;
                }
            }
            Value::Array(array) => {
                for child in array {
                    walk(child, allowed)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(&edl, &allowed)
}

fn canonical_directory_without_symlinks(path: &Path, label: &str) -> Result<PathBuf> {
    reject_symlink_components(path, false, label)?;
    let canonical = path
        .canonicalize()
        .with_context(|| format!("canonicalize {label} {}", path.display()))?;
    if !canonical.is_dir() {
        bail!("{label} is not a directory: {}", path.display());
    }
    Ok(canonical)
}

fn canonical_directory_under_without_symlinks(
    path: &Path,
    root: &Path,
    label: &str,
) -> Result<PathBuf> {
    reject_symlink_components(path, false, label)?;
    let canonical = path
        .canonicalize()
        .with_context(|| format!("canonicalize {label} {}", path.display()))?;
    if !canonical.starts_with(root) || !canonical.is_dir() {
        bail!("{label} escaped its exact allowed root");
    }
    Ok(canonical)
}

fn canonical_file_under_without_symlinks(path: &Path, root: &Path, label: &str) -> Result<PathBuf> {
    reject_symlink_components(path, false, label)?;
    let canonical = path
        .canonicalize()
        .with_context(|| format!("canonicalize {label} {}", path.display()))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        bail!("{label} escaped its exact allowed root");
    }
    Ok(canonical)
}

fn reject_symlink_components(path: &Path, allow_missing_final: bool, label: &str) -> Result<()> {
    if !path.is_absolute() {
        bail!("{label} must be an absolute path");
    }
    let mut current = PathBuf::new();
    let component_count = path.components().count();
    for (index, component) in path.components().enumerate() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            bail!("{label} contains a non-canonical path component");
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("{label} contains symlink component: {}", current.display());
            }
            Ok(_) => {}
            Err(error)
                if allow_missing_final
                    && index + 1 == component_count
                    && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect {label} component {}", current.display()));
            }
        }
    }
    Ok(())
}

fn prepare_execution(studio: &Studio, job: &RenderJobRow) -> Result<ExecutionRender> {
    match job.kind.as_str() {
        "video_project" => prepare_video_project_execution(studio, job),
        "legacy_xry" => prepare_legacy_execution(studio, job),
        "analysis_frames" | "safe_trims" | "cover" => prepare_media_execution(studio, job),
        other => bail!("unsupported render adapter kind '{other}'"),
    }
}

fn prepare_media_execution(studio: &Studio, job: &RenderJobRow) -> Result<ExecutionRender> {
    let expected_snapshot = studio
        .settings
        .render_jobs_dir()
        .join(&job.id)
        .join("snapshot");
    if Path::new(&job.snapshot_dir) != expected_snapshot {
        bail!("stored media snapshot path does not match its job");
    }
    let jobs_root = canonical_directory_without_symlinks(
        &studio.settings.render_jobs_dir(),
        "render jobs root",
    )?;
    let snapshot_dir = canonical_directory_under_without_symlinks(
        &expected_snapshot,
        &jobs_root,
        "media snapshot",
    )?;
    if hash_media_snapshot(&snapshot_dir)? != job.snapshot_hash {
        bail!("private media snapshot changed after submission");
    }
    let request = canonical_file_under_without_symlinks(
        &snapshot_dir.join("request.json"),
        &snapshot_dir,
        "media request",
    )?;
    if job.render_plan.as_deref() != Some(request.to_string_lossy().as_ref()) {
        bail!("stored media request path does not match");
    }
    if job.kind == "cover" {
        let (bytes, _) = provenance::read_regular_with_sha256(&request)?;
        let value: Value = serde_json::from_slice(&bytes)?;
        if value.get("project_id").and_then(Value::as_str) != job.project_id.as_deref()
            || value.get("project_revision").and_then(Value::as_i64) != job.project_revision
            || value.get("document_sha256").and_then(Value::as_str) != job.document_sha.as_deref()
        {
            bail!("cover request binding differs from its authoritative database row");
        }
    }
    let input = canonical_file_under_without_symlinks(
        &snapshot_dir.join("input.media"),
        &snapshot_dir,
        "media input",
    )?;
    if job.kind != "cover" && job.document_sha.as_deref() != Some(hash_file(&input)?.as_str()) {
        bail!("media input hash does not match queued identity");
    }
    let output_dir = studio
        .settings
        .render_jobs_dir()
        .join(&job.id)
        .join("attempt-1")
        .join("output");
    if job.output_dir.as_deref() != Some(output_dir.to_string_lossy().as_ref()) {
        bail!("stored media output path does not match");
    }
    let attempt_dir = output_dir
        .parent()
        .context("media output has no attempt directory")?;
    reject_symlink_components(attempt_dir, true, "media attempt directory")?;
    if attempt_dir.exists() {
        bail!("media output directory already exists");
    }
    let renderer = studio
        .settings
        .project_root
        .join("scripts/video_media_job.py");
    let renderer_root = canonical_directory_without_symlinks(
        renderer.parent().context("media renderer has no parent")?,
        "media renderer root",
    )?;
    let renderer =
        canonical_file_under_without_symlinks(&renderer, &renderer_root, "media renderer")?;
    if hash_file(&renderer)? != job.renderer_hash {
        bail!("media renderer changed after queue submission");
    }
    let python_root = canonical_directory_without_symlinks(
        studio
            .settings
            .video_project_python
            .parent()
            .context("media Python has no parent")?,
        "media Python root",
    )?;
    let python = canonical_file_under_without_symlinks(
        &studio.settings.video_project_python,
        &python_root,
        "media Python",
    )?;
    if job.kind == "cover" {
        let project_id = job
            .project_id
            .as_deref()
            .context("cover media job lacks project id")?;
        let projects_root = canonical_directory_without_symlinks(
            &studio.settings.video_projects_dir,
            "video projects root",
        )?;
        let project = canonical_directory_under_without_symlinks(
            &projects_root.join(project_id),
            &projects_root,
            "cover project",
        )?;
        canonical_directory_under_without_symlinks(
            &project.join("exports"),
            &project,
            "cover exports",
        )?;
    }
    Ok(ExecutionRender {
        renderer,
        python,
        snapshot_dir,
        timeout_seconds: studio.settings.video_project_render_timeout_seconds,
        kind: ExecutionKind::Media {
            request,
            input,
            output_dir,
            funclip_root: studio
                .settings
                .funclip_root
                .as_ref()
                .map(|root| canonical_directory_without_symlinks(root, "FunClip root"))
                .transpose()?,
        },
    })
}

fn prepare_video_project_execution(studio: &Studio, job: &RenderJobRow) -> Result<ExecutionRender> {
    let project_id = job
        .project_id
        .as_deref()
        .ok_or_else(|| anyhow!("video project job has no project id"))?;
    let _revision = job
        .project_revision
        .ok_or_else(|| anyhow!("video project job has no revision"))?;
    let _document_sha = job
        .document_sha
        .as_deref()
        .ok_or_else(|| anyhow!("video project job has no document hash"))?;
    let expected_snapshot = studio
        .settings
        .render_jobs_dir()
        .join(&job.id)
        .join("snapshot");
    if Path::new(&job.snapshot_dir) != expected_snapshot {
        bail!("stored snapshot path does not match its render job");
    }
    let render_jobs_root = studio
        .settings
        .render_jobs_dir()
        .canonicalize()
        .context("canonicalize render jobs root")?;
    let snapshot_dir = canonical_directory_under_without_symlinks(
        &expected_snapshot,
        &render_jobs_root,
        "video project snapshot",
    )?;
    let canonical_edl = canonical_file_under_without_symlinks(
        &snapshot_dir.join("canonical-edl.json"),
        &snapshot_dir,
        "canonical video project EDL",
    )?;
    if job.render_plan.as_deref() != Some(canonical_edl.to_string_lossy().as_ref()) {
        bail!("stored video project render plan path does not match");
    }
    project_render::validate_snapshot(&canonical_edl, &job.snapshot_hash)?;

    let projects_root = canonical_directory_without_symlinks(
        &studio.settings.video_projects_dir,
        "video projects root",
    )?;
    let project_dir = canonical_directory_under_without_symlinks(
        &projects_root.join(project_id),
        &projects_root,
        "video project directory",
    )?;
    let assets_root = canonical_directory_under_without_symlinks(
        &snapshot_dir.join("assets"),
        &snapshot_dir,
        "frozen video project assets",
    )?;
    project_render::revalidate_assets(&canonical_edl, &assets_root)?;
    let expected_output = project_dir.join("exports").join(&job.id);
    if job.output_dir.as_deref() != Some(expected_output.to_string_lossy().as_ref()) {
        bail!("stored video project output path does not match");
    }
    reject_symlink_components(&expected_output, true, "video project output directory")?;
    let attempt_dir = expected_snapshot
        .parent()
        .context("video project snapshot has no job directory")?
        .join("attempt-1");
    reject_symlink_components(&attempt_dir, true, "private render attempt")?;
    if attempt_dir.exists() {
        bail!("private render attempt already exists");
    }
    let primary_output_dir = attempt_dir.join("primary");
    let replay_output_dir = attempt_dir.join("replay");
    let renderer_root = studio
        .settings
        .video_project_renderer
        .parent()
        .ok_or_else(|| anyhow!("configured video project renderer has no parent"))?;
    let renderer_root =
        canonical_directory_without_symlinks(renderer_root, "video project renderer root")?;
    let renderer = canonical_file_under_without_symlinks(
        &studio.settings.video_project_renderer,
        &renderer_root,
        "video project renderer",
    )?;
    if hash_file(&renderer)? != job.renderer_hash {
        bail!("video project renderer changed after queue submission");
    }
    let python_root = studio
        .settings
        .video_project_python
        .parent()
        .ok_or_else(|| anyhow!("configured video project Python has no parent"))?;
    let python_root =
        canonical_directory_without_symlinks(python_root, "video project Python root")?;
    let python = canonical_file_under_without_symlinks(
        &studio.settings.video_project_python,
        &python_root,
        "video project Python",
    )?;
    Ok(ExecutionRender {
        renderer,
        python,
        snapshot_dir,
        timeout_seconds: studio.settings.video_project_render_timeout_seconds,
        kind: ExecutionKind::VideoProject {
            canonical_edl,
            assets_root,
            output_dir: primary_output_dir,
            replay_output_dir,
        },
    })
}

fn prepare_legacy_execution(studio: &Studio, job: &RenderJobRow) -> Result<ExecutionRender> {
    let expected_snapshot = studio
        .settings
        .render_jobs_dir()
        .join(&job.id)
        .join("snapshot");
    if Path::new(&job.snapshot_dir) != expected_snapshot {
        bail!("stored snapshot path does not match its render job");
    }
    let snapshot_dir = canonical_directory_under_without_symlinks(
        &expected_snapshot,
        &studio
            .settings
            .render_jobs_dir()
            .canonicalize()
            .context("canonicalize render jobs root")?,
        "render snapshot",
    )?;
    if hash_snapshot(&snapshot_dir)? != job.snapshot_hash {
        bail!("private render snapshot hash does not match");
    }
    let identities: Vec<SourceIdentity> =
        serde_json::from_slice(&fs::read(snapshot_dir.join("source-identities.json"))?)?;

    let task_root = canonical_directory_without_symlinks(
        &studio.settings.xry_task_root,
        "configured XRY task root",
    )?;
    let task_dir = canonical_directory_under_without_symlinks(
        Path::new(&job.task_dir),
        &task_root,
        "task directory",
    )?;
    let relative = task_dir.strip_prefix(&task_root)?;
    let source_root = canonical_directory_without_symlinks(
        &studio.settings.xry_source_root,
        "configured XRY source root",
    )?;
    let source_dir = canonical_directory_under_without_symlinks(
        &source_root.join(relative),
        &source_root,
        "derived source directory",
    )?;
    for identity in &identities {
        let source = canonical_file_under_without_symlinks(
            &source_dir.join(&identity.filename),
            &source_dir,
            "snapshotted source",
        )?;
        if source.metadata()?.len() != identity.size || hash_file(&source)? != identity.sha256 {
            bail!("source changed after submission: {}", identity.filename);
        }
    }
    let edl = canonical_file_under_without_symlinks(
        &snapshot_dir.join("edl.json"),
        &snapshot_dir,
        "snapshotted EDL",
    )?;
    validate_edl_sources(&edl, &identities)?;
    let input_manifest = canonical_file_under_without_symlinks(
        &snapshot_dir.join("input_manifest.tsv"),
        &snapshot_dir,
        "snapshotted manifest",
    )?;
    let parsed_identities = validate_manifest(&input_manifest, &source_dir)?;
    if parsed_identities
        .iter()
        .map(|item| (&item.filename, &item.sha256, item.size))
        .ne(identities
            .iter()
            .map(|item| (&item.filename, &item.sha256, item.size)))
    {
        bail!("snapshotted manifest source identity set does not match");
    }
    for name in ["subs.zh-en.ass", "subs.ru-en.ass"] {
        canonical_file_under_without_symlinks(
            &snapshot_dir.join(name),
            &snapshot_dir,
            "snapshotted subtitle",
        )?;
    }
    let production_dir = canonical_directory_under_without_symlinks(
        &task_dir.join(".pipeline/production").join(&job.subject_id),
        &task_dir,
        "subject production directory",
    )?;
    let work_dir = production_dir.join("render");
    reject_symlink_components(&work_dir, true, "render work directory")?;

    let renderer_root = studio
        .settings
        .xry_renderer
        .parent()
        .ok_or_else(|| anyhow!("configured XRY renderer has no parent directory"))?;
    let renderer_root =
        canonical_directory_without_symlinks(renderer_root, "official renderer root")?;
    let renderer = canonical_file_under_without_symlinks(
        &studio.settings.xry_renderer,
        &renderer_root,
        "official XRY renderer",
    )?;
    if hash_file(&renderer)? != job.renderer_hash {
        bail!("official renderer changed after queue submission");
    }
    let python_root = studio
        .settings
        .xry_python
        .parent()
        .ok_or_else(|| anyhow!("configured XRY Python has no parent directory"))?;
    let python_root = canonical_directory_without_symlinks(python_root, "official Python root")?;
    let python = canonical_file_under_without_symlinks(
        &studio.settings.xry_python,
        &python_root,
        "official Python",
    )?;
    Ok(ExecutionRender {
        renderer,
        python,
        snapshot_dir,
        timeout_seconds: studio.settings.render_timeout_seconds,
        kind: ExecutionKind::LegacyXry {
            edl,
            source_dir,
            input_manifest,
            work_dir,
        },
    })
}

#[derive(Debug)]
pub struct RenderWorkerHandle {
    shutdown: watch::Sender<bool>,
    join: JoinHandle<()>,
}

impl RenderWorkerHandle {
    pub fn shutdown_notifier(&self) -> watch::Sender<bool> {
        self.shutdown.clone()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    pub async fn wait(self) -> Result<()> {
        self.join.await.context("join render queue worker")
    }
}

struct WorkerLease {
    _file: File,
}

#[cfg(unix)]
fn acquire_worker_lease(studio: &Studio) -> Result<WorkerLease> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let path = studio.settings.render_jobs_dir().join("worker.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .with_context(|| format!("open render worker lease {}", path.display()))?;
    // SAFETY: flock only observes the valid descriptor owned by `file`, which
    // remains alive in WorkerLease for the full worker lifetime.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == -1 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            bail!("another render queue worker holds the exclusive lease");
        }
        return Err(error).context("acquire exclusive render worker lease");
    }
    Ok(WorkerLease { _file: file })
}

#[cfg(not(unix))]
fn acquire_worker_lease(_studio: &Studio) -> Result<WorkerLease> {
    bail!("the persistent render worker lease is unavailable on this platform")
}

pub fn start_worker(studio: Arc<Studio>) -> Result<RenderWorkerHandle> {
    let lease = acquire_worker_lease(&studio)?;
    let (shutdown, receiver) = watch::channel(false);
    let join = tokio::spawn(run_worker(studio, receiver, lease));
    Ok(RenderWorkerHandle { shutdown, join })
}

async fn run_worker(studio: Arc<Studio>, mut shutdown: watch::Receiver<bool>, _lease: WorkerLease) {
    // Recovery is deliberately inside the exclusive lease. A second process
    // can neither reset the active worker's job nor claim another running row.
    if let Err(error) = recover_publications(&studio) {
        tracing::error!(%error, "failed to recover durable publication");
        return;
    }
    if let Err(error) = recover_running_jobs(&studio).await {
        tracing::error!(%error, "failed to recover render queue");
        return;
    }
    if let Err(error) = sweep_terminal_cleanup(&studio) {
        tracing::error!(%error, "failed to complete durable terminal cleanup");
        return;
    }
    loop {
        if *shutdown.borrow() {
            break;
        }
        if let Err(error) = sweep_terminal_cleanup(&studio) {
            tracing::error!(%error, "terminal cleanup blocked further render claims");
            return;
        }
        match studio.database.claim_next_render_job() {
            Ok(Some(job)) => {
                if !process_claimed_job(&studio, &job, &mut shutdown).await {
                    tracing::error!(
                        "render process group cleanup could not be confirmed; worker stopped"
                    );
                    return;
                }
            }
            Ok(None) => {
                tokio::select! {
                    _ = sleep(Duration::from_secs(1)) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                tracing::error!(%error, "render queue claim failed");
                tokio::select! {
                    _ = sleep(Duration::from_secs(2)) => {}
                    _ = shutdown.changed() => break,
                }
            }
        }
    }
}

fn recover_publications(studio: &Studio) -> Result<()> {
    for job in studio.database.running_render_jobs()? {
        let Some(value) = job.publication_intent.as_deref() else {
            continue;
        };
        let intent: PublicationIntent =
            serde_json::from_str(value).context("parse durable publication intent")?;
        if let Err(error) = complete_publication(studio, &job, &intent) {
            studio.database.block_render_publication(
                &job.id,
                &format!("publication recovery blocked: {error}"),
            )?;
            bail!("publication recovery blocked for {}: {error}", job.id);
        }
        #[cfg(target_os = "linux")]
        remove_launch_handshake(studio, &job.id)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn recover_running_jobs(studio: &Studio) -> Result<()> {
    for job in studio.database.running_render_jobs()? {
        let handshake_path = launch_handshake_path(studio, &job.id);
        if job.pid.is_none() || job.pid_starttime.is_none() {
            let handshake = wait_for_recovery_handshake(&handshake_path).await;
            let handshake = match handshake {
                Ok(value) => value,
                Err(error) => {
                    let message = format!(
                        "running job has no persisted process identity and launch handshake is ambiguous: {error}"
                    );
                    studio.database.block_render_recovery(&job.id, &message)?;
                    bail!("{message}; worker stopped to prevent an unregistered executor");
                }
            };
            if let Err(error) = verify_live_handshake(&handshake_path, &handshake, &job) {
                let message = format!("launch handshake identity is ambiguous: {error}");
                studio.database.block_render_recovery(&job.id, &message)?;
                bail!("{message}");
            }
            studio.database.begin_render_recovery(&job.id)?;
            terminate_orphan_process_group(handshake.pid).await?;
            remove_launch_handshake(studio, &job.id)?;
            remove_private_job_relative(studio, &job.id, "attempt-1")?;
            studio.database.recover_render_job_after_cleanup(
                &job.id,
                !job.cancel_requested,
                "previous worker crashed before process identity persistence; handshake-bound process group was terminated",
            )?;
            continue;
        }
        let (Some(pid), Some(expected_starttime)) = (job.pid, job.pid_starttime) else {
            unreachable!()
        };
        let pid = u32::try_from(pid).context("stored renderer PID is invalid")?;
        match read_proc_identity(pid)? {
            Some(identity)
                if identity.starttime == expected_starttime as u64
                    && identity.process_group == pid =>
            {
                if let Err(error) =
                    verify_live_db_process(pid, expected_starttime as u64, &handshake_path, &job)
                {
                    let message = format!(
                        "persisted renderer process identity cannot be proven independently: {error}"
                    );
                    studio.database.block_render_recovery(&job.id, &message)?;
                    bail!("{message}");
                }
                studio.database.begin_render_recovery(&job.id)?;
                terminate_orphan_process_group(pid).await?;
                remove_launch_handshake(studio, &job.id)?;
                remove_private_job_relative(studio, &job.id, "attempt-1")?;
                studio.database.recover_render_job_after_cleanup(
                    &job.id,
                    !job.cancel_requested,
                    "previous worker crashed; verified renderer process group was terminated",
                )?;
            }
            Some(_) => {
                let message = "stored renderer PID was reused; unrelated process was not signaled";
                studio.database.block_render_recovery(&job.id, message)?;
                bail!("{message}");
            }
            None if process_group_live(pid)? => {
                let message = format!(
                    "renderer leader disappeared but process group {pid} remains; identity cannot be proven"
                );
                studio.database.block_render_recovery(&job.id, &message)?;
                bail!("{message}");
            }
            None => {
                studio.database.begin_render_recovery(&job.id)?;
                remove_launch_handshake(studio, &job.id)?;
                remove_private_job_relative(studio, &job.id, "attempt-1")?;
                studio.database.recover_render_job_after_cleanup(
                    &job.id,
                    !job.cancel_requested,
                    "previous renderer was already gone; durable recovery completed",
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_launch_handshake(studio: &Studio, job_id: &str) -> Result<()> {
    if job_id.is_empty()
        || Path::new(job_id).components().count() != 1
        || !Path::new(job_id)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("render job id is not a safe directory component");
    }
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let jobs = options.open(studio.settings.render_jobs_dir())?;
    let job_name = CString::new(job_id.as_bytes())?;
    // SAFETY: jobs is a pinned directory descriptor; O_NOFOLLOW prevents
    // replacement of the exact private job directory.
    let job_fd = unsafe {
        libc::openat(
            jobs.as_raw_fd(),
            job_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if job_fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: job_fd was freshly returned and is uniquely owned here.
    let job_dir = unsafe { File::from_raw_fd(job_fd) };
    let remove_entry = |name: &CString, description: &str| -> Result<bool> {
        // SAFETY: fstatat and unlinkat operate relative to the pinned private
        // job directory. AT_SYMLINK_NOFOLLOW prevents following substitutions.
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                job_dir.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == -1
        {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(error.into());
        }
        // SAFETY: successful fstatat initialized stat.
        let stat = unsafe { stat.assume_init() };
        if !matches!(stat.st_mode & libc::S_IFMT, libc::S_IFREG | libc::S_IFLNK) {
            bail!("{description} is neither a regular file nor a removable symlink");
        }
        if unsafe { libc::unlinkat(job_dir.as_raw_fd(), name.as_ptr(), 0) } == -1 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(true)
    };

    let mut removed = remove_entry(&CString::new("launch-handshake.json")?, "launch handshake")?;
    let pinned_job_dir = format!("/proc/self/fd/{}", job_dir.as_raw_fd());
    for entry in fs::read_dir(pinned_job_dir)? {
        let name = entry?.file_name();
        if is_wrapper_handshake_temp(&name) {
            removed |= remove_entry(
                &CString::new(name.as_encoded_bytes())?,
                "launch handshake temporary",
            )?;
        }
    }
    if removed {
        job_dir.sync_all()?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_wrapper_handshake_temp(name: &std::ffi::OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    let Some(middle) = bytes
        .strip_prefix(b".launch-handshake.")
        .and_then(|value| value.strip_suffix(b".tmp"))
    else {
        return false;
    };
    let mut parts = middle.split(|byte| *byte == b'.');
    let (Some(pid), Some(nonce), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !pid.is_empty()
        && pid.iter().all(u8::is_ascii_digit)
        && nonce.len() == 16
        && nonce
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

#[cfg(not(target_os = "linux"))]
async fn recover_running_jobs(studio: &Studio) -> Result<()> {
    if studio.database.running_render_jobs()?.is_empty() {
        Ok(())
    } else {
        bail!("running render recovery requires Linux /proc process identity")
    }
}

fn launch_handshake_path(studio: &Studio, job_id: &str) -> PathBuf {
    studio
        .settings
        .render_jobs_dir()
        .join(job_id)
        .join("launch-handshake.json")
}

#[cfg(target_os = "linux")]
fn read_launch_handshake(path: &Path) -> Result<LaunchHandshake> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options
        .open(path)
        .with_context(|| format!("open launch handshake {}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 4096 {
        bail!("launch handshake is not a bounded regular file");
    }
    let value: LaunchHandshake = serde_json::from_reader(file)?;
    Ok(value)
}

#[cfg(target_os = "linux")]
fn verify_live_handshake(
    path: &Path,
    handshake: &LaunchHandshake,
    job: &RenderJobRow,
) -> Result<ProcIdentity> {
    if handshake.job_id != job.id || handshake.executor_identity != job.renderer_hash {
        bail!("launch handshake job or executor binding mismatch");
    }
    let identity = read_proc_identity(handshake.pid)?
        .ok_or_else(|| anyhow!("handshake-bound executor process is no longer present"))?;
    if identity.starttime != handshake.starttime || identity.process_group != handshake.pid {
        bail!("launch handshake process identity mismatch");
    }
    let cmdline = fs::read(format!("/proc/{}/cmdline", handshake.pid))
        .context("read handshake executor cmdline")?;
    let arguments = cmdline
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .collect::<Vec<_>>();
    if arguments.len() < 9
        || arguments[1] != b"-c"
        || arguments[2] != EXECUTOR_WRAPPER.as_bytes()
        || arguments[6] != path.as_os_str().as_encoded_bytes()
        || arguments[7] != job.id.as_bytes()
        || arguments[8] != job.renderer_hash.as_bytes()
    {
        bail!("launch handshake fixed wrapper/job cmdline binding mismatch");
    }
    Ok(identity)
}

#[cfg(target_os = "linux")]
fn verify_live_db_process(
    pid: u32,
    expected_starttime: u64,
    handshake_path: &Path,
    job: &RenderJobRow,
) -> Result<()> {
    let identity = read_proc_identity(pid)?
        .ok_or_else(|| anyhow!("persisted executor process is no longer present"))?;
    if identity.starttime != expected_starttime || identity.process_group != pid {
        bail!("persisted executor process identity mismatch");
    }
    let cmdline =
        fs::read(format!("/proc/{pid}/cmdline")).context("read persisted executor cmdline")?;
    let arguments = cmdline
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .collect::<Vec<_>>();
    if arguments.len() < 9
        || arguments[1] != b"-c"
        || arguments[2] != EXECUTOR_WRAPPER.as_bytes()
        || arguments[6] != handshake_path.as_os_str().as_encoded_bytes()
        || arguments[7] != job.id.as_bytes()
        || arguments[8] != job.renderer_hash.as_bytes()
    {
        bail!("persisted executor fixed wrapper/job binding mismatch");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn wait_for_launch_handshake(
    path: &Path,
    expected_pid: u32,
    job: &RenderJobRow,
) -> Result<LaunchHandshake> {
    timeout(Duration::from_secs(5), async {
        loop {
            match read_launch_handshake(path) {
                Ok(handshake) => {
                    if handshake.pid != expected_pid {
                        bail!("launch handshake PID differs from spawned child");
                    }
                    let identity = verify_live_handshake(path, &handshake, job)?;
                    if identity.is_stopped() {
                        return Ok(handshake);
                    }
                    if identity.zombie {
                        bail!("executor exited before entering stopped launch state");
                    }
                }
                Err(error)
                    if error
                        .downcast_ref::<io::Error>()
                        .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) => {}
                Err(error) => return Err(error),
            }
            match read_proc_identity(expected_pid)? {
                None => bail!("executor exited before entering stopped launch state"),
                Some(identity) if identity.zombie => {
                    bail!("executor exited before entering stopped launch state")
                }
                Some(_) => {}
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("timed out waiting for renderer launch handshake")?
}

#[cfg(target_os = "linux")]
async fn wait_for_renderer_resumed(pid: u32, expected_starttime: u64) -> Result<()> {
    timeout(Duration::from_secs(1), async {
        loop {
            match read_proc_identity(pid)? {
                None => return Ok(()),
                Some(identity)
                    if identity.starttime != expected_starttime
                        || identity.process_group != pid =>
                {
                    bail!("renderer process identity changed after SIGCONT");
                }
                Some(identity) if !identity.is_stopped() => return Ok(()),
                Some(_) => sleep(Duration::from_millis(5)).await,
            }
        }
    })
    .await
    .context("renderer remained stopped after SIGCONT")?
}

#[cfg(target_os = "linux")]
async fn wait_for_recovery_handshake(path: &Path) -> Result<LaunchHandshake> {
    timeout(Duration::from_secs(5), async {
        loop {
            match read_launch_handshake(path) {
                Ok(value) => return Ok(value),
                Err(error)
                    if error
                        .downcast_ref::<io::Error>()
                        .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) => {}
                Err(error) => return Err(error),
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("timed out waiting for recovery launch handshake")?
}

async fn process_claimed_job(
    studio: &Studio,
    job: &RenderJobRow,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    if let Err(error) = run_job(studio, job, shutdown).await {
        tracing::error!(job_id = %job.id, %error, "render worker failed");
        if studio
            .database
            .render_job_by_id(&job.id)
            .ok()
            .flatten()
            .is_some_and(|row| row.status == "running" && row.publication_intent.is_some())
        {
            return false;
        }
        let possibly_live_pid = studio
            .database
            .render_job_by_id(&job.id)
            .ok()
            .flatten()
            .and_then(|row| row.pid)
            .and_then(|pid| u32::try_from(pid).ok());
        let canceled = studio
            .database
            .render_job_cancel_requested(&job.id)
            .unwrap_or(false);
        let status = if canceled { "canceled" } else { "failed" };
        if let Err(finish_error) =
            studio
                .database
                .finish_render_job(&job.id, status, None, Some(&format!("{error:#}")))
        {
            tracing::error!(job_id = %job.id, %finish_error, "failed to terminate render job");
        } else if let Err(cleanup_error) = retain_terminal_job_artifacts(studio, job) {
            tracing::error!(job_id = %job.id, %cleanup_error, "failed to apply terminal retention");
            return false;
        }
        if error
            .to_string()
            .contains("PROCESS_GROUP_CLEANUP_UNCONFIRMED")
            || possibly_live_pid
                .and_then(|pid| process_group_live(pid).ok())
                .unwrap_or(false)
        {
            return false;
        }
    }
    true
}

async fn run_job(
    studio: &Studio,
    job: &RenderJobRow,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<()> {
    let prepared = prepare_execution(studio, job)
        .context("validate private render snapshot before execution")?;
    if studio.database.render_job_cancel_requested(&job.id)? {
        studio.database.finish_render_job(
            &job.id,
            "canceled",
            None,
            Some("canceled before renderer start"),
        )?;
        retain_terminal_job_artifacts(studio, job)?;
        return Ok(());
    }
    if *shutdown.borrow() {
        studio.database.finish_render_job(
            &job.id,
            "failed",
            None,
            Some("service shutdown before renderer start"),
        )?;
        retain_terminal_job_artifacts(studio, job)?;
        return Ok(());
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&job.log_path)
        .with_context(|| format!("open render log {}", job.log_path))?;
    let log_err = log.try_clone()?;

    let renderer = open_verified_renderer(&prepared.renderer, &job.renderer_hash)?;
    let renderer_fd = renderer.as_raw_fd();
    let renderer_root = prepared
        .renderer
        .parent()
        .ok_or_else(|| anyhow!("official renderer has no parent"))?;
    let mut command = Command::new(&prepared.python);
    let handshake_path = launch_handshake_path(studio, &job.id);
    reject_symlink_components(&handshake_path, true, "launch handshake")?;
    if handshake_path.exists() {
        bail!("launch handshake already exists before renderer spawn");
    }
    command.args([
        "-c",
        EXECUTOR_WRAPPER,
        &renderer_fd.to_string(),
        &prepared.renderer.to_string_lossy(),
        &renderer_root.to_string_lossy(),
        &handshake_path.to_string_lossy(),
        &job.id,
        &job.renderer_hash,
    ]);
    match &prepared.kind {
        ExecutionKind::LegacyXry {
            edl,
            source_dir,
            input_manifest,
            work_dir,
        } => {
            command
                .args(["--encoder-profile", &job.encoder_profile])
                .arg(edl)
                .arg(&prepared.snapshot_dir)
                .arg(source_dir)
                .arg(input_manifest)
                .arg(work_dir);
        }
        ExecutionKind::VideoProject {
            canonical_edl,
            assets_root,
            output_dir,
            replay_output_dir,
        } => {
            command
                .args(["--job-id", &job.id])
                .args([
                    "--project-id",
                    job.project_id.as_deref().context("project id is missing")?,
                ])
                .args([
                    "--revision",
                    &job.project_revision
                        .context("project revision is missing")?
                        .to_string(),
                ])
                .args([
                    "--document-sha256",
                    job.document_sha
                        .as_deref()
                        .context("document hash is missing")?,
                ])
                .args(["--replay-bundle-sha256", &job.snapshot_hash])
                .arg("--replay-output")
                .arg(replay_output_dir)
                .arg(canonical_edl)
                .arg(assets_root)
                .arg(output_dir);
        }
        ExecutionKind::Media {
            request,
            input,
            output_dir,
            funclip_root,
            ..
        } => {
            if let Some(root) = funclip_root {
                command.env("VWA_FUNCLIP_ROOT", root);
            } else {
                command.env_remove("VWA_FUNCLIP_ROOT");
            }
            command.arg(request).arg(input).arg(output_dir);
        }
    }
    command
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .kill_on_drop(true);
    #[cfg(test)]
    if let Some(hook) = take_test_launch_hook(&job.id) {
        if hook.delay_before_stop_ms > 0 {
            command.env(
                "VWA_TEST_HANDSHAKE_DELAY_BEFORE_STOP_MS",
                hook.delay_before_stop_ms.to_string(),
            );
        }
        if hook.exit_before_stop {
            command.env("VWA_TEST_HANDSHAKE_EXIT_BEFORE_STOP", "1");
        }
    }
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().context("spawn official XRY renderer")?;
    let pid = child
        .id()
        .ok_or_else(|| anyhow!("renderer PID unavailable"))?;
    let mut process = ProcessGroupGuard::new(child, pid);
    let handshake = match wait_for_launch_handshake(&handshake_path, pid, job).await {
        Ok(value) => value,
        Err(error) => {
            let cleanup = process.terminate().await;
            bail!("renderer launch handshake failed: {error}{cleanup}");
        }
    };
    let starttime = handshake.starttime;
    if !studio
        .database
        .set_render_job_process(&job.id, pid, starttime)?
    {
        let cleanup = process.terminate().await;
        let cleanup_unconfirmed = process_group_live(pid).unwrap_or(true);
        if cleanup_unconfirmed {
            bail!("render job became terminal after spawn{cleanup}; PROCESS_GROUP_CLEANUP_UNCONFIRMED");
        }
        bail!("render job became terminal after spawn{cleanup}");
    }
    let attempt_dir = match &prepared.kind {
        ExecutionKind::VideoProject { output_dir, .. }
        | ExecutionKind::Media { output_dir, .. } => output_dir.parent(),
        ExecutionKind::LegacyXry { .. } => None,
    };
    if let Some(attempt_dir) = attempt_dir {
        create_private_directory(attempt_dir)?;
        sync_directory(
            attempt_dir
                .parent()
                .context("private render attempt has no job parent")?,
        )?;
    }
    // SAFETY: the verified handshake proves this PID and process group belong
    // to this exact job and executor. The child stopped itself before reading
    // or executing renderer code.
    if unsafe { libc::kill(pid as i32, libc::SIGCONT) } == -1 {
        let error = io::Error::last_os_error();
        let cleanup = process.terminate().await;
        bail!("continue registered renderer: {error}{cleanup}");
    }
    if let Err(error) = wait_for_renderer_resumed(pid, starttime).await {
        let cleanup = process.terminate().await;
        bail!("renderer did not resume after registration: {error}{cleanup}");
    }

    let deadline = Instant::now() + Duration::from_secs(prepared.timeout_seconds);
    loop {
        if studio.database.render_job_cancel_requested(&job.id)? {
            let cleanup = process.terminate().await;
            let cleanup_unconfirmed = process_group_live(pid).unwrap_or(true);
            studio.database.finish_render_job(
                &job.id,
                "canceled",
                None,
                Some(&format!("canceled by request{cleanup}")),
            )?;
            if cleanup_unconfirmed {
                bail!("PROCESS_GROUP_CLEANUP_UNCONFIRMED");
            }
            break;
        }
        if *shutdown.borrow() {
            let cleanup = process.terminate().await;
            let cleanup_unconfirmed = process_group_live(pid).unwrap_or(true);
            studio.database.finish_render_job(
                &job.id,
                "failed",
                None,
                Some(&format!("service shutdown interrupted render{cleanup}")),
            )?;
            if cleanup_unconfirmed {
                bail!("PROCESS_GROUP_CLEANUP_UNCONFIRMED");
            }
            break;
        }
        if Instant::now() >= deadline {
            let cleanup = process.terminate().await;
            let cleanup_unconfirmed = process_group_live(pid).unwrap_or(true);
            studio.database.finish_render_job(
                &job.id,
                "failed",
                None,
                Some(&format!("render timeout exceeded{cleanup}")),
            )?;
            if cleanup_unconfirmed {
                bail!("PROCESS_GROUP_CLEANUP_UNCONFIRMED");
            }
            break;
        }
        match process.child_mut().try_wait() {
            Ok(Some(status)) => {
                let cleanup = process.terminate().await;
                let cleanup_unconfirmed = process_group_live(pid).unwrap_or(true);
                let mut publication_intent = None;
                let identity_error = if hash_file(&prepared.renderer)? != job.renderer_hash {
                    Some("official renderer changed while the job was executing")
                } else {
                    match &prepared.kind {
                        ExecutionKind::LegacyXry { .. }
                            if hash_snapshot(&prepared.snapshot_dir)? != job.snapshot_hash =>
                        {
                            Some("private render snapshot changed while the job was executing")
                        }
                        ExecutionKind::VideoProject {
                            canonical_edl,
                            assets_root,
                            output_dir,
                            replay_output_dir,
                            ..
                        } => {
                            if project_render::validate_snapshot(canonical_edl, &job.snapshot_hash)
                                .is_err()
                                || project_render::revalidate_assets(canonical_edl, assets_root)
                                    .is_err()
                            {
                                Some(
                                    "video project bundle or assets changed while the job was executing",
                                )
                            } else {
                                match validate_video_project_report(
                                    job,
                                    canonical_edl,
                                    output_dir,
                                    replay_output_dir,
                                ) {
                                    Ok(attestation) => {
                                        let files =
                                            video_project_publication_files(job, output_dir)?;
                                        publication_intent = Some(PublicationIntent {
                                            schema_version: 1,
                                            job_id: job.id.clone(),
                                            kind: job.kind.clone(),
                                            project_id: job.project_id.clone(),
                                            project_revision: job.project_revision,
                                            document_sha256: job.document_sha.clone(),
                                            attestation: Some(serde_json::to_value(attestation)?),
                                            files,
                                        });
                                        None
                                    }
                                    Err(error) => {
                                        tracing::error!(job_id = %job.id, %error, "video project report or replay validation failed");
                                        Some("video project report or deterministic replay failed validation")
                                    }
                                }
                            }
                        }
                        ExecutionKind::Media {
                            request,
                            input,
                            output_dir,
                            ..
                        } => {
                            if hash_media_snapshot(&prepared.snapshot_dir)? != job.snapshot_hash {
                                Some("private media snapshot changed while executing")
                            } else {
                                match finalize_media_result(&job.kind, request, input, output_dir) {
                                    Ok((result_or_attestation, files)) => {
                                        publication_intent = Some(PublicationIntent {
                                            schema_version: 1,
                                            job_id: job.id.clone(),
                                            kind: job.kind.clone(),
                                            project_id: job.project_id.clone(),
                                            project_revision: job.project_revision,
                                            document_sha256: job.document_sha.clone(),
                                            attestation: (job.kind == "cover")
                                                .then_some(result_or_attestation),
                                            files,
                                        });
                                        None
                                    }
                                    Err(error) => {
                                        tracing::error!(job_id = %job.id, %error, "media result validation failed");
                                        Some("media result failed validation")
                                    }
                                }
                            }
                        }
                        _ => None,
                    }
                };
                let succeeded = status.success() && cleanup.is_empty() && identity_error.is_none();
                let error = if succeeded {
                    None
                } else if let Some(error) = identity_error {
                    Some(error)
                } else if status.success() {
                    Some(cleanup.as_str())
                } else {
                    Some("official XRY renderer exited unsuccessfully")
                };
                if succeeded {
                    if let Some(intent) = publication_intent {
                        let intent_json = serde_json::to_string(&intent)?;
                        let outcome = studio.database.set_render_publication_intent(
                            &job.id,
                            status.code(),
                            &intent_json,
                        )?;
                        match outcome {
                            PublicationIntentOutcome::Entered => {
                                if cleanup_unconfirmed {
                                    bail!("PROCESS_GROUP_CLEANUP_UNCONFIRMED");
                                }
                                if let Err(error) = complete_publication(studio, job, &intent) {
                                    studio.database.block_render_publication(
                                        &job.id,
                                        &format!("publication blocked: {error}"),
                                    )?;
                                    return Err(error.context("complete durable publication"));
                                }
                            }
                            PublicationIntentOutcome::CancelWon => {
                                // The database committed canceled before any
                                // project-owned publication could begin.
                            }
                            PublicationIntentOutcome::Stale => {
                                bail!("render publication state changed before intent CAS");
                            }
                        }
                    } else {
                        // Legacy XRY remains a read-compatible adapter and has
                        // no project-owned publication surface.
                        studio.database.finish_render_job(
                            &job.id,
                            "succeeded",
                            status.code(),
                            None,
                        )?;
                    }
                } else {
                    studio
                        .database
                        .finish_render_job(&job.id, "failed", status.code(), error)?;
                }
                if cleanup_unconfirmed {
                    bail!("PROCESS_GROUP_CLEANUP_UNCONFIRMED");
                }
                break;
            }
            Ok(None) => {}
            Err(error) => {
                let cleanup = process.terminate().await;
                let cleanup_unconfirmed = process_group_live(pid).unwrap_or(true);
                studio.database.finish_render_job(
                    &job.id,
                    "failed",
                    None,
                    Some(&format!("wait for official XRY renderer: {error}{cleanup}")),
                )?;
                if cleanup_unconfirmed {
                    bail!("PROCESS_GROUP_CLEANUP_UNCONFIRMED");
                }
                break;
            }
        }
        tokio::select! {
            _ = sleep(Duration::from_millis(100)) => {}
            _ = shutdown.changed() => {}
        }
    }
    retain_terminal_job_artifacts(studio, job)?;
    Ok(())
}

fn retain_terminal_job_artifacts(studio: &Studio, job: &RenderJobRow) -> Result<()> {
    let current = studio
        .database
        .render_job_by_id(&job.id)?
        .context("terminal render job disappeared")?;
    if !matches!(current.status.as_str(), "succeeded" | "failed" | "canceled") {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        remove_launch_handshake(studio, &job.id)?;
        for relative in match job.kind.as_str() {
            "video_project" => vec!["snapshot/assets", "attempt-1"],
            "cover" => vec!["snapshot/input.media", "attempt-1"],
            "analysis_frames" | "safe_trims" => vec!["snapshot/input.media"],
            _ => Vec::new(),
        } {
            remove_private_job_relative(studio, &job.id, relative)?;
        }
    }
    if current.cleanup_pending {
        studio.database.clear_render_cleanup_pending(&current.id)?;
    }
    Ok(())
}

fn sweep_terminal_cleanup(studio: &Studio) -> Result<()> {
    for job in studio.database.cleanup_pending_render_jobs()? {
        retain_terminal_job_artifacts(studio, &job)
            .with_context(|| format!("complete terminal cleanup for render job {}", job.id))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_private_job_relative(studio: &Studio, job_id: &str, relative: &str) -> Result<()> {
    use std::ffi::{CStr, CString};
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    fn open_dir_at(parent: i32, name: &CStr) -> io::Result<File> {
        // SAFETY: parent is a live directory descriptor and name is a bounded
        // C string. O_NOFOLLOW prevents directory-link substitution.
        let fd = unsafe {
            libc::openat(
                parent,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: fd was newly returned and is uniquely owned.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    fn remove_at(parent: i32, name: &CStr) -> Result<()> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                parent,
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == -1
        {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error.into());
        }
        // SAFETY: successful fstatat initialized stat.
        let stat = unsafe { stat.assume_init() };
        match stat.st_mode & libc::S_IFMT {
            libc::S_IFREG => {
                if unsafe { libc::unlinkat(parent, name.as_ptr(), 0) } == -1 {
                    return Err(io::Error::last_os_error().into());
                }
            }
            libc::S_IFDIR => {
                let directory = open_dir_at(parent, name)?;
                // SAFETY: fcntl duplicates a live descriptor for fdopendir,
                // which takes ownership of the duplicate.
                let duplicate =
                    unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
                if duplicate < 0 {
                    return Err(io::Error::last_os_error().into());
                }
                // SAFETY: duplicate is owned by the DIR stream on success.
                let stream = unsafe { libc::fdopendir(duplicate) };
                if stream.is_null() {
                    // SAFETY: fdopendir failed and did not take ownership.
                    unsafe { libc::close(duplicate) };
                    return Err(io::Error::last_os_error().into());
                }
                loop {
                    // SAFETY: stream remains valid until closed below.
                    let entry = unsafe { libc::readdir(stream) };
                    if entry.is_null() {
                        break;
                    }
                    // SAFETY: d_name is NUL terminated by readdir.
                    let child = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
                    if child.to_bytes() == b"." || child.to_bytes() == b".." {
                        continue;
                    }
                    if let Err(error) = remove_at(directory.as_raw_fd(), child) {
                        // SAFETY: stream owns the duplicate descriptor.
                        unsafe { libc::closedir(stream) };
                        return Err(error);
                    }
                }
                // SAFETY: stream is valid and closed exactly once.
                if unsafe { libc::closedir(stream) } == -1 {
                    return Err(io::Error::last_os_error().into());
                }
                directory.sync_all()?;
                if unsafe { libc::unlinkat(parent, name.as_ptr(), libc::AT_REMOVEDIR) } == -1 {
                    return Err(io::Error::last_os_error().into());
                }
            }
            _ => bail!("private retention cleanup refuses unsupported file type"),
        }
        Ok(())
    }

    let components = Path::new(relative)
        .components()
        .map(|component| match component {
            Component::Normal(value) => CString::new(value.as_bytes()).map_err(Into::into),
            _ => bail!("invalid private retention path"),
        })
        .collect::<Result<Vec<CString>>>()?;
    let (last, parents) = components
        .split_last()
        .ok_or_else(|| anyhow!("private retention path is empty"))?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let jobs = options.open(studio.settings.render_jobs_dir())?;
    let job_name = CString::new(job_id)?;
    let mut directory = open_dir_at(jobs.as_raw_fd(), &job_name)?;
    for component in parents {
        directory = open_dir_at(directory.as_raw_fd(), component)?;
    }
    remove_at(directory.as_raw_fd(), last)?;
    directory.sync_all()?;
    Ok(())
}

fn validate_video_project_report(
    job: &RenderJobRow,
    canonical_edl: &Path,
    output_dir: &Path,
    replay_output_dir: &Path,
) -> Result<quality::TrustedRenderAttestation> {
    let report_path = canonical_file_under_without_symlinks(
        &output_dir.join("render-report.json"),
        output_dir,
        "video project render report",
    )?;
    let report_bytes = fs::read(&report_path)?;
    let report_sha256 = format!("{:x}", Sha256::digest(&report_bytes));
    let report: Value = serde_json::from_slice(&report_bytes)?;
    if report["schema_version"] != 2
        || report["job_id"].as_str() != Some(&job.id)
        || report["project_id"].as_str() != job.project_id.as_deref()
        || report["revision"].as_i64() != job.project_revision
        || report["document_sha256"].as_str() != job.document_sha.as_deref()
        || report["replay_bundle_sha256"].as_str() != Some(&job.snapshot_hash)
    {
        bail!("render report trusted job binding mismatch");
    }
    let replay = report["replay"]
        .as_object()
        .context("render report lacks replay evidence")?;
    if replay.get("executed") != Some(&Value::Bool(true))
        || replay.get("deterministic_executor") != Some(&Value::Bool(true))
        || replay.get("job_id").and_then(Value::as_str) != Some(&job.id)
        || replay.get("project_id").and_then(Value::as_str) != job.project_id.as_deref()
        || replay.get("revision").and_then(Value::as_i64) != job.project_revision
        || replay.get("document_sha256").and_then(Value::as_str) != job.document_sha.as_deref()
        || replay.get("replay_bundle_sha256").and_then(Value::as_str) != Some(&job.snapshot_hash)
    {
        bail!("deterministic replay trusted binding mismatch");
    }
    let mut primary = std::collections::BTreeMap::new();
    for entry in fs::read_dir(output_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".mp4") {
            primary.insert(name, hash_file(&entry.path())?);
        }
    }
    let mut replay_actual = std::collections::BTreeMap::new();
    for entry in fs::read_dir(replay_output_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".mp4") {
            replay_actual.insert(name, hash_file(&entry.path())?);
        }
    }
    if primary.is_empty() || primary != replay_actual {
        bail!("primary and replay output hashes differ");
    }
    let primary_report: std::collections::BTreeMap<String, String> =
        serde_json::from_value(replay["primary_sha256"].clone())?;
    let replay_report: std::collections::BTreeMap<String, String> =
        serde_json::from_value(replay["replay_sha256"].clone())?;
    if primary_report != primary || replay_report != replay_actual {
        bail!("render report replay mappings do not match actual outputs");
    }
    let output_sha256: std::collections::BTreeMap<String, String> =
        serde_json::from_value(report["output_sha256"].clone())?;
    let expected = primary
        .iter()
        .map(|(name, hash)| (format!("exports/{}/{name}", job.id), hash.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    if output_sha256 != expected {
        bail!("render report project-relative output mapping mismatch");
    }
    let canonical = serde_json::to_vec(&output_sha256)?;
    let mut hasher = Sha256::new();
    hasher.update(canonical);
    if report["canonical_output_sha256"].as_str()
        != Some(format!("{:x}", hasher.finalize()).as_str())
    {
        bail!("canonical output mapping hash mismatch");
    }
    let project_prefix = format!("exports/{}/", job.id);
    let replay_project = replay_actual
        .iter()
        .map(|(name, hash)| (format!("{project_prefix}{name}"), hash.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let plan: Value = serde_json::from_slice(&fs::read(canonical_edl)?)?;
    let variants: Vec<crate::timeline::VariantSpec> =
        serde_json::from_value(plan["timeline"]["variants"].clone())?;
    let mut trusted_variants = std::collections::BTreeMap::new();
    for (index, variant) in variants.into_iter().enumerate() {
        let key = crate::timeline::variant_key(index, &variant);
        let relative = format!("{project_prefix}{key}.mp4");
        let sha256 = output_sha256
            .get(&relative)
            .with_context(|| format!("render output lacks declared variant '{key}'"))?
            .clone();
        trusted_variants.insert(
            key,
            quality::TrustedVariant {
                index,
                language: variant.language,
                aspect: variant.aspect,
                watermark: variant.watermark,
                cta: variant.cta,
                output_relative: relative,
                sha256,
            },
        );
    }
    let master_relative = format!("{project_prefix}master.mp4");
    if !output_sha256.contains_key(&master_relative)
        || output_sha256.len() != trusted_variants.len() + 1
    {
        bail!("render output set does not exactly match master plus declared variants");
    }
    let reported_variants: std::collections::BTreeMap<String, quality::TrustedVariant> =
        serde_json::from_value(report["variants"].clone())?;
    if reported_variants != trusted_variants {
        bail!("render report variant identity bindings do not match canonical EDL");
    }
    Ok(quality::TrustedRenderAttestation {
        report_relative: format!("{project_prefix}render-report.json"),
        report_sha256,
        bundle_sha256: job.snapshot_hash.clone(),
        output_sha256,
        replay_sha256: replay_project,
        replay_verified: true,
        master_relative,
        variants: trusted_variants,
    })
}

fn video_project_publication_files(
    job: &RenderJobRow,
    primary_output_dir: &Path,
) -> Result<Vec<PublicationFile>> {
    let job_dir = primary_output_dir
        .parent()
        .and_then(Path::parent)
        .context("video project primary output is not inside its job")?;
    let mut paths = fs::read_dir(primary_output_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut files = Vec::new();
    for source in paths {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .context("video project output filename is invalid")?;
        if name != "render-report.json" && !name.ends_with(".mp4") {
            bail!("video project attempt contains an unexpected output");
        }
        files.push(publication_file(
            job_dir,
            &source,
            &format!("exports/{}/{name}", job.id),
        )?);
    }
    if files.is_empty() {
        bail!("video project publication has no files");
    }
    Ok(files)
}

fn finalize_media_result(
    kind: &str,
    request_path: &Path,
    input: &Path,
    output_dir: &Path,
) -> Result<(Value, Vec<PublicationFile>)> {
    let (request_bytes, _) = provenance::read_regular_with_sha256(request_path)?;
    let request: Value = serde_json::from_slice(&request_bytes)?;
    let report_path = output_dir.join("media-report.json");
    let (report_bytes, report_sha256) = provenance::read_regular_with_sha256(&report_path)?;
    let report: Value = serde_json::from_slice(&report_bytes)?;
    if report.get("kind").and_then(Value::as_str) != Some(kind) {
        bail!("media report kind mismatch");
    }
    let duration = report
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .context("media report duration is invalid")?;
    let input_sha256 = hash_file(input)?;
    let result = match kind {
        "analysis_frames" => {
            if request
                .get("asr_segments")
                .and_then(Value::as_array)
                .is_some_and(|segments| {
                    segments.iter().any(|segment| {
                        segment
                            .get("end_seconds")
                            .and_then(Value::as_f64)
                            .is_none_or(|end| end > duration)
                    })
                })
            {
                bail!("ASR segment exceeds actual media duration");
            }
            let frames = report
                .get("frames")
                .and_then(Value::as_array)
                .context("analysis report lacks frames")?;
            let expected = request
                .get("max_frames")
                .and_then(Value::as_u64)
                .context("analysis request lacks max_frames")? as usize;
            if frames.len() != expected {
                bail!("analysis frame count mismatch");
            }
            for (index, frame) in frames.iter().enumerate() {
                let seconds = frame
                    .get("timestamp_seconds")
                    .and_then(Value::as_f64)
                    .context("analysis timestamp missing")?;
                if !(0.0..duration).contains(&seconds) {
                    bail!("analysis timestamp must be strictly before EOF");
                }
                let expected_name = format!("analysis-frame-{index:03}.jpg");
                let name = frame
                    .get("file")
                    .and_then(Value::as_str)
                    .context("analysis frame filename missing")?;
                if name != expected_name {
                    bail!("analysis frame filename is not deterministic");
                }
                let actual = hash_file(&output_dir.join(name))?;
                if frame.get("sha256").and_then(Value::as_str) != Some(&actual) {
                    bail!("analysis frame hash mismatch");
                }
            }
            json!({
                "kind": kind,
                "input_sha256": input_sha256,
                "media_report_sha256": report_sha256,
                "duration_seconds": duration,
                "frames": frames,
                "capability": {
                    "extraction": "available",
                    "vlm_analysis": "not_provided"
                }
            })
        }
        "safe_trims" => {
            let requested_start = request
                .get("requested_start")
                .and_then(Value::as_f64)
                .context("requested_start missing")?;
            let requested_end = request
                .get("requested_end")
                .and_then(Value::as_f64)
                .context("requested_end missing")?;
            if requested_end > duration {
                bail!("requested trim range exceeds actual media duration");
            }
            let supplied_words = request.get("words").cloned().unwrap_or_else(|| json!([]));
            let words_value = if supplied_words
                .as_array()
                .is_some_and(|items| items.is_empty())
            {
                report
                    .get("words")
                    .cloned()
                    .context("safe trim report lacks FunClip word timestamps")?
            } else {
                supplied_words
            };
            let words: Vec<alignment::WordTimestamp> = serde_json::from_value(words_value)?;
            if words.iter().any(|word| word.end > duration) {
                bail!("word timestamp exceeds actual media duration");
            }
            let silent_intervals = serde_json::from_value(
                report
                    .get("silent_intervals")
                    .cloned()
                    .context("silencedetect report lacks silent_intervals")?,
            )?;
            let voiced_intervals = serde_json::from_value(
                report
                    .get("voiced_intervals")
                    .cloned()
                    .context("silencedetect report lacks voiced_intervals")?,
            )?;
            let trim = alignment::trim_silence_safe(&alignment::TrimRequest {
                requested_start,
                requested_end,
                search_radius: request
                    .get("search_radius")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5),
                words,
                voiced_intervals,
                silent_intervals,
            })?;
            json!({
                "kind": kind,
                "input_sha256": input_sha256,
                "media_report_sha256": report_sha256,
                "duration_seconds": duration,
                "trim": trim,
                "capability": {
                    "vad_alignment": "ffmpeg_silencedetect",
                    "word_level_asr_backend": report
                        .pointer("/capability/word_level_asr_backend")
                        .and_then(Value::as_str)
                        .context("safe trim report lacks word timestamp capability")?,
                    "segment_level_asr_backend": "funclip",
                    "timestamp_interpolation": "not_performed"
                }
            })
        }
        "cover" => {
            let stem = request
                .get("stem")
                .and_then(Value::as_str)
                .context("cover stem missing")?;
            let original_name = format!("{stem}-cover-original.png");
            let final_name = format!("{stem}.jpg");
            let original = output_dir.join(&original_name);
            let final_jpg = output_dir.join(&final_name);
            let original_sha256 = hash_file(&original)?;
            let final_sha256 = hash_file(&final_jpg)?;
            if report.get("original_sha256").and_then(Value::as_str) != Some(&original_sha256)
                || report.get("final_sha256").and_then(Value::as_str) != Some(&final_sha256)
            {
                bail!("cover report hash mismatch");
            }
            quality::decode_cover_image(&original, "rendered cover original")?;
            quality::decode_cover_image(&final_jpg, "rendered cover JPEG")?;
            for field in [
                "project_id",
                "project_revision",
                "document_sha256",
                "variant_key",
                "variant",
                "spec",
            ] {
                if request.get(field).is_none() {
                    bail!("cover request lacks trusted {field} binding");
                }
            }
            if report.get("layout_profile") != request.pointer("/spec/layout_profile")
                || report.get("frame_timestamp") != request.pointer("/spec/frame_timestamp")
            {
                bail!("cover renderer report differs from the bound CoverSpec");
            }
            json!({
                "kind": kind,
                "project_id": request.get("project_id"),
                "revision": request.get("project_revision"),
                "document_sha256": request.get("document_sha256"),
                "variant_key": request.get("variant_key"),
                "variant": request.get("variant"),
                "spec": request.get("spec"),
                "input_sha256": input_sha256,
                "media_report_sha256": report_sha256,
                "duration_seconds": duration,
                "original_png": format!("exports/{original_name}"),
                "final_jpg": format!("exports/{final_name}"),
                "original_sha256": original_sha256,
                "final_sha256": final_sha256,
                "layout_profile": report.get("layout_profile"),
                "frame_timestamp": report.get("frame_timestamp")
            })
        }
        _ => bail!("unsupported media result kind"),
    };
    write_new_synced(
        &output_dir.join("result.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    sync_directory(output_dir)?;
    let job_dir = output_dir
        .parent()
        .and_then(Path::parent)
        .context("media attempt output is not inside its job directory")?;
    if kind == "cover" {
        let original_relative = result["original_png"]
            .as_str()
            .context("cover result lacks original path")?;
        let final_relative = result["final_jpg"]
            .as_str()
            .context("cover result lacks final path")?;
        let source_original = output_dir.join(
            Path::new(original_relative)
                .file_name()
                .context("cover original has no filename")?,
        );
        let source_final = output_dir.join(
            Path::new(final_relative)
                .file_name()
                .context("cover final has no filename")?,
        );
        let files = vec![
            publication_file(job_dir, &source_original, original_relative)?,
            publication_file(job_dir, &source_final, final_relative)?,
        ];
        Ok((
            json!({
                "project_id": result.get("project_id"),
                "revision": result.get("revision"),
                "document_sha256": result.get("document_sha256"),
                "variant_key": result.get("variant_key"),
                "variant": result.get("variant"),
                "spec": result.get("spec"),
                "original_relative": result.get("original_png"),
                "final_relative": result.get("final_jpg"),
                "original_sha256": result.get("original_sha256"),
                "final_sha256": result.get("final_sha256"),
            }),
            files,
        ))
    } else {
        Ok((result, Vec::new()))
    }
}

fn publication_file(
    job_dir: &Path,
    source: &Path,
    destination_relative: &str,
) -> Result<PublicationFile> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("publication source must be a regular private file");
    }
    Ok(PublicationFile {
        source_relative: relative_path_string(source.strip_prefix(job_dir)?),
        destination_relative: relative_path_string(&safe_relative(destination_relative)?),
        sha256: hash_file(source)?,
        size: metadata.len(),
    })
}

fn safe_relative(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("publication path must be a safe relative path");
    }
    Ok(path.to_path_buf())
}

fn relative_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn complete_publication(
    studio: &Studio,
    job: &RenderJobRow,
    intent: &PublicationIntent,
) -> Result<()> {
    if intent.schema_version != 1
        || intent.job_id != job.id
        || intent.kind != job.kind
        || intent.project_id != job.project_id
        || intent.project_revision != job.project_revision
        || intent.document_sha256 != job.document_sha
    {
        bail!("publication intent job binding mismatch");
    }
    if !intent.files.is_empty() && intent.project_id.is_none() {
        bail!("publication files require a bound project");
    }
    let mut destinations = std::collections::BTreeSet::new();
    for (index, file) in intent.files.iter().enumerate() {
        safe_relative(&file.source_relative)?;
        safe_relative(&file.destination_relative)?;
        if file.size == 0 || !is_sha256(&file.sha256) {
            bail!("publication file identity is invalid");
        }
        if !destinations.insert(&file.destination_relative) {
            bail!("publication destination is duplicated");
        }
        publish_one_file(studio, job, intent, index, file)?;
    }
    let attestation_json = intent
        .attestation
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    studio
        .database
        .complete_render_publication(&job.id, attestation_json.as_deref())?;
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(target_os = "linux")]
fn publish_one_file(
    studio: &Studio,
    job: &RenderJobRow,
    intent: &PublicationIntent,
    index: usize,
    file: &PublicationFile,
) -> Result<()> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    fn c_name(name: &std::ffi::OsStr) -> Result<CString> {
        CString::new(name.as_bytes()).context("publication component contains NUL")
    }
    fn open_dir_path(path: &Path) -> Result<File> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        Ok(options.open(path)?)
    }
    fn open_dir_at(parent: i32, name: &std::ffi::OsStr, create: bool) -> Result<File> {
        let name = c_name(name)?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: parent is pinned and name is a bounded C string.
        let mut fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
        if fd < 0 && create && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            // SAFETY: mkdirat is confined to the pinned parent.
            if unsafe { libc::mkdirat(parent, name.as_ptr(), 0o700) } == -1 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(error.into());
                }
            }
            // SAFETY: the newly created/existing entry is reopened nofollow.
            fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
        }
        if fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: fd is freshly returned and uniquely owned.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    fn open_regular_at(parent: i32, name: &std::ffi::OsStr) -> Result<File> {
        let name = c_name(name)?;
        // SAFETY: openat is relative to a pinned parent and rejects symlinks.
        let fd = unsafe {
            libc::openat(
                parent,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: fd is freshly returned and uniquely owned.
        let file = unsafe { File::from_raw_fd(fd) };
        if !file.metadata()?.is_file() {
            bail!("publication entry is not a regular file");
        }
        Ok(file)
    }
    fn verify_file(mut file: File, expected: &PublicationFile) -> Result<()> {
        if file.metadata()?.len() != expected.size {
            bail!("publication file size mismatch");
        }
        file.seek(io::SeekFrom::Start(0))?;
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher)?;
        if format!("{:x}", hasher.finalize()) != expected.sha256 {
            bail!("publication file SHA-256 mismatch");
        }
        Ok(())
    }

    let source_relative = safe_relative(&file.source_relative)?;
    let destination_relative = safe_relative(&file.destination_relative)?;
    let source_components = source_relative.components().collect::<Vec<_>>();
    let destination_components = destination_relative.components().collect::<Vec<_>>();
    let (source_name, source_parents) = source_components
        .split_last()
        .context("publication source path is empty")?;
    let (destination_name, destination_parents) = destination_components
        .split_last()
        .context("publication destination path is empty")?;

    let jobs = open_dir_path(&studio.settings.render_jobs_dir())?;
    let mut source_dir = open_dir_at(jobs.as_raw_fd(), std::ffi::OsStr::new(&job.id), false)?;
    for component in source_parents {
        source_dir = open_dir_at(source_dir.as_raw_fd(), component.as_os_str(), false)?;
    }
    let mut source = open_regular_at(source_dir.as_raw_fd(), source_name.as_os_str())?;
    verify_file(source.try_clone()?, file)?;

    let project_id = intent
        .project_id
        .as_deref()
        .context("publication project id is missing")?;
    let projects = open_dir_path(&studio.settings.video_projects_dir)?;
    let mut destination_dir = open_dir_at(
        projects.as_raw_fd(),
        std::ffi::OsStr::new(project_id),
        false,
    )?;
    for component in destination_parents {
        destination_dir = open_dir_at(destination_dir.as_raw_fd(), component.as_os_str(), true)?;
    }

    match open_regular_at(destination_dir.as_raw_fd(), destination_name.as_os_str()) {
        Ok(existing) => {
            verify_file(existing, file)?;
            return Ok(());
        }
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) => {}
        Err(error) => return Err(error),
    }

    let temp_name = CString::new(format!(".publish-{}-{index}.tmp", job.id))?;
    // Remove only a previous deterministic private publication temp. Refuse
    // non-regular entries rather than following or replacing them.
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            destination_dir.as_raw_fd(),
            temp_name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
    {
        // SAFETY: successful fstatat initialized stat.
        let stat = unsafe { stat.assume_init() };
        if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
            bail!("publication temporary path is not a regular file");
        }
        if unsafe { libc::unlinkat(destination_dir.as_raw_fd(), temp_name.as_ptr(), 0) } == -1 {
            return Err(io::Error::last_os_error().into());
        }
        destination_dir.sync_all()?;
    } else if io::Error::last_os_error().kind() != io::ErrorKind::NotFound {
        return Err(io::Error::last_os_error().into());
    }

    // SAFETY: create-new is confined to the pinned destination parent.
    let temp_fd = unsafe {
        libc::openat(
            destination_dir.as_raw_fd(),
            temp_name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if temp_fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: temp_fd is freshly returned and uniquely owned.
    let mut temporary = unsafe { File::from_raw_fd(temp_fd) };
    source.seek(io::SeekFrom::Start(0))?;
    let copied = io::copy(&mut source, &mut temporary)?;
    if copied != file.size {
        bail!("publication source changed while copying");
    }
    temporary.sync_all()?;
    drop(temporary);
    verify_file(
        open_regular_at(
            destination_dir.as_raw_fd(),
            std::ffi::OsStr::new(temp_name.to_str()?),
        )?,
        file,
    )?;

    let destination_c = c_name(destination_name.as_os_str())?;
    // SAFETY: both names are relative to the same pinned parent and
    // RENAME_NOREPLACE forbids overwriting a concurrently created file.
    if unsafe {
        libc::renameat2(
            destination_dir.as_raw_fd(),
            temp_name.as_ptr(),
            destination_dir.as_raw_fd(),
            destination_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == -1
    {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error.into());
        }
        if unsafe { libc::unlinkat(destination_dir.as_raw_fd(), temp_name.as_ptr(), 0) } == -1
            && io::Error::last_os_error().kind() != io::ErrorKind::NotFound
        {
            return Err(io::Error::last_os_error().into());
        }
    }
    destination_dir.sync_all()?;
    verify_file(
        open_regular_at(destination_dir.as_raw_fd(), destination_name.as_os_str())?,
        file,
    )
}

#[cfg(not(target_os = "linux"))]
fn publish_one_file(
    _studio: &Studio,
    _job: &RenderJobRow,
    _intent: &PublicationIntent,
    _index: usize,
    _file: &PublicationFile,
) -> Result<()> {
    bail!("durable no-replace publication requires Linux openat/renameat2")
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) -> Option<String> {
    // SAFETY: the renderer is spawned into a process group whose ID is its PID.
    if unsafe { libc::killpg(pid as i32, signal) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Some(format!("signal render process group: {error}"));
        }
    }
    None
}

#[cfg(not(unix))]
fn signal_process_group(_pid: u32, _signal: i32) -> Option<String> {
    None
}

struct ProcessGroupGuard {
    child: Option<tokio::process::Child>,
    pid: u32,
}

impl ProcessGroupGuard {
    fn new(child: tokio::process::Child, pid: u32) -> Self {
        Self {
            child: Some(child),
            pid,
        }
    }

    fn child_mut(&mut self) -> &mut tokio::process::Child {
        self.child.as_mut().expect("process guard child")
    }

    async fn terminate(&mut self) -> String {
        let Some(child) = self.child.as_mut() else {
            return String::new();
        };
        let result = terminate_process_group(child, self.pid).await;
        self.child = None;
        result
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let _ = signal_process_group(self.pid, libc::SIGTERM);
        for _ in 0..10 {
            if !process_group_live(self.pid).unwrap_or(true) {
                let _ = child.try_wait();
                self.child = None;
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = signal_process_group(self.pid, libc::SIGKILL);
        for _ in 0..20 {
            let _ = child.try_wait();
            if !process_group_live(self.pid).unwrap_or(true) {
                self.child = None;
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

async fn terminate_process_group(child: &mut tokio::process::Child, pid: u32) -> String {
    let mut errors = Vec::new();
    if let Some(error) = signal_process_group(pid, libc::SIGTERM) {
        errors.push(error);
    }
    match timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => errors.push(format!("wait after SIGTERM: {error}")),
        Err(_) => {
            if let Some(error) = signal_process_group(pid, libc::SIGKILL) {
                errors.push(error);
            }
            if let Err(error) = child.wait().await {
                errors.push(format!("wait after SIGKILL: {error}"));
            }
        }
    }
    if let Some(error) = signal_process_group(pid, libc::SIGKILL) {
        errors.push(error);
    }
    match timeout(Duration::from_secs(2), async {
        loop {
            if !process_group_live(pid)? {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => errors.push(format!("inspect process group: {error}")),
        Err(_) => errors.push("process group remained live after SIGKILL".into()),
    }
    if errors.is_empty() {
        String::new()
    } else {
        format!("; cleanup errors: {}", errors.join("; "))
    }
}

#[derive(Debug)]
struct ProcIdentity {
    process_group: u32,
    starttime: u64,
    state: u8,
    zombie: bool,
}

impl ProcIdentity {
    fn is_stopped(&self) -> bool {
        matches!(self.state, b'T' | b't')
    }
}

#[cfg(target_os = "linux")]
fn read_proc_identity(pid: u32) -> Result<Option<ProcIdentity>> {
    let contents = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(contents) => contents,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ESRCH) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error).context("read renderer /proc identity"),
    };
    let close = contents
        .rfind(')')
        .ok_or_else(|| anyhow!("malformed /proc/{pid}/stat"))?;
    let fields: Vec<_> = contents[close + 1..].split_whitespace().collect();
    if fields.len() < 20 {
        bail!("malformed /proc/{pid}/stat");
    }
    let state = fields[0]
        .as_bytes()
        .first()
        .copied()
        .ok_or_else(|| anyhow!("missing process state in /proc/{pid}/stat"))?;
    Ok(Some(ProcIdentity {
        state,
        zombie: state == b'Z',
        process_group: fields[2].parse().context("parse process group")?,
        starttime: fields[19].parse().context("parse process starttime")?,
    }))
}

#[cfg(not(target_os = "linux"))]
fn read_proc_identity(_pid: u32) -> Result<Option<ProcIdentity>> {
    Ok(None)
}

#[cfg(target_os = "linux")]
fn process_group_live(process_group: u32) -> Result<bool> {
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(identity) = read_proc_identity(pid)? {
            if identity.process_group == process_group && !identity.zombie {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(not(target_os = "linux"))]
fn process_group_live(_process_group: u32) -> Result<bool> {
    Ok(false)
}

async fn terminate_orphan_process_group(pid: u32) -> Result<()> {
    if let Some(error) = signal_process_group(pid, libc::SIGTERM) {
        bail!("{error}");
    }
    if timeout(Duration::from_secs(2), async {
        while process_group_live(pid)? {
            sleep(Duration::from_millis(25)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .is_ok()
    {
        return Ok(());
    }
    if let Some(error) = signal_process_group(pid, libc::SIGKILL) {
        bail!("{error}");
    }
    timeout(Duration::from_secs(2), async {
        while process_group_live(pid)? {
            sleep(Duration::from_millis(25)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("renderer process group remained live after SIGKILL")??;
    Ok(())
}

#[cfg(unix)]
fn open_verified_renderer(path: &Path, expected_hash: &str) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .context("open official renderer without symlink following")?;
    if !file.metadata()?.is_file() {
        bail!("official renderer is not a regular file");
    }
    let mut reader = file.try_clone()?;
    let mut hasher = Sha256::new();
    io::copy(&mut reader, &mut hasher)?;
    if format!("{:x}", hasher.finalize()) != expected_hash {
        bail!("official renderer identity changed before spawn");
    }
    // try_clone shares the open-file description and therefore the offset.
    // Rewind before the child consumes this descriptor.
    let mut renderer = &file;
    renderer.rewind()?;
    // SAFETY: fcntl operates on the valid renderer descriptor retained until
    // after spawn. Clearing CLOEXEC makes /proc/self/fd/N available to Python.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    if flags == -1
        || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) } == -1
    {
        return Err(io::Error::last_os_error()).context("preserve renderer descriptor for child");
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::database::Database;
    use crate::engine::FakeEngine;
    use crate::studio::Studio;
    use crate::subtitles::FakeSubtitles;
    use tempfile::tempdir;

    fn studio(root: &Path) -> Studio {
        let task_root = root.join("tasks");
        let task = task_root.join("group/batch");
        let production = task.join(".pipeline/production/S01");
        fs::create_dir_all(&production).unwrap();
        let source_dir = root.join("sources/group/batch");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("source.mp4"), b"fake-video").unwrap();
        for (path, contents) in [
            (
                production.join("edl.json"),
                br#"{"video_segments":[{"type":"clip","source":"source.mp4","in":0,"out":1,"timeline_in":0,"timeline_out":1}],"b_roll_overlays":[]}"#
                    .as_slice(),
            ),
            (production.join("subs.zh-en.ass"), b"ze".as_slice()),
            (production.join("subs.ru-en.ass"), b"re".as_slice()),
            (
                task.join(".pipeline/input_manifest.tsv"),
                b"filename\tsize\tduration\tvideo_codec\twidth\theight\taudio_codec\taudio_rate\nsource.mp4\t10\t1.0\th264\t1920\t1080\taac\t48000\n"
                    .as_slice(),
            ),
            (
                root.join("renderer.py"),
                b"from pathlib import Path\nPath(__file__).with_name('renderer-ran').write_text('yes')\n"
                    .as_slice(),
            ),
        ] {
            fs::write(path, contents).unwrap();
        }
        let settings = Settings {
            data_dir: root.join("data"),
            model_dir: root.join("model"),
            cosyvoice_root: root.join("cosy"),
            setup_token_file: root.join("setup"),
            host: "127.0.0.1".into(),
            port: 7860,
            ssl_certfile: None,
            ssl_keyfile: None,
            mcp_token: None,
            mcp_token_file: root.join("mcp-token"),
            mcp_token_source: None,
            funclip_root: None,
            video_input_dir: root.join("videos"),
            reference_input_dir: root.join("references"),
            video_projects_dir: root.join("video-projects"),
            receipt_key_file: root.join("receipt.key"),
            subtitle_timeout_seconds: 30,
            xry_task_root: task_root,
            xry_source_root: root.join("sources"),
            xry_renderer: root.join("renderer.py"),
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
            Database::open(root.join("data/db.sqlite")).unwrap(),
            Arc::new(FakeEngine::new()),
            Arc::new(FakeSubtitles::default()),
        )
    }

    fn active_shutdown_receiver() -> watch::Receiver<bool> {
        let (_sender, receiver) = watch::channel(false);
        receiver
    }

    fn request() -> SubmitRenderRequest {
        SubmitRenderRequest {
            task_dir: "group/batch".into(),
            subject_id: "S01".into(),
            encoder_profile: default_encoder_profile(),
        }
    }

    async fn wait_for_terminal(studio: &Studio, id: &str) -> RenderJobRow {
        timeout(Duration::from_secs(8), async {
            loop {
                let row = studio.database.render_job_by_id(id).unwrap().unwrap();
                if matches!(row.status.as_str(), "succeeded" | "failed" | "canceled") {
                    return row;
                }
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("render job did not become terminal")
    }

    async fn wait_for_running(studio: &Studio, id: &str) {
        timeout(Duration::from_secs(5), async {
            loop {
                if studio
                    .database
                    .render_job_by_id(id)
                    .unwrap()
                    .unwrap()
                    .status
                    == "running"
                {
                    break;
                }
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("render job did not start");
    }

    fn prepare_real_media_fixture(root: &Path) -> PathBuf {
        let fixture = root.join("videos/fixture.mp4");
        fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        let output = std::process::Command::new("/usr/bin/ffmpeg")
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=duration=4:size=160x90:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1:sample_rate=48000",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=mono:d=1",
                "-filter_complex",
                "[1:a][2:a][1:a][2:a]concat=n=4:v=0:a=1[a]",
                "-map",
                "0:v",
                "-map",
                "[a]",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
                "-n",
            ])
            .arg(&fixture)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fixture
    }

    #[test]
    fn deduplicates_same_frozen_render_inputs() {
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        let request = || SubmitRenderRequest {
            task_dir: "group/batch".into(),
            subject_id: "S01".into(),
            encoder_profile: default_encoder_profile(),
        };
        assert_eq!(submit(&studio, request()).unwrap()["created"], true);
        assert_eq!(submit(&studio, request()).unwrap()["deduplicated"], true);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_edl_symlink_even_when_target_is_a_regular_file() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let edl = studio
            .settings
            .xry_task_root
            .join("group/batch/.pipeline/production/S01/edl.json");
        fs::remove_file(&edl).unwrap();
        symlink("/etc/hosts", &edl).unwrap();

        let error = submit(
            &studio,
            SubmitRenderRequest {
                task_dir: "group/batch".into(),
                subject_id: "S01".into(),
                encoder_profile: default_encoder_profile(),
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("symlink component"));
    }

    #[tokio::test]
    async fn queued_task_input_drift_does_not_change_private_snapshot() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(
            &studio,
            SubmitRenderRequest {
                task_dir: "group/batch".into(),
                subject_id: "S01".into(),
                encoder_profile: default_encoder_profile(),
            },
        )
        .unwrap();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        let edl = studio
            .settings
            .xry_task_root
            .join("group/batch/.pipeline/production/S01/edl.json");
        fs::write(edl, b"{\"drift\":true}").unwrap();

        process_claimed_job(&studio, &job, &mut active_shutdown_receiver()).await;

        let id = payload["job"]["id"].as_str().unwrap();
        assert_eq!(
            (
                studio
                    .database
                    .render_job_by_id(id)
                    .unwrap()
                    .unwrap()
                    .status,
                root.path().join("renderer-ran").exists(),
            ),
            ("succeeded".to_string(), true)
        );
    }

    #[tokio::test]
    async fn cancellation_after_claim_wins_before_renderer_start() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(
            &studio,
            SubmitRenderRequest {
                task_dir: "group/batch".into(),
                subject_id: "S01".into(),
                encoder_profile: default_encoder_profile(),
            },
        )
        .unwrap();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        cancel(&studio, &job.id).unwrap();

        process_claimed_job(&studio, &job, &mut active_shutdown_receiver()).await;

        let id = payload["job"]["id"].as_str().unwrap();
        assert_eq!(
            (
                studio
                    .database
                    .render_job_by_id(id)
                    .unwrap()
                    .unwrap()
                    .status,
                root.path().join("renderer-ran").exists(),
            ),
            ("canceled".to_string(), false)
        );
    }

    #[test]
    fn failed_job_allows_one_new_attempt_under_concurrent_resubmission() {
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        let request = || SubmitRenderRequest {
            task_dir: "group/batch".into(),
            subject_id: "S01".into(),
            encoder_profile: default_encoder_profile(),
        };
        submit(&studio, request()).unwrap();
        let failed = studio.database.claim_next_render_job().unwrap().unwrap();
        studio
            .database
            .finish_render_job(&failed.id, "failed", Some(1), Some("test failure"))
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(5));

        let created = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let studio = studio.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        submit(&studio, request()).unwrap()["created"]
                            .as_bool()
                            .unwrap()
                    })
                })
                .collect();
            barrier.wait();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .filter(|created| *created)
                .count()
        });

        assert_eq!(created, 1);
    }

    #[test]
    fn canceled_job_allows_one_new_attempt_under_concurrent_resubmission() {
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        let request = || SubmitRenderRequest {
            task_dir: "group/batch".into(),
            subject_id: "S01".into(),
            encoder_profile: default_encoder_profile(),
        };
        let first = submit(&studio, request()).unwrap();
        let first_id = first["job"]["id"].as_str().unwrap();
        cancel(&studio, first_id).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(5));

        let created = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let studio = studio.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        submit(&studio, request()).unwrap()["created"]
                            .as_bool()
                            .unwrap()
                    })
                })
                .collect();
            barrier.wait();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .filter(|created| *created)
                .count()
        });

        assert_eq!(created, 1);
    }

    #[test]
    fn rejects_paths_outside_task_root() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let error = submit(
            &studio,
            SubmitRenderRequest {
                task_dir: root.path().to_string_lossy().into_owned(),
                subject_id: "S01".into(),
                encoder_profile: default_encoder_profile(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("inside"));
    }

    #[tokio::test]
    async fn legacy_running_job_without_identity_fails_closed() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(
            &studio,
            SubmitRenderRequest {
                task_dir: "group/batch".into(),
                subject_id: "S01".into(),
                encoder_profile: default_encoder_profile(),
            },
        )
        .unwrap();
        let id = payload["job"]["id"].as_str().unwrap();
        assert!(studio.database.claim_next_render_job().unwrap().is_some());
        assert!(recover_running_jobs(&studio)
            .await
            .unwrap_err()
            .to_string()
            .contains("worker stopped"));
        let row = studio.database.render_job_by_id(id).unwrap().unwrap();
        assert_eq!(row.status, "running");
        assert!(row.recovery_blocked.is_some());
    }

    #[test]
    fn public_job_error_never_exposes_internal_paths() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap();
        studio.database.claim_next_render_job().unwrap().unwrap();
        let private = format!(
            "renderer failed reading {}/secret/input.mp4",
            root.path().display()
        );
        studio
            .database
            .finish_render_job(id, "failed", Some(1), Some(&private))
            .unwrap();
        let public = get(&studio, id).unwrap().unwrap();
        let serialized = serde_json::to_string(&public).unwrap();
        assert!(!serialized.contains(&private));
        assert!(!serialized.contains(&root.path().to_string_lossy().to_string()));
        assert_eq!(public["job"]["error"]["code"], "failed");
        assert_eq!(public["job"]["error"]["message"], "Render failed.");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn recovery_uses_launch_handshake_before_database_pid_persistence() {
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_string();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        let renderer =
            open_verified_renderer(&studio.settings.xry_renderer, &job.renderer_hash).unwrap();
        let fd = renderer.as_raw_fd();
        let handshake = launch_handshake_path(&studio, &id);
        let renderer_root = studio.settings.xry_renderer.parent().unwrap();
        let mut command = Command::new(&studio.settings.xry_python);
        command
            .args([
                "-c",
                EXECUTOR_WRAPPER,
                &fd.to_string(),
                &studio.settings.xry_renderer.to_string_lossy(),
                &renderer_root.to_string_lossy(),
                &handshake.to_string_lossy(),
                &id,
                &job.renderer_hash,
            ])
            .args(["--encoder-profile", "formal-auto"])
            .arg(root.path().join("unused-edl"))
            .arg(root.path().join("unused-snapshot"))
            .arg(root.path().join("unused-source"))
            .arg(root.path().join("unused-manifest"))
            .arg(root.path().join("must-not-run"));
        command.process_group(0);
        let child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        // Simulate the worker disappearing immediately after spawn. Recovery
        // races the child and must wait for its durable self-stop handshake.
        drop(child);
        recover_running_jobs(&studio).await.unwrap();
        let row = studio.database.render_job_by_id(&id).unwrap().unwrap();
        assert_eq!(row.status, "queued");
        assert!(row.pid.is_none());
        assert!(!process_group_live(pid).unwrap());
        assert!(!root.path().join("renderer-ran").exists());
        assert!(!handshake.exists());
        let worker = start_worker(studio.clone()).unwrap();
        let retried = wait_for_terminal(&studio, &id).await;
        assert_eq!(retried.status, "succeeded", "{:?}", retried.error);
        worker.shutdown();
        worker.wait().await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn launch_handshake_final_is_never_visible_partially_written() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_owned();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        let renderer =
            open_verified_renderer(&studio.settings.xry_renderer, &job.renderer_hash).unwrap();
        let fd = renderer.as_raw_fd();
        let handshake = launch_handshake_path(&studio, &id);
        let renderer_root = studio.settings.xry_renderer.parent().unwrap();
        let mut command = Command::new(&studio.settings.xry_python);
        command
            .env("VWA_TEST_HANDSHAKE_PAUSE_AFTER_BYTES", "7")
            .args([
                "-c",
                EXECUTOR_WRAPPER,
                &fd.to_string(),
                &studio.settings.xry_renderer.to_string_lossy(),
                &renderer_root.to_string_lossy(),
                &handshake.to_string_lossy(),
                &id,
                &job.renderer_hash,
            ])
            .args(["--encoder-profile", "formal-auto"])
            .arg(root.path().join("unused-edl"))
            .arg(root.path().join("unused-snapshot"))
            .arg(root.path().join("unused-source"))
            .arg(root.path().join("unused-manifest"))
            .arg(root.path().join("must-not-run"))
            .process_group(0);
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        let job_dir = studio.settings.render_jobs_dir().join(&id);
        let temporary = timeout(Duration::from_secs(5), async {
            loop {
                if let Some(path) = fs::read_dir(&job_dir).unwrap().find_map(|entry| {
                    let path = entry.unwrap().path();
                    let is_temporary = path.file_name().is_some_and(is_wrapper_handshake_temp);
                    is_temporary.then_some(path)
                }) {
                    break path;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        assert!(!handshake.exists());
        let partial = fs::read(&temporary).unwrap();
        assert_eq!(partial.len(), 7);
        assert!(serde_json::from_slice::<LaunchHandshake>(&partial).is_err());
        let mut waiting = Box::pin(wait_for_launch_handshake(&handshake, pid, &job));
        assert!(
            timeout(Duration::from_millis(100), waiting.as_mut())
                .await
                .is_err(),
            "parent accepted a handshake before atomic publication"
        );

        // SAFETY: this test created the child process group and observed its
        // wrapper-owned private temporary handshake.
        assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGCONT) }, 0);
        let value = timeout(Duration::from_secs(5), waiting.as_mut())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(value.pid, pid);
        assert!(handshake.is_file());
        assert!(!temporary.exists());
        assert!(serde_json::from_slice::<LaunchHandshake>(&fs::read(&handshake).unwrap()).is_ok());

        terminate_orphan_process_group(pid).await.unwrap();
        let _ = child.wait().await;
        let unrelated = job_dir.join(".launch-handshake.not-wrapper.tmp");
        fs::write(&unrelated, b"preserve").unwrap();
        remove_launch_handshake(&studio, &id).unwrap();
        assert!(!handshake.exists());
        assert!(unrelated.exists());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn final_handshake_waits_for_self_stop_before_persist_and_resume() {
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_owned();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        set_test_launch_hook(
            &id,
            TestLaunchHook {
                delay_before_stop_ms: 500,
                exit_before_stop: false,
            },
        );

        let worker_studio = studio.clone();
        let worker_job = job.clone();
        let processing = tokio::spawn(async move {
            process_claimed_job(&worker_studio, &worker_job, &mut active_shutdown_receiver()).await
        });
        let handshake_path = launch_handshake_path(&studio, &id);
        let handshake = timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(value) = read_launch_handshake(&handshake_path) {
                    break value;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let identity = read_proc_identity(handshake.pid).unwrap().unwrap();
        assert!(!identity.is_stopped());
        assert!(
            studio
                .database
                .render_job_by_id(&id)
                .unwrap()
                .unwrap()
                .pid
                .is_none(),
            "parent persisted PID before the wrapper stopped"
        );
        assert!(!root.path().join("renderer-ran").exists());
        sleep(Duration::from_millis(100)).await;
        assert!(studio
            .database
            .render_job_by_id(&id)
            .unwrap()
            .unwrap()
            .pid
            .is_none());
        assert!(!root.path().join("renderer-ran").exists());

        assert!(timeout(Duration::from_secs(5), processing)
            .await
            .unwrap()
            .unwrap());
        let completed = studio.database.render_job_by_id(&id).unwrap().unwrap();
        assert_eq!(completed.status, "succeeded", "{:?}", completed.error);
        assert!(root.path().join("renderer-ran").is_file());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn child_exit_after_final_handshake_but_before_stop_fails_without_persisting_pid() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_owned();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        set_test_launch_hook(
            &id,
            TestLaunchHook {
                delay_before_stop_ms: 50,
                exit_before_stop: true,
            },
        );

        assert!(
            process_claimed_job(&studio, &job, &mut active_shutdown_receiver()).await,
            "confirmed child exit should not stop the queue"
        );
        let failed = studio.database.render_job_by_id(&id).unwrap().unwrap();
        assert_eq!(failed.status, "failed");
        assert!(failed.pid.is_none());
        assert!(failed.pid_starttime.is_none());
        assert!(!root.path().join("renderer-ran").exists());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn preexisting_final_handshake_fails_before_renderer_media_work() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_owned();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        let handshake = launch_handshake_path(&studio, &id);
        fs::write(&handshake, b"preexisting").unwrap();
        let mut shutdown = active_shutdown_receiver();

        let error = run_job(&studio, &job, &mut shutdown).await.unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert_eq!(fs::read(&handshake).unwrap(), b"preexisting");
        assert!(!root.path().join("renderer-ran").exists());
        assert!(studio
            .database
            .render_job_by_id(&id)
            .unwrap()
            .unwrap()
            .pid
            .is_none());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn launch_handshake_recovery_survives_one_hundred_loaded_rounds() {
        let permits = Arc::new(tokio::sync::Semaphore::new(8));
        let mut rounds = tokio::task::JoinSet::new();
        for _ in 0..100 {
            let permits = permits.clone();
            rounds.spawn(async move {
                let _permit = permits.acquire_owned().await.unwrap();
                let root = tempdir().unwrap();
                let studio = studio(root.path());
                let payload = submit(&studio, request()).unwrap();
                let id = payload["job"]["id"].as_str().unwrap().to_owned();
                let job = studio.database.claim_next_render_job().unwrap().unwrap();
                let renderer =
                    open_verified_renderer(&studio.settings.xry_renderer, &job.renderer_hash)
                        .unwrap();
                let fd = renderer.as_raw_fd();
                let handshake = launch_handshake_path(&studio, &id);
                let renderer_root = studio.settings.xry_renderer.parent().unwrap();
                let mut command = Command::new(&studio.settings.xry_python);
                command
                    .args([
                        "-c",
                        EXECUTOR_WRAPPER,
                        &fd.to_string(),
                        &studio.settings.xry_renderer.to_string_lossy(),
                        &renderer_root.to_string_lossy(),
                        &handshake.to_string_lossy(),
                        &id,
                        &job.renderer_hash,
                    ])
                    .args(["--encoder-profile", "formal-auto"])
                    .arg(root.path().join("unused-edl"))
                    .arg(root.path().join("unused-snapshot"))
                    .arg(root.path().join("unused-source"))
                    .arg(root.path().join("unused-manifest"))
                    .arg(root.path().join("must-not-run"))
                    .process_group(0);
                let mut child = command.spawn().unwrap();
                assert!(child.id().is_some());
                recover_running_jobs(&studio).await.unwrap();
                timeout(Duration::from_secs(5), child.wait())
                    .await
                    .unwrap()
                    .unwrap();
                let row = studio.database.render_job_by_id(&id).unwrap().unwrap();
                assert_eq!(row.status, "queued");
                assert!(!handshake.exists());
                assert!(!fs::read_dir(studio.settings.render_jobs_dir().join(&id))
                    .unwrap()
                    .any(|entry| is_wrapper_handshake_temp(&entry.unwrap().file_name())));
            });
        }
        while let Some(round) = rounds.join_next().await {
            round.unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn stopped_readiness_survives_one_hundred_directory_fsync_rounds() {
        let permits = Arc::new(tokio::sync::Semaphore::new(8));
        let mut rounds = tokio::task::JoinSet::new();
        for _ in 0..100 {
            let permits = permits.clone();
            rounds.spawn(async move {
                let _permit = permits.acquire_owned().await.unwrap();
                let root = tempdir().unwrap();
                let studio = studio(root.path());
                let payload = submit(&studio, request()).unwrap();
                let id = payload["job"]["id"].as_str().unwrap().to_owned();
                let job = studio.database.claim_next_render_job().unwrap().unwrap();
                let renderer =
                    open_verified_renderer(&studio.settings.xry_renderer, &job.renderer_hash)
                        .unwrap();
                let fd = renderer.as_raw_fd();
                let handshake = launch_handshake_path(&studio, &id);
                let renderer_root = studio.settings.xry_renderer.parent().unwrap();
                let mut command = Command::new(&studio.settings.xry_python);
                command
                    .env("VWA_TEST_HANDSHAKE_DELAY_BEFORE_STOP_MS", "2")
                    .args([
                        "-c",
                        EXECUTOR_WRAPPER,
                        &fd.to_string(),
                        &studio.settings.xry_renderer.to_string_lossy(),
                        &renderer_root.to_string_lossy(),
                        &handshake.to_string_lossy(),
                        &id,
                        &job.renderer_hash,
                    ])
                    .args(["--encoder-profile", "formal-auto"])
                    .arg(root.path().join("unused-edl"))
                    .arg(root.path().join("unused-snapshot"))
                    .arg(root.path().join("unused-source"))
                    .arg(root.path().join("unused-manifest"))
                    .arg(root.path().join("must-not-run"))
                    .process_group(0);
                let mut child = command.spawn().unwrap();
                let pid = child.id().unwrap();
                let value = wait_for_launch_handshake(&handshake, pid, &job)
                    .await
                    .unwrap();
                let stopped = read_proc_identity(pid).unwrap().unwrap();
                assert_eq!(stopped.starttime, value.starttime);
                assert!(stopped.is_stopped());
                assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGCONT) }, 0);
                wait_for_renderer_resumed(pid, value.starttime)
                    .await
                    .unwrap();
                timeout(Duration::from_secs(5), child.wait())
                    .await
                    .unwrap()
                    .unwrap();
            });
        }
        while let Some(round) = rounds.join_next().await {
            round.unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn corrupt_finalized_handshake_without_persisted_identity_fails_closed() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_owned();
        studio.database.claim_next_render_job().unwrap().unwrap();
        let handshake = launch_handshake_path(&studio, &id);
        fs::write(&handshake, b"{corrupt").unwrap();

        let error = recover_running_jobs(&studio).await.unwrap_err();
        assert!(error.to_string().contains("ambiguous"));
        let row = studio.database.render_job_by_id(&id).unwrap().unwrap();
        assert_eq!(row.status, "running");
        assert!(row.recovery_blocked.is_some());
        assert_eq!(fs::read(&handshake).unwrap(), b"{corrupt");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn persisted_process_identity_recovers_despite_corrupt_handshake() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_owned();
        let claimed = studio.database.claim_next_render_job().unwrap().unwrap();
        let renderer =
            open_verified_renderer(&studio.settings.xry_renderer, &claimed.renderer_hash).unwrap();
        let fd = renderer.as_raw_fd();
        let handshake = launch_handshake_path(&studio, &id);
        let renderer_root = studio.settings.xry_renderer.parent().unwrap();
        let mut command = Command::new(&studio.settings.xry_python);
        command
            .args([
                "-c",
                EXECUTOR_WRAPPER,
                &fd.to_string(),
                &studio.settings.xry_renderer.to_string_lossy(),
                &renderer_root.to_string_lossy(),
                &handshake.to_string_lossy(),
                &id,
                &claimed.renderer_hash,
            ])
            .args(["--encoder-profile", "formal-auto"])
            .arg(root.path().join("unused-edl"))
            .arg(root.path().join("unused-snapshot"))
            .arg(root.path().join("unused-source"))
            .arg(root.path().join("unused-manifest"))
            .arg(root.path().join("must-not-run"))
            .process_group(0);
        let mut orphan = command.spawn().unwrap();
        let pid = orphan.id().unwrap();
        let handshake_value = wait_for_launch_handshake(&handshake, pid, &claimed)
            .await
            .unwrap();
        let starttime = handshake_value.starttime;
        assert!(studio
            .database
            .set_render_job_process(&claimed.id, pid, starttime)
            .unwrap());
        studio.database.begin_render_recovery(&claimed.id).unwrap();
        assert_eq!(
            studio
                .database
                .render_job_by_id(&claimed.id)
                .unwrap()
                .unwrap()
                .recovery_intent
                .as_deref(),
            Some("terminate_then_requeue")
        );
        fs::write(&handshake, b"{corrupt").unwrap();

        recover_running_jobs(&studio).await.unwrap();

        assert_eq!(
            studio
                .database
                .render_job_by_id(&id)
                .unwrap()
                .unwrap()
                .status,
            "queued"
        );
        assert!(!process_group_live(pid).unwrap());
        assert!(!handshake.exists());
        let _ = orphan.wait().await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn persisted_process_identity_recovers_with_missing_or_symlink_handshake() {
        use std::os::unix::fs::symlink;

        for symlink_mode in [false, true] {
            let root = tempdir().unwrap();
            let studio = studio(root.path());
            let payload = submit(&studio, request()).unwrap();
            let id = payload["job"]["id"].as_str().unwrap().to_owned();
            let claimed = studio.database.claim_next_render_job().unwrap().unwrap();
            let renderer =
                open_verified_renderer(&studio.settings.xry_renderer, &claimed.renderer_hash)
                    .unwrap();
            let fd = renderer.as_raw_fd();
            let handshake = launch_handshake_path(&studio, &id);
            let renderer_root = studio.settings.xry_renderer.parent().unwrap();
            let mut command = Command::new(&studio.settings.xry_python);
            command
                .args([
                    "-c",
                    EXECUTOR_WRAPPER,
                    &fd.to_string(),
                    &studio.settings.xry_renderer.to_string_lossy(),
                    &renderer_root.to_string_lossy(),
                    &handshake.to_string_lossy(),
                    &id,
                    &claimed.renderer_hash,
                ])
                .args(["--encoder-profile", "formal-auto"])
                .arg(root.path().join("unused-edl"))
                .arg(root.path().join("unused-snapshot"))
                .arg(root.path().join("unused-source"))
                .arg(root.path().join("unused-manifest"))
                .arg(root.path().join("must-not-run"))
                .process_group(0);
            let mut orphan = command.spawn().unwrap();
            let pid = orphan.id().unwrap();
            let starttime = wait_for_launch_handshake(&handshake, pid, &claimed)
                .await
                .unwrap()
                .starttime;
            studio
                .database
                .set_render_job_process(&claimed.id, pid, starttime)
                .unwrap();
            if !symlink_mode {
                studio.database.begin_render_recovery(&claimed.id).unwrap();
                // SAFETY: this test created a process group whose ID is the child PID.
                unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
                let _ = orphan.wait().await;
            }
            fs::remove_file(&handshake).unwrap();
            let outside = root.path().join("handshake-target");
            if symlink_mode {
                fs::write(&outside, b"must survive").unwrap();
                symlink(&outside, &handshake).unwrap();
            }

            recover_running_jobs(&studio).await.unwrap();
            assert_eq!(
                studio
                    .database
                    .render_job_by_id(&id)
                    .unwrap()
                    .unwrap()
                    .status,
                "queued"
            );
            assert!(!process_group_live(pid).unwrap());
            assert!(!handshake.exists());
            if symlink_mode {
                assert_eq!(fs::read(&outside).unwrap(), b"must survive");
            }
            if symlink_mode {
                let _ = orphan.wait().await;
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn ambiguous_persisted_process_keeps_job_running_and_blocks_next_claim() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let first = submit(&studio, request()).unwrap();
        fs::write(
            studio
                .settings
                .xry_task_root
                .join("group/batch/.pipeline/production/S01/subs.zh-en.ass"),
            b"second queued identity",
        )
        .unwrap();
        submit(&studio, request()).unwrap();
        let claimed = studio.database.claim_next_render_job().unwrap().unwrap();
        assert_eq!(claimed.id, first["job"]["id"]);

        let mut unrelated = Command::new("/bin/sleep");
        unrelated.arg("30").process_group(0);
        let mut unrelated = unrelated.spawn().unwrap();
        let pid = unrelated.id().unwrap();
        let starttime = read_proc_identity(pid).unwrap().unwrap().starttime;
        studio
            .database
            .set_render_job_process(&claimed.id, pid, starttime)
            .unwrap();

        assert!(recover_running_jobs(&studio).await.is_err());
        let blocked = studio
            .database
            .render_job_by_id(&claimed.id)
            .unwrap()
            .unwrap();
        assert_eq!(blocked.status, "running");
        assert!(blocked.recovery_blocked.is_some());
        assert!(process_group_live(pid).unwrap());
        assert!(studio.database.claim_next_render_job().is_err());

        // SAFETY: this test created a process group whose ID is the child PID.
        unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
        let _ = unrelated.wait().await;
    }

    #[test]
    fn enqueue_sequence_is_unique_and_claimed_in_fifo_order_with_same_second() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let mut expected = Vec::new();
        for byte in b'a'..=b'h' {
            fs::write(
                studio
                    .settings
                    .xry_task_root
                    .join("group/batch/.pipeline/production/S01/subs.zh-en.ass"),
                [byte],
            )
            .unwrap();
            let payload = submit(&studio, request()).unwrap();
            expected.push((
                payload["job"]["id"].as_str().unwrap().to_owned(),
                payload["job"]["enqueue_seq"].as_i64().unwrap(),
            ));
        }
        assert!(expected.windows(2).all(|pair| pair[0].1 < pair[1].1));
        for (id, _) in expected {
            let claimed = studio.database.claim_next_render_job().unwrap().unwrap();
            assert_eq!(claimed.id, id);
            studio
                .database
                .finish_render_job(&claimed.id, "failed", Some(1), Some("advance FIFO test"))
                .unwrap();
        }
    }

    #[test]
    fn concurrent_enqueues_receive_unique_sequences_and_claim_by_sequence() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        let path = studio.database.path().to_path_buf();
        let barrier = Arc::new(std::sync::Barrier::new(17));
        let mut inserted = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|index| {
                    let path = path.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        let database = Database::open(path).unwrap();
                        let id = format!("concurrent-{index}");
                        let key = format!("key-{index}");
                        barrier.wait();
                        database
                            .insert_or_get_render_job(NewRenderJob {
                                id: &id,
                                render_key: &key,
                                task_dir: "task",
                                subject_id: "S01",
                                encoder_profile: "formal-cpu",
                                log_path: "render.log",
                                snapshot_dir: "snapshot",
                                snapshot_hash: "snapshot-hash",
                                renderer_hash: "renderer-hash",
                            })
                            .unwrap()
                            .0
                    })
                })
                .collect();
            barrier.wait();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        inserted.sort_by_key(|row| row.enqueue_seq);
        assert!(inserted
            .windows(2)
            .all(|pair| pair[0].enqueue_seq < pair[1].enqueue_seq));
        for expected in inserted {
            let claimed = studio.database.claim_next_render_job().unwrap().unwrap();
            assert_eq!(claimed.id, expected.id);
            studio
                .database
                .finish_render_job(&claimed.id, "failed", Some(1), Some("advance FIFO test"))
                .unwrap();
        }
    }

    #[test]
    fn two_database_workers_cannot_create_two_running_jobs() {
        let root = tempdir().unwrap();
        let studio = studio(root.path());
        submit(&studio, request()).unwrap();
        fs::write(
            studio
                .settings
                .xry_task_root
                .join("group/batch/.pipeline/production/S01/subs.zh-en.ass"),
            b"changed frozen input",
        )
        .unwrap();
        submit(&studio, request()).unwrap();

        let first = Database::open(studio.database.path()).unwrap();
        let second = Database::open(studio.database.path()).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        std::thread::scope(|scope| {
            for database in [&first, &second] {
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    let _ = database.claim_next_render_job();
                });
            }
            barrier.wait();
        });

        assert_eq!(studio.database.render_job_counts().unwrap().1, 1);
    }

    #[tokio::test]
    async fn exclusive_worker_lease_rejects_second_worker() {
        let root = tempdir().unwrap();
        let first_studio = Arc::new(studio(root.path()));
        let first = start_worker(first_studio).unwrap();
        let second_studio = Arc::new(studio(root.path()));
        let error = start_worker(second_studio).unwrap_err();
        assert!(error.to_string().contains("exclusive lease"));
        first.shutdown();
        first.wait().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn running_cancel_terminates_process_group_and_descendant() {
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        fs::write(
            &studio.settings.xry_renderer,
            b"import pathlib,subprocess,time\np=subprocess.Popen(['sleep','60'])\npathlib.Path(__file__).with_name('descendant.pid').write_text(str(p.pid))\nwhile True: time.sleep(1)\n",
        )
        .unwrap();
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_string();
        let worker = start_worker(studio.clone()).unwrap();
        wait_for_running(&studio, &id).await;
        let pid_file = root.path().join("descendant.pid");
        timeout(Duration::from_secs(5), async {
            while !pid_file.is_file() {
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        cancel(&studio, &id).unwrap();
        assert_eq!(wait_for_terminal(&studio, &id).await.status, "canceled");
        let pid: i32 = fs::read_to_string(pid_file).unwrap().parse().unwrap();
        timeout(Duration::from_secs(3), async {
            loop {
                // SAFETY: signal 0 only tests whether the recorded child exists.
                if unsafe { libc::kill(pid, 0) } == -1
                    && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    break;
                }
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("renderer descendant survived cancellation");
        worker.shutdown();
        worker.wait().await.unwrap();
    }

    #[tokio::test]
    async fn timeout_terminates_job_and_releases_slot() {
        let root = tempdir().unwrap();
        let mut value = studio(root.path());
        value.settings.render_timeout_seconds = 1;
        fs::write(
            &value.settings.xry_renderer,
            b"import time\nwhile True: time.sleep(1)\n",
        )
        .unwrap();
        let studio = Arc::new(value);
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_string();
        let worker = start_worker(studio.clone()).unwrap();
        let row = wait_for_terminal(&studio, &id).await;
        assert_eq!(row.status, "failed");
        assert!(row.error.unwrap().contains("timeout"));
        assert_eq!(studio.database.render_job_counts().unwrap().1, 0);
        worker.shutdown();
        worker.wait().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_waits_for_running_job_to_become_terminal() {
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        fs::write(
            &studio.settings.xry_renderer,
            b"import time\nwhile True: time.sleep(1)\n",
        )
        .unwrap();
        let payload = submit(&studio, request()).unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_string();
        let worker = start_worker(studio.clone()).unwrap();
        wait_for_running(&studio, &id).await;
        worker.shutdown();
        worker.wait().await.unwrap();
        let row = studio.database.render_job_by_id(&id).unwrap().unwrap();
        assert_eq!(row.status, "failed");
        assert!(row.error.unwrap().contains("service shutdown"));
        assert_eq!(studio.database.render_job_counts().unwrap().1, 0);
    }

    #[tokio::test]
    async fn worker_dispatches_video_project_bundle_to_builtin_renderer() {
        if !Path::new("/usr/bin/ffmpeg").is_file() {
            return;
        }
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        fs::write(
            &studio.settings.video_project_renderer,
            include_bytes!("../scripts/video_project_render.py"),
        )
        .unwrap();
        let project = studio.settings.video_projects_dir.join("worker-render");
        fs::create_dir_all(project.join("assets")).unwrap();
        fs::create_dir(project.join("exports")).unwrap();
        let status = std::process::Command::new("/usr/bin/ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=10",
                "-t",
                "0.6",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-an",
                "-y",
            ])
            .arg(project.join("assets/source.mp4"))
            .status()
            .unwrap();
        assert!(status.success());
        let document = crate::vpe::parse(
            r#"project "Worker Render" {
  canvas 160x90 @ 10fps
  source main = "assets/source.mp4"
  timeline {
    track main {
      clip main source 00:00:00.000..00:00:00.500 at 00:00:00.000
    }
  }
}"#,
        )
        .unwrap();
        let payload = submit_video_project(
            &studio,
            &project,
            "worker-render",
            3,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &document,
        )
        .unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_string();
        // Execution must consume only the immutable snapshot, never this live
        // project asset after export.
        fs::write(project.join("assets/source.mp4"), b"replaced-after-export").unwrap();
        let worker = start_worker(studio.clone()).unwrap();
        let row = wait_for_terminal(&studio, &id).await;
        assert_eq!(row.status, "succeeded", "{:?}", row.error);
        assert!(project
            .join("exports")
            .join(&id)
            .join("master.mp4")
            .is_file());
        assert_eq!(row.kind, "video_project");
        assert_eq!(row.project_revision, Some(3));
        let replay_output = studio
            .settings
            .render_jobs_dir()
            .join(&id)
            .join("replay-output");
        assert!(!replay_output.exists());
        assert!(!studio
            .settings
            .render_jobs_dir()
            .join(&id)
            .join("snapshot/assets")
            .exists());
        assert!(row.attestation_json.is_some());
        worker.shutdown();
        worker.wait().await.unwrap();
    }

    #[tokio::test]
    async fn worker_executes_real_sampling_silence_and_cover_jobs() {
        if !Path::new("/usr/bin/ffmpeg").is_file()
            || !Path::new("/usr/bin/ffprobe").is_file()
            || !Path::new("/usr/share/fonts/TTF/DejaVuSans.ttf").is_file()
        {
            return;
        }
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        fs::create_dir_all(root.path().join("scripts")).unwrap();
        fs::write(
            root.path().join("scripts/video_media_job.py"),
            include_bytes!("../scripts/video_media_job.py"),
        )
        .unwrap();
        let source = prepare_real_media_fixture(root.path());
        fs::create_dir_all(root.path().join("video-projects/cover-project/exports")).unwrap();
        let sample = submit_media(
            &studio,
            "analysis_frames",
            "fixture.mp4",
            &source,
            &json!({
                "kind": "analysis_frames",
                "video_path": "fixture.mp4",
                "max_frames": 4,
                "resolution": [360, 640],
                "add_timestamp_overlay": true,
                "asr_segments": [
                    {"start_seconds": 0.0, "end_seconds": 1.0, "text": "opening"}
                ]
            }),
            None,
        )
        .unwrap();
        let trims = submit_media(
            &studio,
            "safe_trims",
            "fixture.mp4",
            &source,
            &json!({
                "kind": "safe_trims",
                "video_path": "fixture.mp4",
                "requested_start": 1.2,
                "requested_end": 3.2,
                "search_radius": 0.4,
                "words": []
            }),
            None,
        )
        .unwrap();
        let cover_request = json!({
            "kind": "cover",
            "stem": "v001-en-9x16",
            "project_id": "cover-project",
            "project_revision": 1,
            "document_sha256": "document-sha",
            "variant_key": "v001-en-9x16",
            "variant": {
                "language": "EN",
                "aspect": "9:16",
                "subtitles": "captions.ass"
            },
            "spec": {
                "source_video": "fixture.mp4",
                "frame_timestamp": 0.5,
                "layout_profile": "smoke-glass",
                "title": "Queued cover",
                "subtitle": "Real media"
            }
        });
        let cover = submit_media(
            &studio,
            "cover",
            "cover-project:1:document-sha:v001-en-9x16",
            &source,
            &cover_request,
            Some("cover-project"),
        )
        .unwrap();
        let cover_row = studio
            .database
            .render_job_by_id(cover["job"]["id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        prepare_execution(&studio, &cover_row).unwrap();
        let worker = start_worker(studio.clone()).unwrap();
        for payload in [&sample, &trims, &cover] {
            let id = payload["job"]["id"].as_str().unwrap();
            let row = wait_for_terminal(&studio, id).await;
            assert_eq!(
                row.status,
                "succeeded",
                "{}: {}",
                row.kind,
                row.error.unwrap_or_default()
            );
            let private = studio.settings.render_jobs_dir().join(id);
            assert!(!private.join("snapshot/input.media").exists());
            assert!(!private.join("launch-handshake.json").exists());
            assert!(private.join("snapshot/request.json").is_file());
            assert!(private.join("render.log").is_file());
        }
        let sample_result = get(&studio, sample["job"]["id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        let serialized = serde_json::to_string(&sample_result).unwrap();
        assert!(!serialized.contains(&root.path().to_string_lossy().to_string()));
        for private in [
            "log_path",
            "snapshot_dir",
            "output_dir",
            "render_plan",
            "renderer_hash",
            "render_key",
        ] {
            assert!(sample_result["job"].get(private).is_none());
        }
        let duration = sample_result["result"]["duration_seconds"]
            .as_f64()
            .unwrap();
        let frames = sample_result["result"]["frames"].as_array().unwrap();
        assert_eq!(frames.len(), 4);
        assert!(frames.iter().all(|frame| {
            frame["timestamp_seconds"].as_f64().unwrap() < duration
                && frame["sha256"]
                    .as_str()
                    .is_some_and(|hash| hash.len() == 64)
        }));
        assert_eq!(frames[0]["asr_text"], "opening");
        let trim_result = get(&studio, trims["job"]["id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            trim_result["result"]["capability"]["segment_level_asr_backend"],
            "funclip"
        );
        assert!(root
            .path()
            .join("video-projects/cover-project/exports/v001-en-9x16-cover-original.png")
            .is_file());
        assert!(root
            .path()
            .join("video-projects/cover-project/exports/v001-en-9x16.jpg")
            .is_file());
        let trusted_cover: quality::TrustedCoverAttestation = serde_json::from_str(
            studio
                .database
                .render_job_by_id(cover["job"]["id"].as_str().unwrap())
                .unwrap()
                .unwrap()
                .attestation_json
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(trusted_cover.project_id, "cover-project");
        assert_eq!(trusted_cover.revision, 1);
        assert_eq!(trusted_cover.variant_key, "v001-en-9x16");
        let published_hash = hash_file(
            &root
                .path()
                .join("video-projects/cover-project/exports/v001-en-9x16.jpg"),
        )
        .unwrap();
        let duplicate = submit_media(
            &studio,
            "cover",
            "cover-project:1:document-sha:v001-en-9x16",
            &source,
            &cover_request,
            Some("cover-project"),
        )
        .unwrap();
        assert_eq!(duplicate["created"], false);
        assert_eq!(duplicate["deduplicated"], true);
        assert_eq!(duplicate["job"]["id"], cover["job"]["id"]);
        assert_eq!(duplicate["job"]["status"], "succeeded");
        let overwrite = submit_media(
            &studio,
            "cover",
            "cover-project:1:document-sha:v001-en-9x16:changed",
            &source,
            &json!({
                "kind": "cover",
                "stem": "v001-en-9x16",
                "project_id": "cover-project",
                "project_revision": 1,
                "document_sha256": "document-sha",
                "variant_key": "v001-en-9x16",
                "variant": {
                    "language": "EN",
                    "aspect": "9:16",
                    "subtitles": "captions.ass"
                },
                "spec": {
                    "source_video": "fixture.mp4",
                    "frame_timestamp": 0.75,
                    "layout_profile": "banner-card",
                    "title": "Must not overwrite",
                    "subtitle": ""
                }
            }),
            Some("cover-project"),
        )
        .unwrap();
        let overwrite_id = overwrite["job"]["id"].as_str().unwrap();
        let overwrite_row = timeout(Duration::from_secs(10), async {
            loop {
                let row = studio
                    .database
                    .render_job_by_id(overwrite_id)
                    .unwrap()
                    .unwrap();
                if row.recovery_blocked.is_some() {
                    break row;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(overwrite_row.status, "running");
        assert!(overwrite_row.publication_intent.is_some());
        assert!(overwrite_row
            .recovery_blocked
            .as_deref()
            .is_some_and(|reason| reason.contains("publication")));
        assert!(studio.database.claim_next_render_job().unwrap().is_none());
        let overwrite_private = studio.settings.render_jobs_dir().join(overwrite_id);
        assert!(overwrite_private.join("snapshot/input.media").exists());
        assert!(overwrite_private.join("attempt-1").is_dir());
        assert_eq!(
            hash_file(
                &root
                    .path()
                    .join("video-projects/cover-project/exports/v001-en-9x16.jpg")
            )
            .unwrap(),
            published_hash
        );
        worker.shutdown();
        worker.wait().await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stale_private_media_attempt_is_removed_then_retry_succeeds() {
        if !Path::new("/usr/bin/ffmpeg").is_file()
            || !Path::new("/usr/bin/ffprobe").is_file()
            || !Path::new("/usr/share/fonts/TTF/DejaVuSans.ttf").is_file()
        {
            return;
        }
        let root = tempdir().unwrap();
        let studio = Arc::new(studio(root.path()));
        fs::create_dir_all(root.path().join("scripts")).unwrap();
        fs::write(
            root.path().join("scripts/video_media_job.py"),
            include_bytes!("../scripts/video_media_job.py"),
        )
        .unwrap();
        let source = prepare_real_media_fixture(root.path());
        let payload = submit_media(
            &studio,
            "analysis_frames",
            "fixture.mp4",
            &source,
            &json!({
                "kind": "analysis_frames",
                "video_path": "fixture.mp4",
                "max_frames": 2,
                "resolution": [360, 640],
                "add_timestamp_overlay": true,
                "asr_segments": []
            }),
            None,
        )
        .unwrap();
        let id = payload["job"]["id"].as_str().unwrap().to_string();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        let stale = studio
            .settings
            .render_jobs_dir()
            .join(&id)
            .join("attempt-1/output");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("partial.bin"), b"partial").unwrap();

        let renderer = studio
            .settings
            .project_root
            .join("scripts/video_media_job.py");
        let opened = open_verified_renderer(&renderer, &job.renderer_hash).unwrap();
        let fd = opened.as_raw_fd();
        let handshake = launch_handshake_path(&studio, &id);
        let mut command = Command::new(&studio.settings.video_project_python);
        command
            .args([
                "-c",
                EXECUTOR_WRAPPER,
                &fd.to_string(),
                &renderer.to_string_lossy(),
                &renderer.parent().unwrap().to_string_lossy(),
                &handshake.to_string_lossy(),
                &id,
                &job.renderer_hash,
            ])
            .arg("unused-request")
            .arg("unused-input")
            .arg("unused-output")
            .process_group(0);
        let mut orphan = command.spawn().unwrap();
        let pid = orphan.id().unwrap();
        let handshake_value = wait_for_launch_handshake(&handshake, pid, &job)
            .await
            .unwrap();
        assert!(studio
            .database
            .set_render_job_process(&id, pid, handshake_value.starttime)
            .unwrap());

        recover_running_jobs(&studio).await.unwrap();
        assert!(!studio
            .settings
            .render_jobs_dir()
            .join(&id)
            .join("attempt-1")
            .exists());
        assert_eq!(
            studio
                .database
                .render_job_by_id(&id)
                .unwrap()
                .unwrap()
                .status,
            "queued"
        );
        let _ = orphan.wait().await;

        let worker = start_worker(studio.clone()).unwrap();
        let retried = wait_for_terminal(&studio, &id).await;
        assert_eq!(retried.status, "succeeded", "{:?}", retried.error);
        worker.shutdown();
        worker.wait().await.unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn terminal_retention_refuses_symlink_without_touching_target() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let studio = studio(root.path());
        fs::create_dir_all(root.path().join("scripts")).unwrap();
        fs::write(
            root.path().join("scripts/video_media_job.py"),
            include_bytes!("../scripts/video_media_job.py"),
        )
        .unwrap();
        let source = root.path().join("source.mp4");
        fs::write(&source, b"source").unwrap();
        let payload = submit_media(
            &studio,
            "analysis_frames",
            "source.mp4",
            &source,
            &json!({"kind":"analysis_frames","video_path":"source.mp4"}),
            None,
        )
        .unwrap();
        let id = payload["job"]["id"].as_str().unwrap();
        let claimed = studio.database.claim_next_render_job().unwrap().unwrap();
        studio
            .database
            .finish_render_job(id, "failed", None, Some("test terminal"))
            .unwrap();
        let input = studio
            .settings
            .render_jobs_dir()
            .join(id)
            .join("snapshot/input.media");
        fs::remove_file(&input).unwrap();
        let outside = root.path().join("must-survive");
        fs::write(&outside, b"untouched").unwrap();
        symlink(&outside, &input).unwrap();

        assert!(retain_terminal_job_artifacts(&studio, &claimed).is_err());
        assert!(
            studio
                .database
                .render_job_by_id(id)
                .unwrap()
                .unwrap()
                .cleanup_pending
        );
        assert_eq!(fs::read(&outside).unwrap(), b"untouched");
        assert!(input.is_symlink());
        fs::remove_file(&input).unwrap();
        sweep_terminal_cleanup(&studio).unwrap();
        assert!(
            !studio
                .database
                .render_job_by_id(id)
                .unwrap()
                .unwrap()
                .cleanup_pending
        );
    }

    #[cfg(target_os = "linux")]
    fn publication_fixture(
        root: &Path,
        nested_project_output: bool,
    ) -> (Studio, RenderJobRow, PublicationIntent) {
        let studio = studio(root);
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::write(
            root.join("scripts/video_media_job.py"),
            include_bytes!("../scripts/video_media_job.py"),
        )
        .unwrap();
        fs::create_dir_all(root.join("video-projects/publish-project/exports")).unwrap();
        let source = root.join("publication-source.mp4");
        fs::write(&source, b"private source").unwrap();
        let payload = submit_media(
            &studio,
            "cover",
            &format!("publish-{}", Uuid::new_v4()),
            &source,
            &json!({
                "kind": "cover",
                "stem": "durable",
                "project_id": "publish-project",
                "project_revision": 1,
                "document_sha256": "a".repeat(64),
                "variant_key": "v001-en-9x16",
                "variant": {
                    "language": "EN",
                    "aspect": "9:16",
                    "subtitles": null,
                    "watermark": null,
                    "cta": "Buy"
                },
                "spec": {
                    "source_video": "source.mp4",
                    "frame_timestamp": 0.5,
                    "layout_profile": "smoke-glass",
                    "title": "Durable",
                    "subtitle": ""
                }
            }),
            Some("publish-project"),
        )
        .unwrap();
        let job = studio.database.claim_next_render_job().unwrap().unwrap();
        assert_eq!(payload["job"]["id"], job.id);
        let job_dir = studio.settings.render_jobs_dir().join(&job.id);
        let output = job_dir.join("attempt-1/output");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("first.bin"), b"first verified bytes").unwrap();
        fs::write(output.join("second.bin"), b"second verified bytes").unwrap();
        let prefix = if nested_project_output {
            format!("exports/{}/", job.id)
        } else {
            "exports/".to_string()
        };
        let files = vec![
            publication_file(
                &job_dir,
                &output.join("first.bin"),
                &format!("{prefix}first.bin"),
            )
            .unwrap(),
            publication_file(
                &job_dir,
                &output.join("second.bin"),
                &format!("{prefix}second.bin"),
            )
            .unwrap(),
        ];
        let intent = PublicationIntent {
            schema_version: 1,
            job_id: job.id.clone(),
            kind: job.kind.clone(),
            project_id: job.project_id.clone(),
            project_revision: job.project_revision,
            document_sha256: job.document_sha.clone(),
            attestation: Some(json!({"test": "trusted"})),
            files,
        };
        (studio, job, intent)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn durable_publication_forward_completes_every_crash_boundary() {
        for boundary in ["before-first", "after-first", "after-all", "same-hash"] {
            let root = tempdir().unwrap();
            let (studio, job, intent) = publication_fixture(root.path(), boundary == "after-all");
            studio
                .database
                .set_render_publication_intent(
                    &job.id,
                    Some(0),
                    &serde_json::to_string(&intent).unwrap(),
                )
                .unwrap();
            if boundary == "after-first" {
                publish_one_file(&studio, &job, &intent, 0, &intent.files[0]).unwrap();
            } else if boundary == "after-all" {
                for (index, file) in intent.files.iter().enumerate() {
                    publish_one_file(&studio, &job, &intent, index, file).unwrap();
                }
            } else if boundary == "same-hash" {
                let exports = root.path().join("video-projects/publish-project/exports");
                fs::write(exports.join("first.bin"), b"first verified bytes").unwrap();
                fs::write(exports.join("second.bin"), b"second verified bytes").unwrap();
            }

            recover_publications(&studio).unwrap();
            let completed = studio.database.render_job_by_id(&job.id).unwrap().unwrap();
            assert_eq!(completed.status, "succeeded");
            assert!(completed.publication_intent.is_none());
            assert!(completed.cleanup_pending);
            for file in &intent.files {
                let published = root
                    .path()
                    .join("video-projects/publish-project")
                    .join(&file.destination_relative);
                assert_eq!(hash_file(&published).unwrap(), file.sha256);
            }
            sweep_terminal_cleanup(&studio).unwrap();
            let cleaned = studio.database.render_job_by_id(&job.id).unwrap().unwrap();
            assert!(!cleaned.cleanup_pending);
            assert!(!studio
                .settings
                .render_jobs_dir()
                .join(&job.id)
                .join("attempt-1")
                .exists());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_conflict_blocks_claims_and_preserves_existing_bytes() {
        let root = tempdir().unwrap();
        let (studio, job, intent) = publication_fixture(root.path(), false);
        studio
            .database
            .set_render_publication_intent(
                &job.id,
                Some(0),
                &serde_json::to_string(&intent).unwrap(),
            )
            .unwrap();
        let conflict = root
            .path()
            .join("video-projects/publish-project")
            .join(&intent.files[0].destination_relative);
        fs::write(&conflict, b"preexisting conflict").unwrap();

        assert!(recover_publications(&studio).is_err());
        assert_eq!(fs::read(&conflict).unwrap(), b"preexisting conflict");
        let blocked = studio.database.render_job_by_id(&job.id).unwrap().unwrap();
        assert_eq!(blocked.status, "running");
        assert!(blocked.publication_intent.is_some());
        assert!(blocked
            .recovery_blocked
            .as_deref()
            .is_some_and(|value| value.contains("publication")));
        assert!(studio.database.claim_next_render_job().unwrap().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cancel_before_publication_intent_wins_without_public_files_and_restart_unblocks() {
        let root = tempdir().unwrap();
        let (first_studio, job, intent) = publication_fixture(root.path(), false);
        let source = root.path().join("publication-source.mp4");
        let next = submit_media(
            &first_studio,
            "analysis_frames",
            "next-after-cancel",
            &source,
            &json!({"kind":"analysis_frames","video_path":"source.mp4"}),
            None,
        )
        .unwrap();

        let canceled = first_studio
            .database
            .request_cancel_render_job(&job.id)
            .unwrap()
            .unwrap();
        assert!(canceled.cancel_requested);
        assert!(canceled.publication_intent.is_none());
        let outcome = first_studio
            .database
            .set_render_publication_intent(
                &job.id,
                Some(0),
                &serde_json::to_string(&intent).unwrap(),
            )
            .unwrap();
        assert_eq!(outcome, PublicationIntentOutcome::CancelWon);
        let row = first_studio
            .database
            .render_job_by_id(&job.id)
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "canceled");
        assert!(row.cleanup_pending);
        assert!(row.publication_intent.is_none());
        for file in &intent.files {
            assert!(!root
                .path()
                .join("video-projects/publish-project")
                .join(&file.destination_relative)
                .exists());
        }

        drop(first_studio);
        let reopened = studio(root.path());
        recover_publications(&reopened).unwrap();
        sweep_terminal_cleanup(&reopened).unwrap();
        assert_eq!(
            reopened
                .database
                .claim_next_render_job()
                .unwrap()
                .unwrap()
                .id,
            next["job"]["id"]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_intent_before_cancel_rejects_cancel_and_restart_completes() {
        let root = tempdir().unwrap();
        let (first_studio, job, intent) = publication_fixture(root.path(), false);
        let outcome = first_studio
            .database
            .set_render_publication_intent(
                &job.id,
                Some(0),
                &serde_json::to_string(&intent).unwrap(),
            )
            .unwrap();
        assert_eq!(outcome, PublicationIntentOutcome::Entered);

        let after_cancel = first_studio
            .database
            .request_cancel_render_job(&job.id)
            .unwrap()
            .unwrap();
        assert!(!after_cancel.cancel_requested);
        assert!(after_cancel.publication_intent.is_some());
        let direct = rusqlite::Connection::open(first_studio.database.path()).unwrap();
        assert!(direct
            .execute(
                "UPDATE render_jobs SET cancel_requested=1 WHERE id=?1",
                rusqlite::params![job.id],
            )
            .is_err());

        drop(direct);
        drop(first_studio);
        let reopened = studio(root.path());
        recover_publications(&reopened).unwrap();
        let completed = reopened
            .database
            .render_job_by_id(&job.id)
            .unwrap()
            .unwrap();
        assert_eq!(completed.status, "succeeded");
        assert!(!completed.cancel_requested);
        assert!(completed.publication_intent.is_none());
        for file in &intent.files {
            assert_eq!(
                hash_file(
                    &root
                        .path()
                        .join("video-projects/publish-project")
                        .join(&file.destination_relative)
                )
                .unwrap(),
                file.sha256
            );
        }
    }
}
