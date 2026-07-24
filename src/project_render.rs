use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::studio::Studio;
use crate::timeline::{OverlayTrack, TimelineEdl};
use crate::vpe::{CutSpec, HoldSource, VpeDocument};

const CANONICAL_EDL: &str = "canonical-edl.json";
const EFFECT_WHITELIST: &[&str] = &["grayscale", "vignette"];
const TRANSITION_WHITELIST: &[&str] = &["cross_dissolve"];

#[derive(Debug)]
pub struct CompiledProjectRender {
    pub snapshot_dir: PathBuf,
    pub canonical_edl: PathBuf,
    pub assets_root: PathBuf,
    pub output_dir: PathBuf,
    pub bundle_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetIdentity {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    pub has_audio: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CanonicalSource {
    path: String,
    sha256: String,
    size: u64,
    duration: f64,
    width: u32,
    height: u32,
    has_audio: bool,
}

#[derive(Debug, Serialize)]
struct ProjectIdentity<'a> {
    id: &'a str,
    revision: i64,
    document_sha256: &'a str,
}

#[derive(Debug, Serialize)]
struct CanonicalPlan<'a> {
    schema_version: u32,
    project: ProjectIdentity<'a>,
    canvas: &'a crate::vpe::CanvasSpec,
    sources: BTreeMap<String, CanonicalSource>,
    timeline: &'a TimelineEdl,
    cuts: &'a [CutSpec],
    hold_sources: &'a [HoldSource],
    asset_identities: Vec<AssetIdentity>,
}

pub fn compile(
    studio: &Studio,
    job_id: &str,
    project_dir: &Path,
    project_id: &str,
    revision: i64,
    document_sha256: &str,
    document: &VpeDocument,
) -> Result<CompiledProjectRender> {
    validate_edit_operations(document)?;

    let project_root = project_dir
        .canonicalize()
        .context("canonicalize video project directory")?;
    if !project_root.is_dir() {
        bail!("video project directory is unavailable");
    }
    let assets_root = AssetRoot::open(&project_root)?;
    let job_dir = studio.settings.render_jobs_dir().join(job_id);
    reject_symlink_components(&job_dir, true, "render job directory")?;
    fs::create_dir(&job_dir).context("create private render job directory")?;
    #[cfg(unix)]
    fs::set_permissions(&job_dir, fs::Permissions::from_mode(0o700))?;
    let snapshot_dir = job_dir.join("snapshot");
    fs::create_dir(&snapshot_dir).context("create private render snapshot")?;
    #[cfg(unix)]
    fs::set_permissions(&snapshot_dir, fs::Permissions::from_mode(0o700))?;
    let frozen_assets_root = snapshot_dir.join("assets");
    fs::create_dir(&frozen_assets_root).context("create frozen asset directory")?;
    #[cfg(unix)]
    fs::set_permissions(&frozen_assets_root, fs::Permissions::from_mode(0o700))?;

    let mut identities = BTreeMap::<String, AssetIdentity>::new();
    let mut sources = BTreeMap::new();
    for (alias, path) in &document.sources {
        let normalized = relative_path_string(&normalized_asset_path(path)?);
        let identity = match identities.get(&normalized) {
            Some(identity) => identity.clone(),
            None => {
                let identity = freeze_asset(&assets_root, path, true, &frozen_assets_root)?;
                identities.insert(identity.path.clone(), identity.clone());
                identity
            }
        };
        let duration = identity
            .duration
            .ok_or_else(|| anyhow!("source '{alias}' has no video duration"))?;
        let width = identity
            .width
            .ok_or_else(|| anyhow!("source '{alias}' has no video width"))?;
        let height = identity
            .height
            .ok_or_else(|| anyhow!("source '{alias}' has no video height"))?;
        sources.insert(
            alias.clone(),
            CanonicalSource {
                path: identity.path.clone(),
                sha256: identity.sha256.clone(),
                size: identity.size,
                duration,
                width,
                height,
                has_audio: identity.has_audio,
            },
        );
    }
    for clip in document
        .timeline
        .main_tracks
        .iter()
        .flat_map(|track| &track.clips)
        .chain(
            document
                .timeline
                .overlay_tracks
                .iter()
                .filter_map(|overlay| match overlay {
                    OverlayTrack::Broll { clip, .. } | OverlayTrack::Pip { clip, .. } => Some(clip),
                    _ => None,
                }),
        )
    {
        let source = sources
            .get(&clip.source)
            .ok_or_else(|| anyhow!("clip references unknown source '{}'", clip.source))?;
        if clip.source_out > source.duration + 0.000_001 {
            bail!("clip source range exceeds media duration");
        }
    }
    for (overlay, hold) in document
        .timeline
        .overlay_tracks
        .iter()
        .filter(|item| matches!(item, OverlayTrack::Hold { .. }))
        .zip(&document.hold_sources)
    {
        let OverlayTrack::Hold { source_time, .. } = overlay else {
            unreachable!()
        };
        let source = sources
            .get(&hold.source)
            .ok_or_else(|| anyhow!("hold references unknown source '{}'", hold.source))?;
        if *source_time > source.duration {
            bail!("hold source_time exceeds media duration");
        }
    }
    for variant in &document.timeline.variants {
        for path in [&variant.subtitles, &variant.watermark]
            .into_iter()
            .flatten()
        {
            let normalized = relative_path_string(&normalized_asset_path(path)?);
            if !identities.contains_key(&normalized) {
                let identity = freeze_asset(&assets_root, path, false, &frozen_assets_root)?;
                identities.insert(identity.path.clone(), identity);
            }
        }
    }

    let plan = CanonicalPlan {
        schema_version: 1,
        project: ProjectIdentity {
            id: project_id,
            revision,
            document_sha256,
        },
        canvas: &document.canvas,
        sources,
        timeline: &document.timeline,
        cuts: &document.cuts,
        hold_sources: &document.hold_sources,
        asset_identities: identities.into_values().collect(),
    };
    let mut bytes = serde_json::to_vec(&plan)?;
    bytes.push(b'\n');
    let bundle_hash = bytes_sha256(&bytes);

    let canonical_edl = snapshot_dir.join(CANONICAL_EDL);
    write_new_synced(&canonical_edl, &bytes)?;
    sync_directory(&frozen_assets_root)?;
    sync_directory(&snapshot_dir)?;
    sync_directory(&job_dir)?;

    Ok(CompiledProjectRender {
        snapshot_dir,
        canonical_edl,
        assets_root: frozen_assets_root,
        output_dir: project_root.join("exports").join(job_id),
        bundle_hash,
    })
}

