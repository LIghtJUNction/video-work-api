use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

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
CREATE TABLE IF NOT EXISTS render_jobs (
  id TEXT PRIMARY KEY,
  render_key TEXT NOT NULL,
  task_dir TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  encoder_profile TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed','canceled')),
  log_path TEXT NOT NULL,
  enqueue_seq INTEGER NOT NULL,
  snapshot_dir TEXT NOT NULL,
  snapshot_hash TEXT NOT NULL,
  renderer_hash TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'legacy_xry',
  project_id TEXT,
  project_revision INTEGER,
  document_sha TEXT,
  output_dir TEXT,
  render_plan TEXT,
  attestation_json TEXT,
  publication_intent TEXT,
  recovery_intent TEXT,
  recovery_blocked TEXT,
  cleanup_pending INTEGER NOT NULL DEFAULT 0,
  cancel_requested INTEGER NOT NULL DEFAULT 0,
  pid INTEGER,
  pid_starttime INTEGER,
  exit_code INTEGER,
  error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  started_at INTEGER,
  finished_at INTEGER,
  CHECK(NOT (status='running' AND publication_intent IS NOT NULL AND cancel_requested=1))
);
CREATE TABLE IF NOT EXISTS video_projects (
  id TEXT PRIMARY KEY, current_revision INTEGER NOT NULL DEFAULT 0,
  validated_revision INTEGER,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS video_project_revisions (
  project_id TEXT NOT NULL REFERENCES video_projects(id),
  revision INTEGER NOT NULL, sha256 TEXT NOT NULL, created_at INTEGER NOT NULL,
  PRIMARY KEY(project_id,revision)
);
CREATE TABLE IF NOT EXISTS pending_video_project_writes (
  project_id TEXT PRIMARY KEY REFERENCES video_projects(id),
  expected_revision INTEGER NOT NULL,
  new_revision INTEGER NOT NULL,
  old_sha TEXT NOT NULL,
  new_sha TEXT NOT NULL,
  staged_path TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS variant_id_counters (
  namespace TEXT PRIMARY KEY, next_id INTEGER NOT NULL CHECK(next_id > 0)
);
CREATE UNIQUE INDEX IF NOT EXISTS render_jobs_single_running
  ON render_jobs((1))
  WHERE status='running';
CREATE UNIQUE INDEX IF NOT EXISTS render_jobs_deduplicated_key
  ON render_jobs(render_key)
  WHERE status IN ('queued','running','succeeded');
"#;

#[derive(Debug)]
pub struct Database {
    path: PathBuf,
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationIntentOutcome {
    Entered,
    CancelWon,
    Stale,
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
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        if !table_has_column(&conn, "video_projects", "validated_revision")? {
            conn.execute(
                "ALTER TABLE video_projects ADD COLUMN validated_revision INTEGER",
                [],
            )?;
        }
        migrate_render_jobs(&conn)?;
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

    pub fn insert_or_get_render_job(&self, job: NewRenderJob<'_>) -> Result<(RenderJobRow, bool)> {
        let now = now_secs();
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let enqueue_seq: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(enqueue_seq),0)+1 FROM render_jobs",
            [],
            |row| row.get(0),
        )?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO render_jobs(
               id,render_key,task_dir,subject_id,encoder_profile,status,log_path,enqueue_seq,
               snapshot_dir,snapshot_hash,renderer_hash,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,?5,'queued',?6,?7,?8,?9,?10,?11,?11)",
            params![
                job.id,
                job.render_key,
                job.task_dir,
                job.subject_id,
                job.encoder_profile,
                job.log_path,
                enqueue_seq,
                job.snapshot_dir,
                job.snapshot_hash,
                job.renderer_hash,
                now
            ],
        )?;
        let row = query_active_render_job_by_key(&transaction, job.render_key)?
            .ok_or_else(|| anyhow::anyhow!("render job insert disappeared"))?;
        transaction.commit()?;
        Ok((row, inserted == 1))
    }

    pub fn insert_or_get_video_project_render_job(
        &self,
        job: NewVideoProjectRenderJob<'_>,
    ) -> Result<(RenderJobRow, bool)> {
        let now = now_secs();
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let enqueue_seq: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(enqueue_seq),0)+1 FROM render_jobs",
            [],
            |row| row.get(0),
        )?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO render_jobs(
               id,render_key,task_dir,subject_id,encoder_profile,status,log_path,enqueue_seq,
               snapshot_dir,snapshot_hash,renderer_hash,kind,project_id,project_revision,
               document_sha,output_dir,render_plan,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,'video-project','queued',?5,?6,?7,?8,?9,
                      'video_project',?10,?11,?12,?13,?14,?15,?15)",
            params![
                job.id,
                job.render_key,
                job.output_dir,
                job.project_id,
                job.log_path,
                enqueue_seq,
                job.snapshot_dir,
                job.snapshot_hash,
                job.renderer_hash,
                job.project_id,
                job.project_revision,
                job.document_sha,
                job.output_dir,
                job.render_plan,
                now,
            ],
        )?;
        let row = query_active_render_job_by_key(&transaction, job.render_key)?
            .ok_or_else(|| anyhow::anyhow!("video project render job insert disappeared"))?;
        transaction.commit()?;
        Ok((row, inserted == 1))
    }

    pub fn insert_or_get_media_job(&self, job: NewMediaJob<'_>) -> Result<(RenderJobRow, bool)> {
        if !matches!(job.kind, "analysis_frames" | "safe_trims" | "cover") {
            bail!("unsupported media job kind");
        }
        let now = now_secs();
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let enqueue_seq: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(enqueue_seq),0)+1 FROM render_jobs",
            [],
            |row| row.get(0),
        )?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO render_jobs(
               id,render_key,task_dir,subject_id,encoder_profile,status,log_path,enqueue_seq,
               snapshot_dir,snapshot_hash,renderer_hash,kind,project_id,project_revision,
               document_sha,output_dir,render_plan,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,'media','queued',?5,?6,?7,?8,?9,?10,?11,?12,
                      ?13,?14,?15,?16,?16)",
            params![
                job.id,
                job.render_key,
                job.output_dir,
                job.subject,
                job.log_path,
                enqueue_seq,
                job.snapshot_dir,
                job.snapshot_hash,
                job.renderer_hash,
                job.kind,
                job.project_id,
                job.project_revision,
                job.document_sha,
                job.output_dir,
                job.request_path,
                now,
            ],
        )?;
        let row = query_active_render_job_by_key(&transaction, job.render_key)?
            .ok_or_else(|| anyhow::anyhow!("media job insert disappeared"))?;
        transaction.commit()?;
        Ok((row, inserted == 1))
    }

    pub fn render_job_by_id(&self, id: &str) -> Result<Option<RenderJobRow>> {
        let conn = self.lock()?;
        query_render_job(&conn, "id", id)
    }

    pub(crate) fn running_render_jobs(&self) -> Result<Vec<RenderJobRow>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT id,render_key,task_dir,subject_id,encoder_profile,status,log_path,enqueue_seq,
             snapshot_dir,snapshot_hash,renderer_hash,kind,project_id,project_revision,
             document_sha,output_dir,render_plan,cancel_requested,pid,pid_starttime,
             attestation_json,publication_intent,recovery_intent,recovery_blocked,cleanup_pending,
             exit_code,error,created_at,updated_at,started_at,finished_at
             FROM render_jobs WHERE status='running' ORDER BY enqueue_seq",
        )?;
        let rows = statement
            .query_map([], map_render_job)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(crate) fn recent_render_jobs(&self) -> Result<Vec<RenderJobRow>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT id,render_key,task_dir,subject_id,encoder_profile,status,log_path,enqueue_seq,
             snapshot_dir,snapshot_hash,renderer_hash,kind,project_id,project_revision,
             document_sha,output_dir,render_plan,cancel_requested,pid,pid_starttime,
             attestation_json,publication_intent,recovery_intent,recovery_blocked,cleanup_pending,
             exit_code,error,created_at,updated_at,started_at,finished_at
             FROM render_jobs ORDER BY enqueue_seq DESC LIMIT 100",
        )?;
        let rows = statement
            .query_map([], map_render_job)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(crate) fn recover_render_job_after_cleanup(
        &self,
        id: &str,
        requeue: bool,
        error: &str,
    ) -> Result<()> {
        let now = now_secs();
        let status = if requeue { "queued" } else { "canceled" };
        let changed = self.lock()?.execute(
            "UPDATE render_jobs SET status=?1,pid=NULL,pid_starttime=NULL,
             publication_intent=NULL,recovery_intent=NULL,recovery_blocked=NULL,
             cleanup_pending=CASE WHEN ?1='queued' THEN 0 ELSE 1 END,
             started_at=CASE WHEN ?1='queued' THEN NULL ELSE started_at END,
             finished_at=CASE WHEN ?1='queued' THEN NULL ELSE ?2 END,
             error=?3,updated_at=?2 WHERE id=?4 AND status='running'",
            params![status, now, error, id],
        )?;
        if changed != 1 {
            bail!("running render job disappeared during recovery");
        }
        Ok(())
    }

    pub(crate) fn set_render_publication_intent(
        &self,
        id: &str,
        exit_code: Option<i32>,
        publication_intent: &str,
    ) -> Result<PublicationIntentOutcome> {
        let now = now_secs();
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE render_jobs SET publication_intent=?1,pid=NULL,pid_starttime=NULL,
             exit_code=?2,recovery_intent=NULL,recovery_blocked=NULL,updated_at=?3
             WHERE id=?4 AND status='running' AND publication_intent IS NULL
             AND cancel_requested=0 AND recovery_intent IS NULL
             AND recovery_blocked IS NULL",
            params![publication_intent, exit_code, now, id],
        )?;
        if changed == 1 {
            transaction.commit()?;
            return Ok(PublicationIntentOutcome::Entered);
        }
        let state = query_render_job(&transaction, "id", id)?;
        let outcome = match state {
            Some(row)
                if row.status == "running"
                    && row.publication_intent.is_none()
                    && row.cancel_requested =>
            {
                let canceled = transaction.execute(
                    "UPDATE render_jobs SET status='canceled',pid=NULL,pid_starttime=NULL,
                     attestation_json=NULL,publication_intent=NULL,
                     recovery_intent=NULL,recovery_blocked=NULL,
                     cleanup_pending=1,exit_code=?1,error='canceled before publication',
                     finished_at=?2,updated_at=?2
                     WHERE id=?3 AND status='running' AND publication_intent IS NULL
                     AND cancel_requested=1",
                    params![exit_code, now, id],
                )?;
                if canceled != 1 {
                    bail!("cancel-won publication transition changed concurrently");
                }
                PublicationIntentOutcome::CancelWon
            }
            _ => PublicationIntentOutcome::Stale,
        };
        transaction.commit()?;
        Ok(outcome)
    }

    pub(crate) fn block_render_publication(&self, id: &str, reason: &str) -> Result<()> {
        let changed = self.lock()?.execute(
            "UPDATE render_jobs SET recovery_blocked=?1,error=?1,updated_at=?2
             WHERE id=?3 AND status='running' AND publication_intent IS NOT NULL",
            params![reason, now_secs(), id],
        )?;
        if changed != 1 {
            bail!("render publication state changed while blocking");
        }
        Ok(())
    }

    pub(crate) fn complete_render_publication(
        &self,
        id: &str,
        attestation_json: Option<&str>,
    ) -> Result<()> {
        let now = now_secs();
        let changed = self.lock()?.execute(
            "UPDATE render_jobs SET status='succeeded',pid=NULL,pid_starttime=NULL,
             attestation_json=?1,publication_intent=NULL,recovery_intent=NULL,
             recovery_blocked=NULL,cleanup_pending=1,error=NULL,
             finished_at=?2,updated_at=?2
             WHERE id=?3 AND status='running' AND publication_intent IS NOT NULL
             AND cancel_requested=0",
            params![attestation_json, now, id],
        )?;
        if changed != 1 {
            bail!("render publication could not be committed");
        }
        Ok(())
    }

    pub(crate) fn begin_render_recovery(&self, id: &str) -> Result<()> {
        let changed = self.lock()?.execute(
            "UPDATE render_jobs SET recovery_intent='terminate_then_requeue',
             recovery_blocked=NULL,updated_at=?1 WHERE id=?2 AND status='running'",
            params![now_secs(), id],
        )?;
        if changed != 1 {
            bail!("running render job disappeared before recovery intent");
        }
        Ok(())
    }

    pub(crate) fn block_render_recovery(&self, id: &str, reason: &str) -> Result<()> {
        let changed = self.lock()?.execute(
            "UPDATE render_jobs SET recovery_blocked=?1,updated_at=?2
             WHERE id=?3 AND status='running'",
            params![reason, now_secs(), id],
        )?;
        if changed != 1 {
            bail!("running render job disappeared while blocking recovery");
        }
        Ok(())
    }

    pub fn claim_next_render_job(&self) -> Result<Option<RenderJobRow>> {
        let now = now_secs();
        let mut conn = self.lock()?;
        let transaction = conn.transaction()?;
        let id: Option<String> = transaction
            .query_row(
                "SELECT id FROM render_jobs
                 WHERE status='queued' AND cancel_requested=0
                 ORDER BY enqueue_seq LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            transaction.commit()?;
            return Ok(None);
        };
        let changed = transaction.execute(
            "UPDATE render_jobs SET status='running',started_at=?1,updated_at=?1
             WHERE id=?2 AND status='queued' AND cancel_requested=0",
            params![now, id],
        )?;
        transaction.commit()?;
        if changed == 0 {
            return Ok(None);
        }
        drop(conn);
        self.render_job_by_id(&id)
    }

    pub fn request_cancel_render_job(&self, id: &str) -> Result<Option<RenderJobRow>> {
        let now = now_secs();
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE render_jobs SET
               cancel_requested=1,
               status=CASE WHEN status='queued' THEN 'canceled' ELSE status END,
               cleanup_pending=CASE WHEN status='queued' THEN 1 ELSE cleanup_pending END,
             finished_at=CASE WHEN status='queued' THEN ?1 ELSE finished_at END,
               updated_at=?1
             WHERE id=?2 AND status IN ('queued','running')
             AND (status != 'running' OR publication_intent IS NULL)",
            params![now, id],
        )?;
        if changed == 0 && query_render_job(&conn, "id", id)?.is_none() {
            return Ok(None);
        }
        query_render_job(&conn, "id", id)
    }

    pub fn render_job_cancel_requested(&self, id: &str) -> Result<bool> {
        let conn = self.lock()?;
        Ok(conn.query_row(
            "SELECT cancel_requested FROM render_jobs WHERE id=?1",
            params![id],
            |row| row.get::<_, i64>(0),
        )? != 0)
    }

    pub fn set_render_job_process(&self, id: &str, pid: u32, pid_starttime: u64) -> Result<bool> {
        let changed = self.lock()?.execute(
            "UPDATE render_jobs SET pid=?1,pid_starttime=?2,updated_at=?3
             WHERE id=?4 AND status='running'",
            params![
                i64::from(pid),
                i64::try_from(pid_starttime).context("renderer starttime exceeds SQLite range")?,
                now_secs(),
                id
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn finish_render_job(
        &self,
        id: &str,
        status: &str,
        exit_code: Option<i32>,
        error: Option<&str>,
    ) -> Result<()> {
        let now = now_secs();
        let changed = self.lock()?.execute(
            "UPDATE render_jobs SET
             status=CASE WHEN cancel_requested=1 THEN 'canceled' ELSE ?1 END,
             pid=NULL,pid_starttime=NULL,exit_code=?2,
             error=CASE
               WHEN cancel_requested=1 AND ?1 != 'canceled' THEN 'canceled by request'
               ELSE ?3
             END,
             cleanup_pending=1,recovery_intent=NULL,recovery_blocked=NULL,
             finished_at=?4,updated_at=?4 WHERE id=?5 AND status='running'
             AND publication_intent IS NULL",
            params![status, exit_code, error, now, id],
        )?;
        if changed != 1 {
            bail!("render job is no longer running");
        }
        Ok(())
    }

    pub fn render_job_counts(&self) -> Result<(u64, u64)> {
        let conn = self.lock()?;
        let queued = conn.query_row(
            "SELECT COUNT(*) FROM render_jobs WHERE status='queued'",
            [],
            |row| row.get(0),
        )?;
        let running = conn.query_row(
            "SELECT COUNT(*) FROM render_jobs WHERE status='running'",
            [],
            |row| row.get(0),
        )?;
        Ok((queued, running))
    }

    pub(crate) fn cleanup_pending_render_jobs(&self) -> Result<Vec<RenderJobRow>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT id,render_key,task_dir,subject_id,encoder_profile,status,log_path,enqueue_seq,
             snapshot_dir,snapshot_hash,renderer_hash,kind,project_id,project_revision,
             document_sha,output_dir,render_plan,cancel_requested,pid,pid_starttime,
             attestation_json,publication_intent,recovery_intent,recovery_blocked,cleanup_pending,
             exit_code,error,created_at,updated_at,started_at,finished_at
             FROM render_jobs
             WHERE cleanup_pending=1 AND status IN ('succeeded','failed','canceled')
             ORDER BY enqueue_seq",
        )?;
        let rows = statement
            .query_map([], map_render_job)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(crate) fn clear_render_cleanup_pending(&self, id: &str) -> Result<()> {
        let changed = self.lock()?.execute(
            "UPDATE render_jobs SET cleanup_pending=0,updated_at=?1
             WHERE id=?2 AND cleanup_pending=1
             AND status IN ('succeeded','failed','canceled')",
            params![now_secs(), id],
        )?;
        if changed != 1 {
            bail!("terminal cleanup state changed unexpectedly");
        }
        Ok(())
    }

    pub fn allocate_variant_ids(
        &self,
        namespace: &str,
        count: u32,
        languages: &[String],
    ) -> Result<Vec<String>> {
        if count == 0 || count > 10_000 {
            bail!("count must be between 1 and 10000");
        }
        if namespace.is_empty()
            || namespace.len() > 32
            || !namespace
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '-')
        {
            bail!("namespace must use uppercase ASCII letters, digits, or hyphens");
        }
        let mut suffixes = Vec::new();
        for language in languages {
            let suffix = language.trim().to_ascii_uppercase();
            if suffix.is_empty()
                || suffix.len() > 8
                || !suffix.chars().all(|ch| ch.is_ascii_alphanumeric())
                || suffixes.contains(&suffix)
            {
                bail!("languages must be unique ASCII alphanumeric suffixes");
            }
            suffixes.push(suffix);
        }
        let mut conn = self.lock()?;
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO variant_id_counters(namespace,next_id) VALUES(?1,1)",
            params![namespace],
        )?;
        let start: i64 = transaction.query_row(
            "SELECT next_id FROM variant_id_counters WHERE namespace=?1",
            params![namespace],
            |row| row.get(0),
        )?;
        transaction.execute(
            "UPDATE variant_id_counters SET next_id=next_id+?1 WHERE namespace=?2",
            params![i64::from(count), namespace],
        )?;
        transaction.commit()?;
        let mut ids = Vec::new();
        for value in start..start + i64::from(count) {
            let base = format!("{namespace}-{value:06}");
            if suffixes.is_empty() {
                ids.push(base);
            } else {
                ids.extend(suffixes.iter().map(|suffix| format!("{base}-{suffix}")));
            }
        }
        Ok(ids)
    }

    pub fn list_video_projects(&self) -> Result<Vec<VideoProjectRow>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT p.id,p.current_revision,p.validated_revision,r.sha256,p.created_at,p.updated_at
             FROM video_projects p
             JOIN video_project_revisions r
               ON r.project_id=p.id AND r.revision=p.current_revision
             ORDER BY p.id",
        )?;
        let rows = statement
            .query_map([], map_video_project)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn video_project(&self, id: &str) -> Result<Option<VideoProjectRow>> {
        let conn = self.lock()?;
        query_video_project(&conn, id)
    }

    pub fn prepare_video_project_write(
        &self,
        id: &str,
        expected_revision: i64,
        old_sha: &str,
        new_sha: &str,
        staged_path: &str,
        create: bool,
    ) -> Result<PendingVideoProjectWrite> {
        let now = now_secs();
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(i64, Option<String>)> = transaction
            .query_row(
                "SELECT p.current_revision,r.sha256
                 FROM video_projects p
                 LEFT JOIN video_project_revisions r
                   ON r.project_id=p.id AND r.revision=p.current_revision
                 WHERE p.id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match (create, current) {
            (true, Some(_)) => bail!("video project already exists"),
            (true, None) => {
                if expected_revision != 0 || !old_sha.is_empty() {
                    bail!("new video project must start at revision zero");
                }
                transaction.execute(
                    "INSERT INTO video_projects(
                       id,current_revision,validated_revision,created_at,updated_at
                     ) VALUES(?1,0,NULL,?2,?2)",
                    params![id, now],
                )?;
            }
            (false, None) => bail!("video project was not found"),
            (false, Some((current_revision, current_sha))) => {
                if current_revision != expected_revision {
                    bail!(
                        "revision conflict: expected {expected_revision}, current revision is {current_revision}"
                    );
                }
                if current_sha.as_deref() != Some(old_sha) {
                    bail!("video project revision hash changed");
                }
            }
        }
        let new_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("video project revision overflow"))?;
        transaction.execute(
            "INSERT INTO pending_video_project_writes(
               project_id,expected_revision,new_revision,old_sha,new_sha,staged_path,created_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                expected_revision,
                new_revision,
                old_sha,
                new_sha,
                staged_path,
                now
            ],
        )?;
        transaction.commit()?;
        Ok(PendingVideoProjectWrite {
            project_id: id.to_string(),
            expected_revision,
            new_revision,
            old_sha: old_sha.to_string(),
            new_sha: new_sha.to_string(),
            staged_path: staged_path.to_string(),
            created_at: now,
        })
    }

    pub fn pending_video_project_write(
        &self,
        id: &str,
    ) -> Result<Option<PendingVideoProjectWrite>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT project_id,expected_revision,new_revision,old_sha,new_sha,staged_path,created_at
             FROM pending_video_project_writes WHERE project_id=?1",
            params![id],
            map_pending_video_project_write,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_pending_video_project_writes(&self) -> Result<Vec<PendingVideoProjectWrite>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT project_id,expected_revision,new_revision,old_sha,new_sha,staged_path,created_at
             FROM pending_video_project_writes ORDER BY project_id",
        )?;
        let rows = statement
            .query_map([], map_pending_video_project_write)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn finalize_video_project_write(
        &self,
        pending: &PendingVideoProjectWrite,
    ) -> Result<VideoProjectRow> {
        let now = now_secs();
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let durable: Option<PendingVideoProjectWrite> = transaction
            .query_row(
                "SELECT project_id,expected_revision,new_revision,old_sha,new_sha,staged_path,created_at
                 FROM pending_video_project_writes WHERE project_id=?1",
                params![pending.project_id],
                map_pending_video_project_write,
            )
            .optional()?;
        if durable.as_ref() != Some(pending) {
            bail!("video project write intent changed");
        }
        let current: Option<(i64, Option<String>)> = transaction
            .query_row(
                "SELECT p.current_revision,r.sha256
                 FROM video_projects p
                 LEFT JOIN video_project_revisions r
                   ON r.project_id=p.id AND r.revision=p.current_revision
                 WHERE p.id=?1",
                params![pending.project_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current_revision, current_sha)) = current else {
            bail!("video project was not found");
        };
        if current_revision != pending.expected_revision
            || current_sha.as_deref().unwrap_or_default() != pending.old_sha
        {
            bail!("video project changed before write finalization");
        }
        transaction.execute(
            "INSERT INTO video_project_revisions(project_id,revision,sha256,created_at)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(project_id,revision) DO UPDATE SET sha256=excluded.sha256
             WHERE video_project_revisions.sha256=excluded.sha256",
            params![
                pending.project_id,
                pending.new_revision,
                pending.new_sha,
                pending.created_at
            ],
        )?;
        let revision_sha: String = transaction.query_row(
            "SELECT sha256 FROM video_project_revisions
             WHERE project_id=?1 AND revision=?2",
            params![pending.project_id, pending.new_revision],
            |row| row.get(0),
        )?;
        if revision_sha != pending.new_sha {
            bail!("history revision hash conflicts with write intent");
        }
        let updated = transaction.execute(
            "UPDATE video_projects
             SET current_revision=?1,validated_revision=NULL,updated_at=?2
             WHERE id=?3 AND current_revision=?4",
            params![
                pending.new_revision,
                now,
                pending.project_id,
                pending.expected_revision
            ],
        )?;
        if updated != 1 {
            bail!("revision conflict while finalizing video project write");
        }
        transaction.execute(
            "DELETE FROM pending_video_project_writes
             WHERE project_id=?1 AND new_revision=?2 AND new_sha=?3",
            params![pending.project_id, pending.new_revision, pending.new_sha],
        )?;
        transaction.commit()?;
        drop(conn);
        self.video_project(&pending.project_id)?
            .ok_or_else(|| anyhow::anyhow!("updated video project disappeared"))
    }

    pub fn mark_video_project_validated(
        &self,
        id: &str,
        revision: i64,
        sha256: &str,
    ) -> Result<VideoProjectRow> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(i64, String)> = transaction
            .query_row(
                "SELECT p.current_revision,r.sha256
                 FROM video_projects p
                 JOIN video_project_revisions r
                   ON r.project_id=p.id AND r.revision=p.current_revision
                 WHERE p.id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current_revision, current_sha256)) = current else {
            bail!("video project was not found");
        };
        if current_revision != revision || current_sha256 != sha256 {
            bail!("video project changed during validation");
        }
        transaction.execute(
            "UPDATE video_projects SET validated_revision=?1 WHERE id=?2",
            params![revision, id],
        )?;
        transaction.commit()?;
        drop(conn);
        self.video_project(id)?
            .ok_or_else(|| anyhow::anyhow!("validated video project disappeared"))
    }
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names.iter().any(|name| name == column))
}

