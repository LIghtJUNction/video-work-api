use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

use crate::provenance::{self, AuditReceipt, ReceiptError};
use crate::quality::Phase;
use crate::vpe;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_dry_run() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRequest {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupResult {
    pub dry_run: bool,
    pub selected: Vec<String>,
    pub removed: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRequest {
    pub deliverable_stem: String,
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveResult {
    pub dry_run: bool,
    pub archived: bool,
    pub idempotent: bool,
    pub files: Vec<String>,
    pub archive_directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchiveJournal {
    version: u32,
    project_id: String,
    revision: i64,
    document_sha256: String,
    acceptance_receipt_hash: String,
    final_relative: String,
    entries: Vec<ArchiveEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchiveEntry {
    identity: String,
    source_relative: String,
    staged_relative: String,
    sha256: String,
}

const TEMP_ROOTS: &[&str] = &["cache", "proxies", "passes"];
const TEMP_FILE_SUFFIXES: &[&str] = &[".cache", ".proxy.mp4", ".pass.wav"];

pub fn cleanup_intermediates(
    project_dir: &Path,
    request: &CleanupRequest,
) -> Result<CleanupResult> {
    let root = canonical_project(project_dir)?;
    let _lock = acquire_project_lock(&root)?;
    let mut selected = Vec::new();
    let mut missing = Vec::new();
    let mut candidates = Vec::new();
    let mut unique = HashSet::new();
    for raw in &request.paths {
        let relative = safe_relative(raw)?;
        if !allowed_temporary(&relative) {
            bail!("cleanup path is not a service-defined intermediate: {raw}");
        }
        let normalized = display_relative(&relative);
        if !unique.insert(normalized.clone()) {
            continue;
        }
        match inspect_relative(&root, &relative)? {
            None => missing.push(normalized),
            Some(is_directory) => {
                if is_directory && directory_not_empty(&root.join(&relative))? {
                    bail!("cleanup only removes empty allowlisted directories: {raw}");
                }
                selected.push(normalized.clone());
                candidates.push((normalized, relative, is_directory));
            }
        }
    }
    selected.sort();
    missing.sort();
    let mut removed = Vec::new();
    if !request.dry_run {
        candidates.sort_by_key(|item| std::cmp::Reverse(item.1.components().count()));
        for (relative, path, directory) in candidates {
            remove_relative_nofollow(&root, &path, directory)
                .with_context(|| format!("remove intermediate {relative}"))?;
            removed.push(relative);
        }
        removed.sort();
    }
    Ok(CleanupResult {
        dry_run: request.dry_run,
        selected,
        removed,
        missing,
    })
}

pub fn archive_completed_sources(
    project_dir: &Path,
    request: &ArchiveRequest,
    receipt_key_file: &Path,
) -> Result<ArchiveResult> {
    validate_stem(&request.deliverable_stem)?;
    let root = canonical_project(project_dir)?;
    let _lock = acquire_project_lock(&root)?;
    ensure_delivery_set_preserved(&root, &request.deliverable_stem)?;
    let (revision, receipts) = verified_acceptance_chain(&root, receipt_key_file)?;
    let pre_render = &receipts[0];
    let acceptance = &receipts[2];
    let project_id = receipt_string(acceptance, "project_id")?.to_owned();
    let document_sha256 = receipt_string(acceptance, "document_sha256")?.to_owned();
    let document_path = root.join("project.vpe");
    let (document_bytes, actual_document_sha256) =
        provenance::read_regular_with_sha256(&document_path)?;
    if actual_document_sha256 != document_sha256 {
        bail!("current project.vpe does not match the accepted revision");
    }
    let document_text = std::str::from_utf8(&document_bytes).context("project.vpe is not UTF-8")?;
    let document = vpe::parse(document_text).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let source_hashes = pre_render
        .canonical_params
        .get("source_sha256")
        .and_then(Value::as_object)
        .context("pre-render receipt does not bind source_sha256")?;
    let archive_relative = format!("archive/{}", request.deliverable_stem);
    let final_dir = root.join(&archive_relative);
    let archive_root = ensure_archive_root(&root)?;
    let journal_path = archive_root.join(format!(
        ".{}.archive-journal.json",
        request.deliverable_stem
    ));
    let staging_name = format!(".{}.archive-staging", request.deliverable_stem);
    let staging_dir = archive_root.join(&staging_name);

    if final_dir.exists() {
        let journal = read_journal_if_present(&journal_path)?
            .context("archive destination exists without its durable journal")?;
        verify_completed_archive(&root, &final_dir, &journal)?;
        if journal.acceptance_receipt_hash != acceptance.receipt_hash {
            bail!("existing archive belongs to another acceptance receipt");
        }
        return Ok(ArchiveResult {
            dry_run: request.dry_run,
            archived: true,
            idempotent: true,
            files: journal
                .entries
                .iter()
                .map(|entry| entry.source_relative.clone())
                .collect(),
            archive_directory: archive_relative,
        });
    }

    let journal = if let Some(journal) = read_journal_if_present(&journal_path)? {
        if journal.acceptance_receipt_hash != acceptance.receipt_hash
            || journal.revision != revision
            || journal.final_relative != archive_relative
        {
            bail!("in-progress archive journal conflicts with current acceptance");
        }
        journal
    } else {
        let mut entries = Vec::new();
        for (identity, relative) in &document.sources {
            let expected = source_hashes
                .get(identity)
                .and_then(Value::as_str)
                .with_context(|| format!("receipt does not bind source identity '{identity}'"))?;
            let source = checked_source_path(&root, relative)?;
            let actual = provenance::sha256_file(&source)?;
            if actual != expected {
                bail!("source hash mismatch for identity '{identity}'");
            }
            entries.push(ArchiveEntry {
                identity: identity.clone(),
                source_relative: relative.clone(),
                staged_relative: relative.clone(),
                sha256: actual,
            });
        }
        if entries.is_empty() {
            bail!("project has no source assets to archive");
        }
        let journal = ArchiveJournal {
            version: 1,
            project_id,
            revision,
            document_sha256,
            acceptance_receipt_hash: acceptance.receipt_hash.clone(),
            final_relative: archive_relative.clone(),
            entries,
        };
        if request.dry_run {
            return Ok(ArchiveResult {
                dry_run: true,
                archived: false,
                idempotent: false,
                files: journal
                    .entries
                    .iter()
                    .map(|entry| entry.source_relative.clone())
                    .collect(),
                archive_directory: archive_relative,
            });
        }
        write_journal(&journal_path, &journal)?;
        journal
    };

    if request.dry_run {
        return Ok(ArchiveResult {
            dry_run: true,
            archived: false,
            idempotent: false,
            files: journal
                .entries
                .iter()
                .map(|entry| entry.source_relative.clone())
                .collect(),
            archive_directory: archive_relative,
        });
    }
    ensure_staging(&staging_dir)?;
    for entry in &journal.entries {
        forward_complete_entry(&root, &staging_dir, entry)?;
    }
    sync_dir(&staging_dir)?;
    if final_dir.exists() {
        bail!("archive final destination appeared during transaction");
    }
    fs::rename(&staging_dir, &final_dir).context("publish completed source archive")?;
    sync_dir(&archive_root)?;
    verify_completed_archive(&root, &final_dir, &journal)?;
    Ok(ArchiveResult {
        dry_run: false,
        archived: true,
        idempotent: false,
        files: journal
            .entries
            .iter()
            .map(|entry| entry.source_relative.clone())
            .collect(),
        archive_directory: archive_relative,
    })
}

fn verified_acceptance_chain(root: &Path, key_file: &Path) -> Result<(i64, Vec<AuditReceipt>)> {
    let receipts_root = checked_directory(&root.join("receipts"), root)?;
    let mut revisions = fs::read_dir(&receipts_root)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_prefix("rev-")?.parse::<i64>().ok()
        })
        .collect::<Vec<_>>();
    revisions.sort_unstable();
    let revision = *revisions
        .last()
        .context("no revision-scoped receipts exist")?;
    let paths = [Phase::PreRender, Phase::PrePackage, Phase::Acceptance]
        .into_iter()
        .map(|phase| {
            receipts_root
                .join(format!("rev-{revision}"))
                .join(phase.as_str())
                .join("audit_receipt.json")
        })
        .collect::<Vec<_>>();
    if paths.iter().any(|path| !path.exists()) {
        bail!("archive requires the complete pre-render to acceptance receipt chain");
    }
    let receipts = provenance::verify_chain(&paths, key_file).map_err(|error| {
        anyhow::anyhow!(match error {
            ReceiptError::CapabilityUnavailable =>
                "archive receipt verification capability unavailable".to_string(),
            other => format!("archive receipt chain verification failed: {other}"),
        })
    })?;
    let mut identity: Option<(&str, i64, &str)> = None;
    for (receipt, phase) in
        receipts
            .iter()
            .zip([Phase::PreRender, Phase::PrePackage, Phase::Acceptance])
    {
        let project = receipt_string(receipt, "project_id")?;
        let bound_revision = receipt_i64(receipt, "revision")?;
        let document = receipt_string(receipt, "document_sha256")?;
        if bound_revision != revision
            || receipt_string(receipt, "phase")? != phase.as_str()
            || receipt
                .canonical_params
                .get("passed")
                .and_then(Value::as_bool)
                != Some(true)
        {
            bail!("receipt chain contains an invalid phase binding");
        }
        if let Some(expected) = identity {
            if expected != (project, bound_revision, document) {
                bail!("receipt chain crosses project revisions");
            }
        } else {
            identity = Some((project, bound_revision, document));
        }
        let report = receipts_root
            .join(format!("rev-{revision}"))
            .join(phase.as_str())
            .join("validation_report.json");
        provenance::verify_report_binding(receipt, &report)
            .map_err(|error| anyhow::anyhow!("validation report binding failed: {error}"))?;
    }
    Ok((revision, receipts))
}

fn receipt_string<'a>(receipt: &'a AuditReceipt, key: &str) -> Result<&'a str> {
    receipt
        .canonical_params
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("receipt lacks {key}"))
}