pub fn validate_snapshot(path: &Path, expected_hash: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).context("inspect canonical EDL")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("canonical EDL must be a regular non-symlink file");
    }
    if hash_file_nofollow(path)? != expected_hash {
        bail!("canonical EDL bundle hash does not match");
    }
    Ok(())
}

pub fn revalidate_assets(canonical_edl: &Path, assets_root: &Path) -> Result<()> {
    let plan: Value = serde_json::from_slice(&fs::read(canonical_edl)?)?;
    let identities = plan["asset_identities"]
        .as_array()
        .ok_or_else(|| anyhow!("canonical EDL has no asset identities"))?;
    for expected in identities {
        let path = expected["path"]
            .as_str()
            .ok_or_else(|| anyhow!("asset identity path is invalid"))?;
        let relative = normalized_asset_path(path)?;
        let candidate = assets_root.join(relative);
        reject_symlink_components(&candidate, false, "frozen project asset")?;
        let metadata = fs::symlink_metadata(&candidate)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || hash_file_nofollow(&candidate)? != expected["sha256"].as_str().unwrap_or_default()
            || metadata.len() != expected["size"].as_u64().unwrap_or(u64::MAX)
        {
            bail!("frozen project asset identity mismatch");
        }
    }
    Ok(())
}

fn validate_edit_operations(document: &VpeDocument) -> Result<()> {
    document.validate().map_err(anyhow::Error::msg)?;
    let mut transition_boundaries = BTreeSet::new();
    for transition in &document.timeline.transitions {
        if !TRANSITION_WHITELIST.contains(&transition.kind.as_str()) {
            bail!("unsupported transition '{}'", transition.kind);
        }
        let track = document
            .timeline
            .main_tracks
            .iter()
            .find(|track| track.name == transition.track)
            .context("transition references unknown main track")?;
        let Some(index) = track
            .clips
            .iter()
            .take(track.clips.len().saturating_sub(1))
            .position(|clip| {
                ordered_time(clip.timeline_out) == ordered_time(transition.timeline_time)
            })
        else {
            bail!("transition must identify a main track clip boundary");
        };
        let boundary = (
            transition.track.clone(),
            ordered_time(transition.timeline_time),
        );
        if !transition_boundaries.insert(boundary) {
            bail!("transition must identify one unique main track clip boundary");
        }
        let outgoing = track.clips[index].timeline_out - track.clips[index].timeline_in;
        let incoming = track.clips[index + 1].timeline_out - track.clips[index + 1].timeline_in;
        if transition.duration > outgoing || transition.duration > incoming {
            bail!("cross dissolve exceeds an adjacent clip duration");
        }
    }
    let hold_count = document
        .timeline
        .overlay_tracks
        .iter()
        .filter(|item| matches!(item, OverlayTrack::Hold { .. }))
        .count();
    if hold_count != document.hold_sources.len() {
        bail!("hold source mapping is incomplete");
    }
    for overlay in &document.timeline.overlay_tracks {
        if let OverlayTrack::Effect { name, .. } = overlay {
            if !EFFECT_WHITELIST.contains(&name.as_str()) {
                bail!("unsupported effect '{name}'");
            }
        }
    }
    Ok(())
}