fn migrate_render_jobs(conn: &Connection) -> Result<()> {
    for (column, declaration) in [
        ("enqueue_seq", "INTEGER NOT NULL DEFAULT 0"),
        ("snapshot_dir", "TEXT NOT NULL DEFAULT ''"),
        ("snapshot_hash", "TEXT NOT NULL DEFAULT ''"),
        ("renderer_hash", "TEXT NOT NULL DEFAULT ''"),
        ("pid_starttime", "INTEGER"),
        ("kind", "TEXT NOT NULL DEFAULT 'legacy_xry'"),
        ("project_id", "TEXT"),
        ("project_revision", "INTEGER"),
        ("document_sha", "TEXT"),
        ("output_dir", "TEXT"),
        ("render_plan", "TEXT"),
        ("attestation_json", "TEXT"),
        ("publication_intent", "TEXT"),
        ("recovery_intent", "TEXT"),
        ("recovery_blocked", "TEXT"),
        ("cleanup_pending", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !table_has_column(conn, "render_jobs", column)? {
            conn.execute(
                &format!("ALTER TABLE render_jobs ADD COLUMN {column} {declaration}"),
                [],
            )?;
        }
    }
    conn.execute_batch(
        "WITH ordered AS (
           SELECT rowid, ROW_NUMBER() OVER (ORDER BY created_at,rowid) AS sequence
           FROM render_jobs
         )
         UPDATE render_jobs
         SET enqueue_seq=(SELECT sequence FROM ordered WHERE ordered.rowid=render_jobs.rowid)
         WHERE enqueue_seq=0;
         UPDATE render_jobs
         SET cancel_requested=0
         WHERE status='running' AND publication_intent IS NOT NULL
           AND cancel_requested=1;
         CREATE UNIQUE INDEX IF NOT EXISTS render_jobs_enqueue_seq
           ON render_jobs(enqueue_seq);
         CREATE INDEX IF NOT EXISTS render_jobs_status_enqueue
           ON render_jobs(status,enqueue_seq);
         CREATE TRIGGER IF NOT EXISTS render_jobs_no_cancel_during_publication_insert
         BEFORE INSERT ON render_jobs
         WHEN NEW.status='running' AND NEW.publication_intent IS NOT NULL
           AND NEW.cancel_requested=1
         BEGIN
           SELECT RAISE(ABORT, 'running publication cannot also be canceled');
         END;
         CREATE TRIGGER IF NOT EXISTS render_jobs_no_cancel_during_publication_update
         BEFORE UPDATE ON render_jobs
         WHEN NEW.status='running' AND NEW.publication_intent IS NOT NULL
           AND NEW.cancel_requested=1
         BEGIN
           SELECT RAISE(ABORT, 'running publication cannot also be canceled');
         END;",
    )?;
    Ok(())
}

