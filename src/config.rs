use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Runtime configuration from `VWA_*` environment variables.
#[derive(Debug, Clone)]
pub struct Settings {
    pub data_dir: PathBuf,
    pub model_dir: PathBuf,
    pub cosyvoice_root: PathBuf,
    pub setup_token_file: PathBuf,
    pub host: String,
    pub port: u16,
    pub ssl_certfile: Option<PathBuf>,
    pub ssl_keyfile: Option<PathBuf>,
    pub mcp_token: Option<String>,
    pub mcp_token_file: PathBuf,
    pub mcp_token_source: Option<McpTokenSource>,
    pub funclip_root: Option<PathBuf>,
    pub video_input_dir: PathBuf,
    pub reference_input_dir: PathBuf,
    pub subtitle_timeout_seconds: u64,
    pub project_root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTokenSource {
    Environment,
    File,
}

impl McpTokenSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Environment => "env",
            Self::File => "file",
        }
    }
}

impl Settings {
    pub fn from_env() -> Result<Self> {
        let project_root = discover_project_root();
        let data_lexical = env_lexical_path("VWA_DATA_DIR", default_data_dir());
        let data = data_lexical.canonicalize_lossy();
        let model = env_path(
            "VWA_MODEL_DIR",
            data.join("models").join("Fun-CosyVoice3-0.5B-2512"),
        );
        let cosyvoice = env_path(
            "VWA_COSYVOICE_ROOT",
            project_root.join("vendor").join("CosyVoice"),
        );
        let token = env_path("VWA_SETUP_TOKEN_FILE", data.join("setup-token"));
        // Keep this path lexical so the token layer can reject the final path
        // itself when it is a symlink. Canonicalizing here would erase that fact.
        let mcp_token_file = env_lexical_path("VWA_MCP_TOKEN_FILE", data_lexical.join("mcp-token"));
        let (mcp_token, mcp_token_source) = match env::var("VWA_MCP_TOKEN") {
            Ok(value) if !value.is_empty() => (Some(value), Some(McpTokenSource::Environment)),
            _ => (None, None),
        };
        let funclip_default = project_root.join("vendor").join("FunClip");
        let funclip_root = match env::var_os("VWA_FUNCLIP_ROOT") {
            Some(v) => Some(PathBuf::from(v).expand_user().canonicalize_lossy()),
            None if funclip_default.is_dir() => Some(funclip_default.canonicalize_lossy()),
            None => None,
        };
        let video_input_dir = env_path("VWA_VIDEO_INPUT_DIR", data.join("videos"));
        let reference_input_dir = env_path("VWA_REFERENCE_INPUT_DIR", data.join("references"));
        Ok(Self {
            data_dir: data,
            model_dir: model,
            cosyvoice_root: cosyvoice,
            setup_token_file: token,
            host: env::var("VWA_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: env::var("VWA_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(7860),
            ssl_certfile: env::var_os("VWA_SSL_CERTFILE").map(|v| PathBuf::from(v).expand_user()),
            ssl_keyfile: env::var_os("VWA_SSL_KEYFILE").map(|v| PathBuf::from(v).expand_user()),
            mcp_token,
            mcp_token_file,
            mcp_token_source,
            funclip_root,
            video_input_dir,
            reference_input_dir,
            subtitle_timeout_seconds: env::var("VWA_SUBTITLE_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1800),
            project_root,
        })
    }

    /// Load the persistent MCP token only for commands that need MCP state.
    pub fn load_mcp_token(&mut self) -> Result<()> {
        if self.mcp_token_source == Some(McpTokenSource::Environment) {
            return Ok(());
        }
        match crate::mcp_token::load(&self.mcp_token_file)? {
            Some(token) => {
                self.mcp_token = Some(token);
                self.mcp_token_source = Some(McpTokenSource::File);
            }
            None => {
                self.mcp_token = None;
                self.mcp_token_source = None;
            }
        }
        Ok(())
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("studio.sqlite3")
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.data_dir.join("profiles")
    }

    pub fn generations_dir(&self) -> PathBuf {
        self.data_dir.join("generations")
    }

    pub fn static_dir(&self) -> PathBuf {
        self.project_root.join("static")
    }

    pub fn create_data_dirs(&self) -> Result<()> {
        for path in [
            &self.data_dir,
            &self.profiles_dir(),
            &self.generations_dir(),
            &self.video_input_dir,
            &self.reference_input_dir,
        ] {
            fs::create_dir_all(path)
                .with_context(|| format!("create directory {}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            }
        }
        Ok(())
    }
}

fn default_data_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("video-work-api");
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("video-work-api");
    }
    PathBuf::from("/var/lib/video-work-api")
}

fn discover_project_root() -> PathBuf {
    if let Ok(root) = env::var("VWA_PROJECT_ROOT") {
        return PathBuf::from(root);
    }
    // Prefer cwd when it looks like the project (dev / installed layout).
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("static").is_dir() && cwd.join("Cargo.toml").is_file() {
        return cwd;
    }
    if cwd.join("static").is_dir() && cwd.join("vendor").is_dir() {
        return cwd;
    }
    // Installed package path.
    let installed = PathBuf::from("/usr/lib/video-work-api");
    if installed.join("static").is_dir() {
        return installed;
    }
    cwd
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    env::var_os(name)
        .map(|v| PathBuf::from(v).expand_user().canonicalize_lossy())
        .unwrap_or_else(|| default.expand_user().canonicalize_lossy())
}

fn env_lexical_path(name: &str, default: PathBuf) -> PathBuf {
    env::var_os(name)
        .map(|value| PathBuf::from(value).expand_user())
        .unwrap_or_else(|| default.expand_user())
}

trait PathExt {
    fn expand_user(self) -> PathBuf;
    fn canonicalize_lossy(&self) -> PathBuf;
}

impl PathExt for PathBuf {
    fn expand_user(self) -> PathBuf {
        expand_user_path(&self)
    }

    fn canonicalize_lossy(&self) -> PathBuf {
        self.canonicalize().unwrap_or_else(|_| self.clone())
    }
}

impl PathExt for &Path {
    fn expand_user(self) -> PathBuf {
        expand_user_path(self)
    }

    fn canonicalize_lossy(&self) -> PathBuf {
        self.canonicalize()
            .unwrap_or_else(|_| (*self).to_path_buf())
    }
}

fn expand_user_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if s == "~" {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde() {
        if env::var_os("HOME").is_some() {
            let p = expand_user_path(Path::new("~/video-work-api"));
            assert!(!p.to_string_lossy().starts_with('~'));
        }
    }
}
