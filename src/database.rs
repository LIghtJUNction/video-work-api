use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

const SCHEMA: &str = r#"
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS admin (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1), password_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY, created_at INTEGER NOT NULL
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
            .query_row(
                "SELECT singleton FROM admin WHERE singleton=1",
                [],
                |r| r.get(0),
            )
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
        conn.execute(
            "DELETE FROM sessions WHERE token_hash=?1",
            params![digest],
        )?;
        Ok(())
    }

    pub fn list_speakers(&self) -> Result<Vec<SpeakerRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id,name,created_at FROM speakers ORDER BY created_at,name",
        )?;
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
        let n = conn.execute(
            "DELETE FROM speakers WHERE id=?1",
            params![speaker_id],
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
        conn.execute(
            "DELETE FROM profiles WHERE id=?1",
            params![profile_id],
        )?;
        Ok(())
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
}
