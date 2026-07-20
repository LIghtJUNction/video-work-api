use std::fs;
use std::path::{Path, PathBuf};

/// Resolve `raw_path` if it is a regular file inside `root` (or a subdirectory).
pub fn resolve_under_root(raw_path: &str, root: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let candidate = {
        let p = PathBuf::from(raw_path);
        if p.is_absolute() {
            p
        } else {
            root.join(p)
        }
    };
    let resolved = candidate.canonicalize().ok()?;
    if resolved.parent()? != root.as_path() && !resolved.starts_with(&root) {
        return None;
    }
    if !resolved.is_file() {
        return None;
    }
    // Reject symlinks for the final path component.
    let meta = fs::symlink_metadata(&resolved).ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    Some(resolved)
}

/// Opaque UUID-named WAV under `root` that is a regular non-symlink file.
pub fn safe_owned_file(root: &Path, name: &str) -> Option<PathBuf> {
    let stem = name.strip_suffix(".wav")?;
    if uuid::Uuid::parse_str(stem).is_err() {
        return None;
    }
    if name != format!("{stem}.wav") {
        return None;
    }
    let path = root.join(name);
    let meta = fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let resolved = path.canonicalize().ok()?;
    let root_resolved = root.canonicalize().ok()?;
    if resolved.parent()? != root_resolved.as_path() {
        return None;
    }
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn sandbox_accepts_relative_under_root() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("clip.mp4");
        fs::write(&file, b"x").unwrap();
        let got = resolve_under_root("clip.mp4", dir.path()).unwrap();
        assert_eq!(got, file.canonicalize().unwrap());
    }

    #[test]
    fn sandbox_rejects_escape() {
        let dir = tempdir().unwrap();
        assert!(resolve_under_root("../etc/passwd", dir.path()).is_none());
    }

    #[test]
    fn owned_wav_requires_uuid_name() {
        let dir = tempdir().unwrap();
        let id = uuid::Uuid::new_v4();
        let name = format!("{id}.wav");
        let path = dir.path().join(&name);
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(b"RIFF").unwrap();
        assert!(safe_owned_file(dir.path(), &name).is_some());
        assert!(safe_owned_file(dir.path(), "not-a-uuid.wav").is_none());
    }
}
