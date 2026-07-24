//! Persistent MCP bearer-token storage.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use uuid::Uuid;

/// Read a persistent token from a regular, non-symlink file.
pub fn load(path: &Path) -> Result<Option<String>> {
    validate_parent_if_present(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| "open MCP token file safely"),
    };
    let metadata = file
        .metadata()
        .with_context(|| "inspect opened MCP token file")?;
    validate_metadata(&metadata)?;

    let mut raw = Vec::new();
    file.read_to_end(&mut raw)
        .with_context(|| "read MCP token file")?;
    let token = strip_one_line_ending(&raw)?;
    validate_token(token)?;
    Ok(Some(token.to_owned()))
}

/// Return the existing token or create one without replacing an existing file.
pub fn ensure(path: &Path) -> Result<String> {
    if let Some(token) = load(path)? {
        return Ok(token);
    }
    let parent = prepare_parent(path)?;
    cleanup_stale_temps(parent, path)?;
    let token = generate();
    let temporary = temporary_path(path, "ensure");
    let mut staged = TemporaryFile::new(temporary, parent);
    write_new(staged.path(), &token).with_context(|| "create staged MCP token")?;
    let publication = fs::hard_link(staged.path(), path);
    match publication {
        Ok(()) => {
            sync_directory(parent)?;
            staged.remove()?;
            Ok(token)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            staged.remove()?;
            load(path)?.ok_or_else(|| {
                anyhow::anyhow!("MCP token file disappeared during concurrent creation")
            })
        }
        Err(error) => {
            staged.remove()?;
            Err(error).with_context(|| "publish MCP token file")
        }
    }
}

/// Atomically replace a regular token file with a newly generated token.
pub fn rotate(path: &Path) -> Result<String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        validate_regular_path(&metadata)?;
    }
    let parent = prepare_parent(path)?;
    cleanup_stale_temps(parent, path)?;

    let token = generate();
    let temporary = temporary_path(path, "rotate");
    let mut staged = TemporaryFile::new(temporary, parent);
    write_new(staged.path(), &token).with_context(|| "create replacement MCP token")?;

    let result: Result<()> = (|| {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            validate_regular_path(&metadata)?;
        }
        fs::rename(staged.path(), path).with_context(|| "publish replacement MCP token")?;
        staged.disarm();
        sync_directory(parent)?;
        Ok(())
    })();
    result?;
    Ok(token)
}

fn validate_metadata(metadata: &fs::Metadata) -> Result<()> {
    validate_regular_path(metadata)?;
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!("MCP token file permissions must not grant group or other access");
    }
    Ok(())
}

fn validate_regular_path(metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("MCP token path must be a regular non-symlink file");
    }
    Ok(())
}

fn validate_token(token: &str) -> Result<()> {
    if token.len() != 43
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("MCP token file contains invalid data");
    }
    Ok(())
}

fn strip_one_line_ending(raw: &[u8]) -> Result<&str> {
    let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    std::str::from_utf8(raw).context("MCP token file is not UTF-8")
}

fn prepare_parent(path: &Path) -> Result<&Path> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("MCP token path has no parent directory"))?;
    let created = match fs::symlink_metadata(parent) {
        Ok(_) => false,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(parent).with_context(|| "create MCP token parent directory")?;
            true
        }
        Err(error) => return Err(error).with_context(|| "inspect MCP token parent directory"),
    };
    #[cfg(unix)]
    if created {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .with_context(|| "protect new MCP token parent directory")?;
    }
    let metadata =
        fs::symlink_metadata(parent).with_context(|| "inspect MCP token parent directory")?;
    validate_parent_metadata(&metadata)?;
    Ok(parent)
}

fn validate_parent_if_present(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("MCP token path has no parent directory"))?;
    match fs::symlink_metadata(parent) {
        Ok(metadata) => validate_parent_metadata(&metadata),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| "inspect MCP token parent directory"),
    }
}

fn validate_parent_metadata(metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("MCP token parent must be a regular non-symlink directory");
    }
    #[cfg(unix)]
    {
        if metadata.uid() != unsafe { libc::geteuid() } {
            bail!("MCP token parent must be owned by the current user");
        }
        if metadata.permissions().mode() & 0o022 != 0 {
            bail!("MCP token parent must not be writable by group or other users");
        }
    }
    Ok(())
}

fn generate() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn write_new(path: &Path, token: &str) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    writeln!(file, "{token}")?;
    file.sync_all()?;
    Ok(())
}

fn temporary_path(path: &Path, operation: &str) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mcp-token");
    path.with_file_name(format!(".{name}.{operation}-{}", Uuid::new_v4()))
}

struct TemporaryFile<'a> {
    path: PathBuf,
    parent: &'a Path,
    active: bool,
}

impl<'a> TemporaryFile<'a> {
    fn new(path: PathBuf, parent: &'a Path) -> Self {
        Self {
            path,
            parent,
            active: true,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn remove(&mut self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| "remove staged MCP token"),
        }
        self.active = false;
        sync_directory(self.parent)
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for TemporaryFile<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = fs::remove_file(&self.path);
            let _ = sync_directory(self.parent);
        }
    }
}

fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| "sync MCP token directory")
}