fn ordered_time(value: f64) -> i64 {
    (value * 1_000_000.0).round() as i64
}

struct AssetRoot {
    directory: File,
}

impl AssetRoot {
    fn open(project_root: &Path) -> Result<Self> {
        let project =
            open_directory_nofollow(project_root).context("open pinned video project directory")?;
        let assets = openat_directory(project.as_raw_fd(), std::ffi::OsStr::new("assets"))
            .context("open pinned project assets directory")?;
        Ok(Self { directory: assets })
    }

    fn open_regular(&self, relative: &Path) -> Result<File> {
        let components = relative
            .components()
            .map(|component| match component {
                Component::Normal(name) if !name.is_empty() => Ok(name),
                _ => bail!("project asset path must be a safe relative path"),
            })
            .collect::<Result<Vec<_>>>()?;
        let (last, parents) = components
            .split_last()
            .ok_or_else(|| anyhow!("project asset path must not be empty"))?;
        let mut directory = duplicate_fd(self.directory.as_raw_fd())?;
        for component in parents {
            directory = openat_directory(directory.as_raw_fd(), component)?;
        }
        openat_regular(directory.as_raw_fd(), last)
    }
}

fn freeze_asset(
    assets_root: &AssetRoot,
    value: &str,
    require_video: bool,
    frozen_assets_root: &Path,
) -> Result<AssetIdentity> {
    let relative = normalized_asset_path(value)?;
    let mut input = assets_root
        .open_regular(&relative)
        .with_context(|| format!("open project asset {}", relative.display()))?;
    let source_metadata = input.metadata()?;
    if !source_metadata.is_file() {
        bail!("project asset must be a regular non-symlink file");
    }
    let destination = frozen_assets_root.join(&relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut output_options = OpenOptions::new();
    output_options
        .create_new(true)
        .write(true)
        .mode(0o400)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut output = output_options.open(&destination)?;
    let mut hasher = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = std::io::Read::read(&mut input, &mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        copied += read as u64;
    }
    if copied != source_metadata.len() {
        bail!("project asset size changed while freezing");
    }
    output.sync_all()?;
    drop(output);
    sync_directory(
        destination
            .parent()
            .ok_or_else(|| anyhow!("frozen asset has no parent"))?,
    )?;
    let sha256 = format!("{:x}", hasher.finalize());
    if hash_file_nofollow(&destination)? != sha256 {
        bail!("frozen project asset hash verification failed");
    }
    let media = probe_media(&destination)?;
    if require_video && media.duration.is_none() {
        bail!("source asset is not a decodable video");
    }
    Ok(AssetIdentity {
        path: relative_path_string(&relative),
        sha256,
        size: copied,
        duration: media.duration,
        width: media.width,
        height: media.height,
        has_audio: media.has_audio,
    })
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options.open(path)?;
    if !file.metadata()?.is_dir() {
        bail!("path is not a directory");
    }
    Ok(file)
}

#[cfg(unix)]
fn duplicate_fd(fd: i32) -> Result<File> {
    // SAFETY: fcntl duplicates the supplied live directory descriptor and the
    // returned descriptor is immediately owned by File.
    let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicated < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: duplicated is a fresh descriptor owned by this function.
    Ok(unsafe { File::from_raw_fd(duplicated) })
}

#[cfg(unix)]
fn openat_directory(parent: i32, name: &std::ffi::OsStr) -> Result<File> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(name.as_bytes()).context("directory name contains NUL")?;
    // SAFETY: parent is a live pinned directory descriptor and name is NUL
    // terminated. O_NOFOLLOW prevents replacement with a symlink.
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: fd is newly returned and uniquely owned.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn openat_regular(parent: i32, name: &std::ffi::OsStr) -> Result<File> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(name.as_bytes()).context("file name contains NUL")?;
    // SAFETY: parent is a live pinned directory descriptor and O_NOFOLLOW
    // rejects a final symlink.
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
    // SAFETY: fd is newly returned and uniquely owned.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        bail!("project asset must be a regular file");
    }
    Ok(file)
}