fn receipt_i64(receipt: &AuditReceipt, key: &str) -> Result<i64> {
    receipt
        .canonical_params
        .get(key)
        .and_then(Value::as_i64)
        .with_context(|| format!("receipt lacks {key}"))
}

fn ensure_delivery_set_preserved(root: &Path, stem: &str) -> Result<()> {
    let exports = checked_directory(&root.join("exports"), root)?;
    for name in delivery_names(stem) {
        let path = exports.join(&name);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("missing delivery artifact {name}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            bail!("delivery artifact {name} must remain a non-empty regular file");
        }
        provenance::sha256_file(&path)?;
    }
    Ok(())
}

fn checked_source_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = safe_relative(relative)?;
    if relative.components().next().and_then(|value| match value {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }) != Some("assets")
    {
        bail!("project source must live under project/assets");
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        bail!("source asset must be a non-empty regular non-symlink file");
    }
    provenance::sha256_file(&path)?;
    Ok(path)
}

fn forward_complete_entry(root: &Path, staging: &Path, entry: &ArchiveEntry) -> Result<()> {
    let source = root.join(&entry.source_relative);
    let destination = staging.join(&entry.staged_relative);
    if let Some(parent) = destination.parent() {
        create_relative_directories(staging, parent.strip_prefix(staging)?)?;
    }
    let source_exists = source.exists();
    let destination_exists = destination.exists();
    match (source_exists, destination_exists) {
        (true, false) => {
            if provenance::sha256_file(&source)? != entry.sha256 {
                bail!("source changed during archive: {}", entry.source_relative);
            }
            fs::rename(&source, &destination)?;
            sync_dir(source.parent().context("source has no parent")?)?;
            sync_dir(destination.parent().context("destination has no parent")?)?;
        }
        (false, true) => {
            if provenance::sha256_file(&destination)? != entry.sha256 {
                bail!("staged source hash mismatch: {}", entry.source_relative);
            }
        }
        (true, true) => bail!("archive conflict leaves source and staged copy both present"),
        (false, false) => bail!("archive source and staged copy are both missing"),
    }
    Ok(())
}