fn query_video_project(conn: &Connection, id: &str) -> Result<Option<VideoProjectRow>> {
    conn.query_row(
        "SELECT p.id,p.current_revision,p.validated_revision,r.sha256,p.created_at,p.updated_at
         FROM video_projects p
         JOIN video_project_revisions r
           ON r.project_id=p.id AND r.revision=p.current_revision
         WHERE p.id=?1",
        params![id],
        map_video_project,
    )
    .optional()
    .map_err(Into::into)
}

fn map_video_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<VideoProjectRow> {
    Ok(VideoProjectRow {
        id: row.get(0)?,
        current_revision: row.get(1)?,
        validated_revision: row.get(2)?,
        current_sha256: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn map_pending_video_project_write(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PendingVideoProjectWrite> {
    Ok(PendingVideoProjectWrite {
        project_id: row.get(0)?,
        expected_revision: row.get(1)?,
        new_revision: row.get(2)?,
        old_sha: row.get(3)?,
        new_sha: row.get(4)?,
        staged_path: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn query_render_job(conn: &Connection, field: &str, value: &str) -> Result<Option<RenderJobRow>> {
    debug_assert_eq!(field, "id");
    let sql = format!(
        "SELECT id,render_key,task_dir,subject_id,encoder_profile,status,log_path,enqueue_seq,
         snapshot_dir,snapshot_hash,renderer_hash,kind,project_id,project_revision,
         document_sha,output_dir,render_plan,cancel_requested,pid,pid_starttime,
         attestation_json,publication_intent,recovery_intent,recovery_blocked,cleanup_pending,
         exit_code,error,created_at,updated_at,started_at,finished_at
         FROM render_jobs WHERE {field}=?1"
    );
    conn.query_row(&sql, params![value], map_render_job)
        .optional()
        .map_err(Into::into)
}

fn query_active_render_job_by_key(
    conn: &Connection,
    render_key: &str,
) -> Result<Option<RenderJobRow>> {
    conn.query_row(
        "SELECT id,render_key,task_dir,subject_id,encoder_profile,status,log_path,enqueue_seq,
         snapshot_dir,snapshot_hash,renderer_hash,kind,project_id,project_revision,
         document_sha,output_dir,render_plan,cancel_requested,pid,pid_starttime,
         attestation_json,publication_intent,recovery_intent,recovery_blocked,cleanup_pending,
         exit_code,error,created_at,updated_at,started_at,finished_at
         FROM render_jobs
         WHERE render_key=?1 AND status IN ('queued','running','succeeded')
         ORDER BY enqueue_seq DESC LIMIT 1",
        params![render_key],
        map_render_job,
    )
    .optional()
    .map_err(Into::into)
}

fn map_render_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<RenderJobRow> {
    Ok(RenderJobRow {
        id: row.get(0)?,
        render_key: row.get(1)?,
        task_dir: row.get(2)?,
        subject_id: row.get(3)?,
        encoder_profile: row.get(4)?,
        status: row.get(5)?,
        log_path: row.get(6)?,
        enqueue_seq: row.get(7)?,
        snapshot_dir: row.get(8)?,
        snapshot_hash: row.get(9)?,
        renderer_hash: row.get(10)?,
        kind: row.get(11)?,
        project_id: row.get(12)?,
        project_revision: row.get(13)?,
        document_sha: row.get(14)?,
        output_dir: row.get(15)?,
        render_plan: row.get(16)?,
        cancel_requested: row.get::<_, i64>(17)? != 0,
        pid: row.get(18)?,
        pid_starttime: row.get(19)?,
        attestation_json: row.get(20)?,
        publication_intent: row.get(21)?,
        recovery_intent: row.get(22)?,
        recovery_blocked: row.get(23)?,
        cleanup_pending: row.get::<_, i64>(24)? != 0,
        exit_code: row.get(25)?,
        error: row.get(26)?,
        created_at: row.get(27)?,
        updated_at: row.get(28)?,
        started_at: row.get(29)?,
        finished_at: row.get(30)?,
    })
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct RenderJobRow {
    pub id: String,
    pub render_key: String,
    pub task_dir: String,
    pub subject_id: String,
    pub encoder_profile: String,
    pub status: String,
    pub log_path: String,
    pub enqueue_seq: i64,
    pub snapshot_dir: String,
    pub snapshot_hash: String,
    pub renderer_hash: String,
    pub kind: String,
    pub project_id: Option<String>,
    pub project_revision: Option<i64>,
    pub document_sha: Option<String>,
    pub output_dir: Option<String>,
    pub render_plan: Option<String>,
    pub attestation_json: Option<String>,
    pub publication_intent: Option<String>,
    pub recovery_intent: Option<String>,
    pub recovery_blocked: Option<String>,
    pub cleanup_pending: bool,
    pub cancel_requested: bool,
    pub pid: Option<i64>,
    pub pid_starttime: Option<i64>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

pub struct NewVideoProjectRenderJob<'a> {
    pub id: &'a str,
    pub render_key: &'a str,
    pub project_id: &'a str,
    pub project_revision: i64,
    pub document_sha: &'a str,
    pub output_dir: &'a str,
    pub log_path: &'a str,
    pub snapshot_dir: &'a str,
    pub snapshot_hash: &'a str,
    pub renderer_hash: &'a str,
    pub render_plan: &'a str,
}

pub struct NewMediaJob<'a> {
    pub id: &'a str,
    pub render_key: &'a str,
    pub kind: &'a str,
    pub subject: &'a str,
    pub project_id: Option<&'a str>,
    pub project_revision: Option<i64>,
    pub document_sha: &'a str,
    pub output_dir: &'a str,
    pub log_path: &'a str,
    pub snapshot_dir: &'a str,
    pub snapshot_hash: &'a str,
    pub renderer_hash: &'a str,
    pub request_path: &'a str,
}

pub struct NewRenderJob<'a> {
    pub id: &'a str,
    pub render_key: &'a str,
    pub task_dir: &'a str,
    pub subject_id: &'a str,
    pub encoder_profile: &'a str,
    pub log_path: &'a str,
    pub snapshot_dir: &'a str,
    pub snapshot_hash: &'a str,
    pub renderer_hash: &'a str,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VideoProjectRow {
    pub id: String,
    pub current_revision: i64,
    pub validated_revision: Option<i64>,
    pub current_sha256: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingVideoProjectWrite {
    pub project_id: String,
    pub expected_revision: i64,
    pub new_revision: i64,
    pub old_sha: String,
    pub new_sha: String,
    pub staged_path: String,
    pub created_at: i64,
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

    #[test]
    fn variant_ids_are_unique_across_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        Database::open(&path).unwrap();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    Database::open(path)
                        .unwrap()
                        .allocate_variant_ids("XRY", 10, &["ZE".into(), "RE".into()])
                        .unwrap()
                })
            })
            .collect();
        let mut ids: Vec<String> = handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 80);
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 80);
        assert!(ids.contains(&"XRY-000001-ZE".to_string()));
    }

    #[test]
    fn existing_video_project_schema_gains_validated_revision() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE video_projects (
                   id TEXT PRIMARY KEY, current_revision INTEGER NOT NULL DEFAULT 0,
                   created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
                 );",
            )
            .unwrap();
        drop(connection);
        let database = Database::open(&path).unwrap();
        let connection = database.lock().unwrap();
        assert!(table_has_column(&connection, "video_projects", "validated_revision").unwrap());
        let pending_table: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name='pending_video_project_writes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending_table, 1);
    }

    #[test]
    fn existing_render_queue_schema_gains_identity_snapshot_and_fifo_columns() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("legacy-render.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE render_jobs (
                   id TEXT PRIMARY KEY, render_key TEXT NOT NULL, task_dir TEXT NOT NULL,
                   subject_id TEXT NOT NULL, encoder_profile TEXT NOT NULL, status TEXT NOT NULL,
                   log_path TEXT NOT NULL, cancel_requested INTEGER NOT NULL DEFAULT 0,
                   pid INTEGER, exit_code INTEGER, error TEXT, created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER
                 );
                 INSERT INTO render_jobs
                   (id,render_key,task_dir,subject_id,encoder_profile,status,log_path,created_at,updated_at)
                 VALUES
                   ('first','a','task','S01','formal-cpu','failed','a.log',10,10),
                   ('second','b','task','S01','formal-cpu','failed','b.log',10,10);",
            )
            .unwrap();
        drop(connection);

        let database = Database::open(&path).unwrap();
        let connection = database.lock().unwrap();
        for column in [
            "enqueue_seq",
            "snapshot_dir",
            "snapshot_hash",
            "renderer_hash",
            "pid_starttime",
            "kind",
            "project_id",
            "project_revision",
            "document_sha",
            "output_dir",
            "render_plan",
            "attestation_json",
            "publication_intent",
            "recovery_intent",
            "recovery_blocked",
            "cleanup_pending",
        ] {
            assert!(table_has_column(&connection, "render_jobs", column).unwrap());
        }
        let sequences: Vec<i64> = connection
            .prepare("SELECT enqueue_seq FROM render_jobs ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(sequences, vec![1, 2]);
    }

    #[test]
    fn migration_normalizes_impossible_running_publication_cancel_state() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("publication-race.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE render_jobs (
                   id TEXT PRIMARY KEY, render_key TEXT NOT NULL, task_dir TEXT NOT NULL,
                   subject_id TEXT NOT NULL, encoder_profile TEXT NOT NULL, status TEXT NOT NULL,
                   log_path TEXT NOT NULL, publication_intent TEXT,
                   cancel_requested INTEGER NOT NULL DEFAULT 0,
                   pid INTEGER, exit_code INTEGER, error TEXT, created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER
                 );
                 INSERT INTO render_jobs
                   (id,render_key,task_dir,subject_id,encoder_profile,status,log_path,
                    publication_intent,cancel_requested,created_at,updated_at)
                 VALUES
                   ('race','race-key','task','S01','formal-cpu','running','race.log',
                    '{\"schema_version\":1}',1,10,10);",
            )
            .unwrap();
        drop(connection);

        let database = Database::open(&path).unwrap();
        let row = database.render_job_by_id("race").unwrap().unwrap();
        assert_eq!(row.status, "running");
        assert!(row.publication_intent.is_some());
        assert!(!row.cancel_requested);
        let connection = database.lock().unwrap();
        assert!(connection
            .execute(
                "UPDATE render_jobs SET cancel_requested=1 WHERE id='race'",
                [],
            )
            .is_err());
    }
}