fn normalized_asset_path(value: &str) -> Result<PathBuf> {
    let mut path = Path::new(value);
    if path
        .components()
        .next()
        .is_some_and(|component| matches!(component, Component::Normal(name) if name == "assets"))
    {
        path = path.strip_prefix("assets").context("strip assets prefix")?;
    }
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("asset path must be canonical and relative to project/assets");
    }
    Ok(path.to_path_buf())
}

#[derive(Debug)]
struct MediaProbe {
    duration: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
    has_audio: bool,
}

fn probe_media(path: &Path) -> Result<MediaProbe> {
    let output = Command::new("/usr/bin/ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type,width,height",
            "-of",
            "json",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("run ffprobe for project asset")?;
    if !output.status.success() {
        bail!("ffprobe rejected a project asset");
    }
    let value: Value = serde_json::from_slice(&output.stdout).context("parse ffprobe output")?;
    let streams = value["streams"].as_array().cloned().unwrap_or_default();
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video");
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|item| item.parse::<f64>().ok())
        .filter(|item| item.is_finite() && *item > 0.0);
    Ok(MediaProbe {
        duration: video.and(duration),
        width: video
            .and_then(|stream| stream["width"].as_u64())
            .and_then(|value| u32::try_from(value).ok()),
        height: video
            .and_then(|stream| stream["height"].as_u64())
            .and_then(|value| u32::try_from(value).ok()),
        has_audio: streams.iter().any(|stream| stream["codec_type"] == "audio"),
    })
}

fn hash_file_nofollow(path: &Path) -> Result<String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        bail!("asset is not a regular file");
    }
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn bytes_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn reject_symlink_components(path: &Path, allow_missing_final: bool, label: &str) -> Result<()> {
    let component_count = path.components().count();
    let mut current = PathBuf::new();
    for (index, component) in path.components().enumerate() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            bail!("{label} contains a non-canonical path component");
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("{label} contains a symlink component");
            }
            Ok(_) => {}
            Err(error)
                if allow_missing_final
                    && index + 1 == component_count
                    && error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("inspect {label}")),
        }
    }
    Ok(())
}

fn relative_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn pinned_asset_root_rejects_parent_swap_import() {
        let root = tempdir().unwrap();
        let project = root.path().join("project");
        let original = project.join("assets");
        let external = root.path().join("external");
        fs::create_dir_all(original.join("nested")).unwrap();
        fs::create_dir_all(&external).unwrap();
        fs::write(original.join("nested/source.bin"), b"trusted").unwrap();
        fs::write(external.join("source.bin"), b"external").unwrap();

        let pinned = AssetRoot::open(&project).unwrap();
        fs::rename(&original, project.join("assets-old")).unwrap();
        symlink(&external, &original).unwrap();

        let mut file = pinned.open_regular(Path::new("nested/source.bin")).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"trusted");
        assert_ne!(bytes, fs::read(external.join("source.bin")).unwrap());
    }
}