fn verify_completed_archive(root: &Path, final_dir: &Path, journal: &ArchiveJournal) -> Result<()> {
    let final_dir = checked_directory(final_dir, root)?;
    for entry in &journal.entries {
        if root.join(&entry.source_relative).exists() {
            bail!(
                "completed archive still has live source {}",
                entry.source_relative
            );
        }
        let archived = final_dir.join(&entry.staged_relative);
        if provenance::sha256_file(&archived)? != entry.sha256 {
            bail!("completed archive hash mismatch for {}", entry.identity);
        }
    }
    Ok(())
}

fn write_journal(path: &Path, journal: &ArchiveJournal) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(journal)?;
    provenance::write_immutable(path, &bytes)
        .map_err(|error| anyhow::anyhow!("create archive journal: {error}"))
}

fn read_journal_if_present(path: &Path) -> Result<Option<ArchiveJournal>> {
    let mut file = match provenance::open_regular_nofollow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn ensure_archive_root(root: &Path) -> Result<PathBuf> {
    let path = root.join("archive");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!("archive root must be a regular non-symlink directory")
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path)?;
            sync_dir(root)?;
        }
        Err(error) => return Err(error.into()),
    }
    checked_directory(&path, root)
}

fn ensure_staging(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!("archive staging must be a regular non-symlink directory")
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            sync_dir(path.parent().context("staging has no parent")?)?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn create_relative_directories(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            bail!("invalid archive subdirectory");
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                bail!("archive subdirectory is not a regular directory")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn canonical_project(project_dir: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(project_dir)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("project must be a regular non-symlink directory");
    }
    Ok(project_dir.canonicalize()?)
}

fn checked_directory(path: &Path, root: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("project subdirectory must be a regular non-symlink directory");
    }
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root) {
        bail!("project subdirectory escaped project root");
    }
    Ok(canonical)
}

