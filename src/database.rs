use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

const SCHEMA: &str = r#"
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS admin (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1), password_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY, created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS passkeys (
  id TEXT PRIMARY KEY, name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 100),
  credential_id TEXT NOT NULL UNIQUE, credential_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS speakers (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS profiles (
  id TEXT PRIMARY KEY, speaker_id TEXT NOT NULL REFERENCES speakers(id),
  style_name TEXT NOT NULL, prompt_text TEXT NOT NULL, audio_name TEXT NOT NULL UNIQUE,
  duration_seconds REAL NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS generations (
  id TEXT PRIMARY KEY, speaker_id TEXT NOT NULL REFERENCES speakers(id),
  profile_id TEXT NOT NULL REFERENCES profiles(id), target_text TEXT NOT NULL,
  speed REAL NOT NULL, audio_name TEXT, status TEXT NOT NULL, created_at INTEGER NOT NULL
);
"#;

#[derive(Debug)]
pub struct Database {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self {
            path,
            conn: Mutex::new(conn),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))
    }

    pub fn configured(&self) -> Result<bool> {
        let conn = self.lock()?;
        let row: Option<i64> = conn
            .query_row("SELECT singleton FROM admin WHERE singleton=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(row.is_some())
    }

    pub fn set_admin(&self, password_hash: &str) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "INSERT OR IGNORE INTO admin(singleton,password_hash) VALUES(1,?1)",
            params![password_hash],
        )?;
        Ok(n == 1)
    }

    pub fn admin_password_hash(&self) -> Result<Option<String>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT password_hash FROM admin WHERE singleton=1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(row)
    }

    pub fn delete_admin_hash(&self, password_hash: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM admin WHERE singleton=1 AND password_hash=?1",
            params![password_hash],
        )?;
        Ok(())
    }

    /// Changes the existing administrator password and deletes all web sessions
    /// in one transaction.
    ///
    /// Returns an error when no administrator is configured. This operation
    /// preserves passkeys, speakers, profiles, generations, and files.
    pub fn change_admin_password_and_clear_sessions(&self, password_hash: &str) -> Result<()> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction()?;
        let updated = transaction.execute(
            "UPDATE admin SET password_hash=?1 WHERE singleton=1",
            params![password_hash],
        )?;
        if updated != 1 {
            bail!("administrator is not configured");
        }
        transaction.execute("DELETE FROM sessions", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn create_session(&self, digest: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO sessions(token_hash,created_at) VALUES(?1,?2)",
            params![digest, now_secs()],
        )?;
        Ok(())
    }

    pub fn session_exists(&self, digest: &str) -> Result<bool> {
        let conn = self.lock()?;
        let row: Option<String> = conn
            .query_row(
                "SELECT token_hash FROM sessions WHERE token_hash=?1",
                params![digest],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.is_some())
    }

    pub fn delete_session(&self, digest: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM sessions WHERE token_hash=?1", params![digest])?;
        Ok(())
    }

    pub fn count_passkeys(&self) -> Result<u64> {
        let conn = self.lock()?;
        let count = conn.query_row("SELECT COUNT(*) FROM passkeys", [], |r| r.get(0))?;
        Ok(count)
    }

    pub fn list_passkeys(&self) -> Result<Vec<PasskeyRow>> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT id,name,created_at FROM passkeys ORDER BY created_at,name")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PasskeyRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn load_passkeys(&self) -> Result<Vec<StoredPasskeyRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id,name,credential_id,credential_json,created_at FROM passkeys \
             ORDER BY created_at,name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(StoredPasskeyRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    credential_id: r.get(2)?,
                    credential_json: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_passkey(
        &self,
        id: &str,
        name: &str,
        credential_id: &str,
        credential_json: &str,
    ) -> Result<Option<PasskeyRow>> {
        let created_at = now_secs();
        let conn = self.lock()?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO passkeys(id,name,credential_id,credential_json,created_at) \
             VALUES(?1,?2,?3,?4,?5)",
            params![id, name, credential_id, credential_json, created_at],
        )?;
        Ok((inserted == 1).then(|| PasskeyRow {
            id: id.to_string(),
            name: name.to_string(),
            created_at,
        }))
    }

    pub fn update_passkey(&self, id: &str, credential_json: &str) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE passkeys SET credential_json=?1 WHERE id=?2",
            params![credential_json, id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_passkey(&self, id: &str) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute("DELETE FROM passkeys WHERE id=?1", params![id])?;
        Ok(n > 0)
    }

    pub fn list_speakers(&self) -> Result<Vec<SpeakerRow>> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT id,name,created_at FROM speakers ORDER BY created_at,name")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SpeakerRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_profiles(&self, speaker_id: &str) -> Result<Vec<ProfileRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id,style_name,prompt_text,duration_seconds,created_at,audio_name,speaker_id \
             FROM profiles WHERE speaker_id=?1 ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![speaker_id], |r| {
                Ok(ProfileRow {
                    id: r.get(0)?,
                    style_name: r.get(1)?,
                    prompt_text: r.get(2)?,
                    duration_seconds: r.get(3)?,
                    created_at: r.get(4)?,
                    audio_name: r.get(5)?,
                    speaker_id: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_speaker(&self, id: &str, name: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO speakers(id,name,created_at) VALUES(?1,?2,?3)",
            params![id, name, now_secs()],
        )?;
        Ok(())
    }

    pub fn speaker_by_id(&self, id: &str) -> Result<Option<SpeakerRow>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id,name,created_at FROM speakers WHERE id=?1",
            params![id],
            |r| {
                Ok(SpeakerRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn speaker_by_name(&self, name: &str) -> Result<Option<SpeakerRow>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id,name,created_at FROM speakers WHERE name=?1",
            params![name],
            |r| {
                Ok(SpeakerRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn speaker_has_profiles(&self, speaker_id: &str) -> Result<bool> {
        let conn = self.lock()?;
        let row: Option<String> = conn
            .query_row(
                "SELECT id FROM profiles WHERE speaker_id=?1 LIMIT 1",
                params![speaker_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.is_some())
    }

    pub fn delete_speaker(&self, speaker_id: &str) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute("DELETE FROM speakers WHERE id=?1", params![speaker_id])?;
        Ok(n > 0)
    }

    pub fn rename_speaker(&self, speaker_id: &str, name: &str) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE speakers SET name=?1 WHERE id=?2",
            params![name, speaker_id],
        )?;
        Ok(n > 0)
    }

    pub fn insert_profile(
        &self,
        id: &str,
        speaker_id: &str,
        style_name: &str,
        prompt_text: &str,
        audio_name: &str,
        duration_seconds: f64,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO profiles(id,speaker_id,style_name,prompt_text,audio_name,duration_seconds,created_at) \
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                speaker_id,
                style_name,
                prompt_text,
                audio_name,
                duration_seconds,
                now_secs()
            ],
        )?;
        Ok(())
    }

    pub fn profile_by_id(&self, id: &str) -> Result<Option<ProfileRow>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id,style_name,prompt_text,duration_seconds,created_at,audio_name,speaker_id \
             FROM profiles WHERE id=?1",
            params![id],
            |r| {
                Ok(ProfileRow {
                    id: r.get(0)?,
                    style_name: r.get(1)?,
                    prompt_text: r.get(2)?,
                    duration_seconds: r.get(3)?,
                    created_at: r.get(4)?,
                    audio_name: r.get(5)?,
                    speaker_id: r.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn profile_for_speaker(
        &self,
        profile_id: &str,
        speaker_id: &str,
    ) -> Result<Option<ProfileRow>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id,style_name,prompt_text,duration_seconds,created_at,audio_name,speaker_id \
             FROM profiles WHERE id=?1 AND speaker_id=?2",
            params![profile_id, speaker_id],
            |r| {
                Ok(ProfileRow {
                    id: r.get(0)?,
                    style_name: r.get(1)?,
                    prompt_text: r.get(2)?,
                    duration_seconds: r.get(3)?,
                    created_at: r.get(4)?,
                    audio_name: r.get(5)?,
                    speaker_id: r.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn profile_exists_style(&self, speaker_id: &str, style_name: &str) -> Result<bool> {
        let conn = self.lock()?;
        let row: Option<String> = conn
            .query_row(
                "SELECT id FROM profiles WHERE speaker_id=?1 AND style_name=?2",
                params![speaker_id, style_name],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.is_some())
    }

    pub fn profile_in_use(&self, profile_id: &str) -> Result<bool> {
        let conn = self.lock()?;
        let row: Option<String> = conn
            .query_row(
                "SELECT id FROM generations WHERE profile_id=?1 LIMIT 1",
                params![profile_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.is_some())
    }

    pub fn delete_profile(&self, profile_id: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM profiles WHERE id=?1", params![profile_id])?;
        Ok(())
    }

    pub fn rename_profile_style(&self, profile_id: &str, style_name: &str) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE profiles SET style_name=?1 WHERE id=?2",
            params![style_name, profile_id],
        )?;
        Ok(n > 0)
    }

    pub fn insert_generation_running(
        &self,
        id: &str,
        speaker_id: &str,
        profile_id: &str,
        target_text: &str,
        speed: f64,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO generations(id,speaker_id,profile_id,target_text,speed,status,created_at) \
             VALUES(?1,?2,?3,?4,?5,'running',?6)",
            params![id, speaker_id, profile_id, target_text, speed, now_secs()],
        )?;
        Ok(())
    }

    pub fn complete_generation(&self, id: &str, audio_name: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE generations SET status='complete',audio_name=?1 WHERE id=?2",
            params![audio_name, id],
        )?;
        Ok(())
    }

    pub fn fail_generation(&self, id: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE generations SET status='failed' WHERE id=?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn generation_by_id(&self, id: &str) -> Result<Option<GenerationRow>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id,status,audio_name,target_text,speed,created_at,speaker_id,profile_id \
             FROM generations WHERE id=?1",
            params![id],
            |r| {
                Ok(GenerationRow {
                    id: r.get(0)?,
                    status: r.get(1)?,
                    audio_name: r.get(2)?,
                    target_text: r.get(3)?,
                    speed: r.get(4)?,
                    created_at: r.get(5)?,
                    speaker_id: r.get(6)?,
                    profile_id: r.get(7)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn complete_generation_audio(&self, id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT audio_name FROM generations WHERE id=?1 AND status='complete'",
            params![id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|o| o.flatten())
        .map_err(Into::into)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeakerRow {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PasskeyRow {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct StoredPasskeyRow {
    pub id: String,
    pub name: String,
    pub credential_id: String,
    pub credential_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProfileRow {
    pub id: String,
    pub style_name: String,
    pub prompt_text: String,
    pub duration_seconds: f64,
    pub created_at: i64,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub audio_name: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub speaker_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GenerationRow {
    pub id: String,
    pub status: String,
    pub audio_name: Option<String>,
    pub target_text: String,
    pub speed: f64,
    pub created_at: i64,
    pub speaker_id: String,
    pub profile_id: String,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn admin_and_session() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("t.sqlite3")).unwrap();
        assert!(!db.configured().unwrap());
        assert!(db.set_admin("hash").unwrap());
        assert!(db.configured().unwrap());
        assert!(!db.set_admin("other").unwrap());
        db.create_session("abc").unwrap();
        assert!(db.session_exists("abc").unwrap());
    }

    #[test]
    fn password_change_updates_hash_and_clears_all_sessions() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("t.sqlite3")).unwrap();
        assert!(db.set_admin("old-hash").unwrap());
        db.create_session("session-one").unwrap();
        db.create_session("session-two").unwrap();

        db.change_admin_password_and_clear_sessions("new-hash")
            .unwrap();

        assert_eq!(
            db.admin_password_hash().unwrap().as_deref(),
            Some("new-hash")
        );
        assert!(!db.session_exists("session-one").unwrap());
        assert!(!db.session_exists("session-two").unwrap());
    }

    #[test]
    fn password_change_rejects_an_unconfigured_database() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("t.sqlite3")).unwrap();

        let error = db
            .change_admin_password_and_clear_sessions("new-hash")
            .unwrap_err();

        assert!(error.to_string().contains("not configured"));
        assert!(!db.configured().unwrap());
    }

    #[test]
    fn password_change_preserves_passkeys() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("t.sqlite3")).unwrap();
        assert!(db.set_admin("old-hash").unwrap());
        db.insert_passkey("one", "Laptop", "credential", "{\"v\":1}")
            .unwrap()
            .unwrap();

        db.change_admin_password_and_clear_sessions("new-hash")
            .unwrap();

        assert_eq!(db.count_passkeys().unwrap(), 1);
    }

    #[test]
    fn password_change_rolls_back_when_session_deletion_fails() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("t.sqlite3")).unwrap();
        assert!(db.set_admin("old-hash").unwrap());
        db.create_session("session-one").unwrap();
        db.lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_session_delete BEFORE DELETE ON sessions \
                 BEGIN SELECT RAISE(FAIL, 'injected failure'); END;",
            )
            .unwrap();

        assert!(db
            .change_admin_password_and_clear_sessions("new-hash")
            .is_err());

        assert_eq!(
            db.admin_password_hash().unwrap().as_deref(),
            Some("old-hash")
        );
        assert!(db.session_exists("session-one").unwrap());
    }

    #[test]
    fn password_change_preserves_voice_and_generation_records() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("t.sqlite3")).unwrap();
        assert!(db.set_admin("old-hash").unwrap());
        db.insert_speaker("speaker-one", "Speaker").unwrap();
        db.insert_profile(
            "profile-one",
            "speaker-one",
            "Neutral",
            "Exact transcript",
            "voice.wav",
            8.0,
        )
        .unwrap();
        db.insert_generation_running(
            "generation-one",
            "speaker-one",
            "profile-one",
            "Target text",
            1.0,
        )
        .unwrap();

        db.change_admin_password_and_clear_sessions("new-hash")
            .unwrap();

        assert!(db.speaker_by_id("speaker-one").unwrap().is_some());
        assert!(db.profile_by_id("profile-one").unwrap().is_some());
        assert!(db.generation_by_id("generation-one").unwrap().is_some());
    }

    #[test]
    fn passkey_crud_and_duplicate_credential_id() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("t.sqlite3")).unwrap();
        let row = db
            .insert_passkey("one", "Laptop", "credential", "{\"v\":1}")
            .unwrap()
            .unwrap();
        assert_eq!(row.name, "Laptop");
        assert_eq!(db.count_passkeys().unwrap(), 1);
        assert_eq!(db.list_passkeys().unwrap().len(), 1);
        assert_eq!(db.load_passkeys().unwrap()[0].credential_json, "{\"v\":1}");
        assert!(db.update_passkey("one", "{\"v\":2}").unwrap());
        assert!(db
            .insert_passkey("two", "Phone", "credential", "{\"v\":1}")
            .unwrap()
            .is_none());
        assert!(db.delete_passkey("one").unwrap());
        assert_eq!(db.count_passkeys().unwrap(), 0);
    }
}