fn cleanup_stale_temps(parent: &Path, token_path: &Path) -> Result<()> {
    let token_name = token_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mcp-token");
    let prefixes = [
        format!(".{token_name}.ensure-"),
        format!(".{token_name}.rotate-"),
    ];
    let mut removed = false;
    for entry in fs::read_dir(parent).with_context(|| "scan MCP token directory")? {
        let entry = entry.with_context(|| "inspect MCP token directory entry")?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(suffix) = prefixes.iter().find_map(|prefix| name.strip_prefix(prefix)) else {
            continue;
        };
        if Uuid::parse_str(suffix).is_err() {
            continue;
        }
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            // Another concurrent ensure/rotate may remove its temporary entry
            // after read_dir returned it. That is a successful cleanup race.
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| "inspect staged MCP token file");
            }
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        #[cfg(unix)]
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            continue;
        }
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= Duration::from_secs(60 * 60));
        if stale {
            match fs::remove_file(entry.path()) {
                Ok(()) => removed = true,
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| "remove stale MCP token temp file");
                }
            }
        }
    }
    if removed {
        sync_directory(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn ensure_generates_once_and_reload_is_stable() {
        let root = tempdir().unwrap();
        let path = root.path().join("mcp-token");
        let first = ensure(&path).unwrap();
        let second = ensure(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn rotate_changes_the_token() {
        let root = tempdir().unwrap();
        let path = root.path().join("mcp-token");
        let first = ensure(&path).unwrap();
        let second = rotate(&path).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn load_rejects_unsafe_token_content() {
        let root = tempdir().unwrap();
        let path = root.path().join("mcp-token");
        fs::write(&path, "not a generated bearer token\n").unwrap();
        assert!(load(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn generated_file_has_private_permissions() {
        let root = tempdir().unwrap();
        let path = root.path().join("mcp-token");
        ensure(&path).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_a_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let target = root.path().join("target");
        fs::write(&target, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n").unwrap();
        let path = root.path().join("mcp-token");
        symlink(target, &path).unwrap();
        assert!(load(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_group_readable_permissions() {
        let root = tempdir().unwrap();
        let path = root.path().join("mcp-token");
        fs::write(&path, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_does_not_chmod_an_existing_private_parent() {
        let root = tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o750)).unwrap();
        ensure(&root.path().join("mcp-token")).unwrap();
        assert_eq!(
            fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }

    #[cfg(unix)]
    #[test]
    fn rotate_recovers_a_corrupt_group_readable_regular_file() {
        let root = tempdir().unwrap();
        let path = root.path().join("mcp-token");
        fs::write(&path, "corrupt\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let token = rotate(&path).unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some(token.as_str()));
    }

    #[cfg(unix)]
    #[test]
    fn rotate_refuses_symlink_without_changing_link_or_target() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let target = root.path().join("target");
        let original = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n";
        fs::write(&target, original).unwrap();
        let path = root.path().join("mcp-token");
        symlink(&target, &path).unwrap();
        assert!(rotate(&path).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), original);
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_removes_only_stale_owned_uuid_temp_files() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let root = tempdir().unwrap();
        let stale = root
            .path()
            .join(format!(".mcp-token.ensure-{}", Uuid::new_v4()));
        fs::write(&stale, "partial").unwrap();
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o600)).unwrap();
        let path = CString::new(stale.as_os_str().as_bytes()).unwrap();
        let old = libc::timespec {
            tv_sec: chrono::Utc::now().timestamp() - 7200,
            tv_nsec: 0,
        };
        let times = [old, old];
        assert_eq!(
            unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), times.as_ptr(), 0) },
            0
        );

        ensure(&root.path().join("mcp-token")).unwrap();
        assert!(!stale.exists());
    }

    #[test]
    fn temporary_guard_removes_partial_file_on_error_path() {
        let root = tempdir().unwrap();
        let path = root
            .path()
            .join(format!(".mcp-token.ensure-{}", Uuid::new_v4()));
        {
            let _guard = TemporaryFile::new(path.clone(), root.path());
            fs::write(&path, "partial").unwrap();
        }
        assert!(!path.exists());
    }

    #[test]
    fn successful_ensure_leaves_no_staged_files() {
        let root = tempdir().unwrap();
        ensure(&root.path().join("mcp-token")).unwrap();
        let staged = fs::read_dir(root.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mcp-token.")
            });
        assert!(!staged);
    }

    #[test]
    fn concurrent_ensure_returns_one_stable_token() {
        for _ in 0..32 {
            let root = tempdir().unwrap();
            let path = Arc::new(root.path().join("mcp-token"));
            let barrier = Arc::new(Barrier::new(17));
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    let path = Arc::clone(&path);
                    let barrier = Arc::clone(&barrier);
                    thread::spawn(move || {
                        barrier.wait();
                        ensure(&path).unwrap()
                    })
                })
                .collect();
            barrier.wait();
            let tokens: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            assert!(tokens.windows(2).all(|pair| pair[0] == pair[1]));
        }
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_stale_cleanup_tolerates_entries_removed_after_read_dir() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        for _ in 0..32 {
            let root = tempdir().unwrap();
            let token = Arc::new(root.path().join("mcp-token"));
            for _ in 0..32 {
                let stale = root
                    .path()
                    .join(format!(".mcp-token.ensure-{}", Uuid::new_v4()));
                fs::write(&stale, b"partial").unwrap();
                fs::set_permissions(&stale, fs::Permissions::from_mode(0o600)).unwrap();
                let stale = CString::new(stale.as_os_str().as_bytes()).unwrap();
                let old = libc::timespec {
                    tv_sec: chrono::Utc::now().timestamp() - 7200,
                    tv_nsec: 0,
                };
                assert_eq!(
                    unsafe {
                        libc::utimensat(libc::AT_FDCWD, stale.as_ptr(), [old, old].as_ptr(), 0)
                    },
                    0
                );
            }
            let barrier = Arc::new(Barrier::new(17));
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    let token = Arc::clone(&token);
                    let barrier = Arc::clone(&barrier);
                    thread::spawn(move || {
                        barrier.wait();
                        cleanup_stale_temps(token.parent().unwrap(), &token)
                    })
                })
                .collect();
            barrier.wait();
            for handle in handles {
                handle.join().unwrap().unwrap();
            }
        }
    }
}