fn safe_relative(raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if raw.is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("path must be a safe relative path");
    }
    Ok(path.to_path_buf())
}

fn allowed_temporary(path: &Path) -> bool {
    let mut components = path.components();
    let first = components.next().and_then(|component| match component {
        Component::Normal(value) => value.to_str(),
        _ => None,
    });
    if !first.is_some_and(|value| TEMP_ROOTS.contains(&value)) {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    TEMP_ROOTS.contains(&name)
        || TEMP_FILE_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn inspect_relative(root: &Path, relative: &Path) -> Result<Option<bool>> {
    reject_relative_symlink_components(root, relative)?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!("cleanup refuses symlink"),
        Ok(metadata) if metadata.is_file() => Ok(Some(false)),
        Ok(metadata) if metadata.is_dir() => Ok(Some(true)),
        Ok(_) => bail!("cleanup path has unsupported type"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn reject_relative_symlink_components(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("cleanup refuses a symlink path component")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn directory_not_empty(path: &Path) -> Result<bool> {
    Ok(fs::read_dir(path)?.next().transpose()?.is_some())
}

#[cfg(unix)]
fn remove_relative_nofollow(root: &Path, relative: &Path, directory: bool) -> Result<()> {
    let root_fd = open_directory_nofollow(root)?;
    let mut parent_fd = root_fd;
    let components = relative.components().collect::<Vec<_>>();
    for component in &components[..components.len().saturating_sub(1)] {
        let Component::Normal(name) = component else {
            bail!("invalid cleanup path");
        };
        let name = std::ffi::CString::new(name.as_encoded_bytes())?;
        let fd = unsafe {
            libc::openat(
                parent_fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        parent_fd = unsafe { File::from_raw_fd(fd) };
    }
    let name = relative
        .file_name()
        .context("cleanup path has no filename")?;
    let name = std::ffi::CString::new(name.as_encoded_bytes())?;
    let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
    if unsafe { libc::unlinkat(parent_fd.as_raw_fd(), name.as_ptr(), flags) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    parent_fd.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn remove_relative_nofollow(root: &Path, relative: &Path, directory: bool) -> Result<()> {
    let path = root.join(relative);
    if directory {
        fs::remove_dir(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    Ok(options.open(path)?)
}

fn acquire_project_lock(root: &Path) -> Result<File> {
    let path = root.join(".lifecycle.lock");
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options.open(path)?;
    #[cfg(unix)]
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(file)
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn display_relative(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn validate_stem(stem: &str) -> Result<()> {
    if stem.is_empty()
        || Path::new(stem).components().count() != 1
        || !stem
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("deliverable_stem must be a safe filename stem");
    }
    Ok(())
}

fn delivery_names(stem: &str) -> Vec<String> {
    vec![
        format!("{stem}.mp4"),
        format!("{stem}.txt"),
        format!("{stem}.jpg"),
        format!("{stem}-cover-original.png"),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::*;

    fn project() -> tempfile::TempDir {
        let root = tempdir().unwrap();
        for name in ["exports", "receipts", ".tmp", "cache", "proxies", "passes"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        root
    }

    #[test]
    fn cleanup_preserves_project_tmp_and_defaults_to_dry_run() {
        let root = project();
        fs::write(root.path().join(".tmp/core.tmp"), b"core").unwrap();
        fs::write(root.path().join("proxies/clip.proxy.mp4"), b"x").unwrap();
        assert!(cleanup_intermediates(
            root.path(),
            &CleanupRequest {
                paths: vec![".tmp/core.tmp".into()],
                dry_run: false,
            },
        )
        .is_err());
        let result = cleanup_intermediates(
            root.path(),
            &CleanupRequest {
                paths: vec!["proxies/clip.proxy.mp4".into()],
                dry_run: true,
            },
        )
        .unwrap();
        assert!(result.removed.is_empty());
        assert!(root.path().join("proxies/clip.proxy.mp4").exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_rejects_symlink_replacement() {
        use std::os::unix::fs::symlink;
        let root = project();
        symlink("/etc/hosts", root.path().join("cache/item.cache")).unwrap();
        assert!(cleanup_intermediates(
            root.path(),
            &CleanupRequest {
                paths: vec!["cache/item.cache".into()],
                dry_run: false,
            },
        )
        .is_err());
    }

    fn archived_project() -> (tempfile::TempDir, PathBuf) {
        let root = project();
        fs::create_dir(root.path().join("assets")).unwrap();
        fs::write(root.path().join("assets/a.mp4"), b"source-a").unwrap();
        fs::write(root.path().join("assets/b.mp4"), b"source-b").unwrap();
        let document = r#"project "Archive" {
  canvas 1920x1080 @ 30fps
  source a = "assets/a.mp4"
  source b = "assets/b.mp4"
  timeline {
    track main {
      clip a source 00:00:00.000..00:00:03.000 at 00:00:00.000
    }
  }
  marker "Opening hook" at 00:00:03.000
}"#;
        fs::write(root.path().join("project.vpe"), document).unwrap();
        for name in delivery_names("clip") {
            fs::write(root.path().join("exports").join(name), b"delivery").unwrap();
        }
        let key = root.path().join("key");
        fs::write(&key, b"test-only-secret").unwrap();
        let document_sha256 = provenance::sha256_file(&root.path().join("project.vpe")).unwrap();
        let source_hashes = BTreeMap::from([
            (
                "a".to_owned(),
                provenance::sha256_file(&root.path().join("assets/a.mp4")).unwrap(),
            ),
            (
                "b".to_owned(),
                provenance::sha256_file(&root.path().join("assets/b.mp4")).unwrap(),
            ),
        ]);
        let mut previous = None;
        for phase in [Phase::PreRender, Phase::PrePackage, Phase::Acceptance] {
            let scope = format!("rev-1/{}", phase.as_str());
            let directory = provenance::prepare_receipt_scope(root.path(), &scope).unwrap();
            let report = directory.join("validation_report.json");
            fs::write(
                &report,
                serde_json::to_vec(&serde_json::json!({
                    "phase": phase.as_str(),
                    "passed": true
                }))
                .unwrap(),
            )
            .unwrap();
            let report_hash = provenance::sha256_file(&report).unwrap();
            let params = serde_json::json!({
                "phase": phase.as_str(),
                "project_id": "archive-project",
                "revision": 1,
                "document_sha256": document_sha256,
                "validation_report_sha256": report_hash,
                "passed": true,
                "gate_results": [],
                "source_sha256": if phase == Phase::PreRender {
                    serde_json::to_value(&source_hashes).unwrap()
                } else {
                    serde_json::json!({})
                },
                "output_sha256": {},
                "previous_receipt_hash": previous,
            });
            let (receipt, _) = provenance::create_receipt(
                root.path(),
                &scope,
                if phase == Phase::PreRender {
                    source_hashes.clone()
                } else {
                    BTreeMap::new()
                },
                params,
                previous.clone(),
                &key,
            )
            .unwrap();
            previous = Some(receipt.receipt_hash);
        }
        (root, key)
    }

    #[test]
    fn archive_resumes_partial_source_moves_and_preserves_outputs() {
        let (root, key) = archived_project();
        let archive_root = ensure_archive_root(root.path()).unwrap();
        let staging = archive_root.join(".clip.archive-staging");
        ensure_staging(&staging).unwrap();
        let source_hashes = ["a", "b"]
            .into_iter()
            .map(|identity| {
                (
                    identity.to_owned(),
                    provenance::sha256_file(&root.path().join(format!("assets/{identity}.mp4")))
                        .unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let acceptance = provenance::verify_receipt(
            &root
                .path()
                .join("receipts/rev-1/acceptance/audit_receipt.json"),
            &key,
        )
        .unwrap();
        let journal = ArchiveJournal {
            version: 1,
            project_id: "archive-project".into(),
            revision: 1,
            document_sha256: provenance::sha256_file(&root.path().join("project.vpe")).unwrap(),
            acceptance_receipt_hash: acceptance.receipt_hash,
            final_relative: "archive/clip".into(),
            entries: ["a", "b"]
                .into_iter()
                .map(|identity| ArchiveEntry {
                    identity: identity.into(),
                    source_relative: format!("assets/{identity}.mp4"),
                    staged_relative: format!("assets/{identity}.mp4"),
                    sha256: source_hashes[identity].clone(),
                })
                .collect(),
        };
        write_journal(&archive_root.join(".clip.archive-journal.json"), &journal).unwrap();
        create_relative_directories(&staging, Path::new("assets")).unwrap();
        fs::rename(
            root.path().join("assets/a.mp4"),
            staging.join("assets/a.mp4"),
        )
        .unwrap();

        let request = ArchiveRequest {
            deliverable_stem: "clip".into(),
            dry_run: false,
        };
        let result = archive_completed_sources(root.path(), &request, &key).unwrap();
        assert!(result.archived && !result.idempotent);
        assert!(root.path().join("archive/clip/assets/a.mp4").is_file());
        assert!(root.path().join("archive/clip/assets/b.mp4").is_file());
        for name in delivery_names("clip") {
            assert!(root.path().join("exports").join(name).is_file());
        }
        let repeated = archive_completed_sources(root.path(), &request, &key).unwrap();
        assert!(repeated.idempotent);
    }
}
