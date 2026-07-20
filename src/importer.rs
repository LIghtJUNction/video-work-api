use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use uuid::Uuid;

use crate::audio::{self, convert_reference};
use crate::config::Settings;
use crate::database::Database;

/// Import `speaker/style.{audio,txt}` pairs from a folder tree.
pub fn import_folder(root: &Path, settings: &Settings, database: &Database) -> Result<usize> {
    let meta = fs::symlink_metadata(root).context("stat import root")?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        bail!("Import root must be a regular directory");
    }

    let mut pairs: Vec<(String, String, std::path::PathBuf, String)> = Vec::new();
    let mut speakers: Vec<_> = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    speakers.sort_by_key(|e| e.file_name());

    for speaker_entry in speakers {
        let speaker_dir = speaker_entry.path();
        let info = fs::symlink_metadata(&speaker_dir)?;
        if info.file_type().is_symlink() {
            bail!("Symlinks are not accepted");
        }
        if !info.is_dir() {
            bail!("Import root may contain only speaker directories");
        }
        let speaker_name = speaker_entry.file_name().to_string_lossy().into_owned();
        let mut styles: std::collections::BTreeMap<String, std::path::PathBuf> =
            std::collections::BTreeMap::new();
        let mut entries: Vec<_> = fs::read_dir(&speaker_dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            audio::require_regular(&path)?;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "txt" {
                continue;
            }
            if !audio::extension_allowed(&path) {
                bail!("Unsupported file: {}", path.file_name().unwrap().to_string_lossy());
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if styles.contains_key(&stem) {
                bail!("Ambiguous style: {stem}");
            }
            styles.insert(stem, path);
        }
        for (style, audio_path) in styles {
            let transcript_path = speaker_dir.join(format!("{style}.txt"));
            audio::require_regular(&transcript_path)?;
            let transcript = fs::read_to_string(&transcript_path)?
                .trim()
                .to_string();
            if transcript.is_empty() {
                bail!(
                    "Empty transcript: {}",
                    transcript_path.file_name().unwrap().to_string_lossy()
                );
            }
            pairs.push((speaker_name.clone(), style, audio_path, transcript));
        }
    }

    if pairs.is_empty() {
        bail!("No audio/transcript pairs found");
    }

    let mut imported = 0usize;
    for (speaker_name, style, source, transcript) in pairs {
        let speaker_id = if let Some(s) = database.speaker_by_name(&speaker_name)? {
            s.id
        } else {
            let id = Uuid::new_v4().to_string();
            database.insert_speaker(&id, &speaker_name)?;
            id
        };
        if database.profile_exists_style(&speaker_id, &style)? {
            bail!("Profile already exists: {speaker_name}/{style}");
        }
        let profile_id = Uuid::new_v4().to_string();
        let audio_name = format!("{}.wav", Uuid::new_v4());
        let destination = settings.profiles_dir().join(&audio_name);
        let duration = convert_reference(&source, &destination)?;
        if let Err(e) = database.insert_profile(
            &profile_id,
            &speaker_id,
            &style,
            &transcript,
            &audio_name,
            duration,
        ) {
            let _ = fs::remove_file(&destination);
            return Err(e);
        }
        imported += 1;
    }
    Ok(imported)
}
