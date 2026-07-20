//! `vwactl` — Video Work API control binary.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use rand::RngCore;
use tracing_subscriber::EnvFilter;
use video_work_api::config::Settings;
use video_work_api::database::Database;
use video_work_api::engine::{CosyVoiceEngine, SpeechEngine};
use video_work_api::http::{build_router, AppState, LoginLimiter};
use video_work_api::importer::import_folder;
use video_work_api::studio::Studio;
use video_work_api::subtitles::{FunClipExtractor, SubtitleExtractor};

#[derive(Parser, Debug)]
#[command(name = "vwactl", version, about = "Video Work API control")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create data dirs and one-time setup token
    Init,
    /// Create Python venv for CosyVoice/FunClip vendor dependencies
    Setup,
    /// Model operations
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },
    /// Import voice profiles from a folder tree
    Import {
        path: PathBuf,
        #[arg(long)]
        confirm_rights: bool,
    },
    /// Show readiness status
    Status,
    /// Print configured paths
    Paths,
    /// Run the HTTP service
    Serve,
}

#[derive(Subcommand, Debug)]
enum ModelCommands {
    /// Download the pinned CosyVoice3 snapshot
    Download,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let cli = Cli::parse();
    let settings = Settings::from_env()?;

    match cli.command {
        Commands::Init => cmd_init(&settings),
        Commands::Setup => cmd_setup(&settings),
        Commands::Model {
            command: ModelCommands::Download,
        } => cmd_model_download(&settings),
        Commands::Import {
            path,
            confirm_rights,
        } => cmd_import(&settings, &path, confirm_rights),
        Commands::Status => cmd_status(&settings),
        Commands::Paths => cmd_paths(&settings),
        Commands::Serve => cmd_serve(settings).await,
    }
}

fn cmd_init(settings: &Settings) -> Result<()> {
    settings.create_data_dirs()?;
    let db = Database::open(settings.database_path())?;
    if db.configured()? {
        println!("Already configured; no setup token created.");
        return Ok(());
    }
    match create_token(&settings.setup_token_file) {
        Ok(token) => {
            println!("One-time setup token (store it privately):");
            println!("{token}");
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let meta = fs::symlink_metadata(&settings.setup_token_file)?;
            if meta.file_type().is_symlink() || !meta.is_file() {
                bail!("Refusing unsafe setup-token path");
            }
            eprintln!(
                "A setup token already exists. Remove it only if you intend to rotate setup."
            );
            std::process::exit(1);
        }
        Err(e) => Err(e.into()),
    }
}

fn create_token(path: &Path) -> std::io::Result<String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    writeln!(file, "{token}")?;
    Ok(token)
}

fn cmd_setup(settings: &Settings) -> Result<()> {
    settings.create_data_dirs()?;
    let venv = settings.data_dir.join(".venv");
    let status = Command::new("uv")
        .args([
            "venv",
            venv.to_str().context("venv path")?,
            "--python",
            "3.10",
        ])
        .status()
        .context("uv venv")?;
    if !status.success() {
        bail!("uv venv failed");
    }
    let python = venv.join("bin").join("python");
    let cosy_req = settings.cosyvoice_root.join("requirements.txt");
    if cosy_req.is_file() {
        let status = Command::new("uv")
            .args([
                "pip",
                "install",
                "--python",
                python.to_str().unwrap(),
                "-r",
                cosy_req.to_str().unwrap(),
            ])
            .status()
            .context("install CosyVoice deps")?;
        if !status.success() {
            bail!("CosyVoice dependency install failed");
        }
    }
    let funclip_req = settings
        .funclip_root
        .as_ref()
        .map(|r| r.join("requirements.txt"))
        .filter(|p| p.is_file());
    if let Some(req) = funclip_req {
        let status = Command::new("uv")
            .args([
                "pip",
                "install",
                "--python",
                python.to_str().unwrap(),
                "-r",
                req.to_str().unwrap(),
            ])
            .status()
            .context("install FunClip deps")?;
        if !status.success() {
            bail!("FunClip dependency install failed");
        }
    }
    println!("Python vendor environment ready at {}", venv.display());
    println!(
        "Set VWA_PYTHON={} when running serve if needed",
        python.display()
    );
    Ok(())
}

fn cmd_model_download(settings: &Settings) -> Result<()> {
    let helper = settings
        .project_root
        .join("scripts")
        .join("download_model.py");
    if !helper.is_file() {
        bail!("download_model.py missing at {}", helper.display());
    }
    let status = Command::new("python3")
        .arg(&helper)
        .arg("--output")
        .arg(&settings.model_dir)
        .status()
        .context("download model")?;
    if !status.success() {
        bail!("model download failed");
    }
    Ok(())
}

fn cmd_import(settings: &Settings, path: &Path, confirm_rights: bool) -> Result<()> {
    if !confirm_rights {
        eprintln!("--confirm-rights is required");
        std::process::exit(2);
    }
    settings.create_data_dirs()?;
    let db = Database::open(settings.database_path())?;
    let n = import_folder(&path.canonicalize()?, settings, &db)?;
    println!("Imported {n} profiles.");
    Ok(())
}

fn cmd_status(settings: &Settings) -> Result<()> {
    let configured = if settings.database_path().is_file() {
        Database::open(settings.database_path())?.configured()?
    } else {
        false
    };
    let extractor = FunClipExtractor::new(
        settings.funclip_root.clone(),
        settings.subtitle_timeout_seconds,
    );
    println!(
        "configured: {}",
        if configured { "yes" } else { "no" }
    );
    println!(
        "model: {}",
        if settings.model_dir.is_dir() {
            "present"
        } else {
            "missing"
        }
    );
    println!(
        "cosyvoice: {}",
        if settings.cosyvoice_root.is_dir() {
            "present"
        } else {
            "missing"
        }
    );
    println!(
        "funclip: {}",
        if extractor.ready() { "ready" } else { "missing" }
    );
    println!(
        "mcp: {}",
        if settings.mcp_token.is_some() {
            "configured"
        } else {
            "unset"
        }
    );
    println!("listen: {}:{}", settings.host, settings.port);
    Ok(())
}

fn cmd_paths(settings: &Settings) -> Result<()> {
    let pairs = [
        ("data", settings.data_dir.display().to_string()),
        ("database", settings.database_path().display().to_string()),
        ("profiles", settings.profiles_dir().display().to_string()),
        (
            "generations",
            settings.generations_dir().display().to_string(),
        ),
        ("videos", settings.video_input_dir.display().to_string()),
        (
            "references",
            settings.reference_input_dir.display().to_string(),
        ),
        ("model", settings.model_dir.display().to_string()),
        ("cosyvoice", settings.cosyvoice_root.display().to_string()),
        (
            "funclip",
            settings
                .funclip_root
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "unset".into()),
        ),
        (
            "setup_token",
            settings.setup_token_file.display().to_string(),
        ),
    ];
    for (label, path) in pairs {
        println!("{label}: {path}");
    }
    println!(
        "ssl_certfile: {}",
        settings
            .ssl_certfile
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "disabled".into())
    );
    println!(
        "ssl_keyfile: {}",
        settings
            .ssl_keyfile
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "disabled".into())
    );
    println!(
        "mcp_token: {}",
        if settings.mcp_token.is_some() {
            "configured"
        } else {
            "unset"
        }
    );
    Ok(())
}

async fn cmd_serve(settings: Settings) -> Result<()> {
    if settings.ssl_certfile.is_some() != settings.ssl_keyfile.is_some() {
        eprintln!("VWA_SSL_CERTFILE and VWA_SSL_KEYFILE must be configured together");
        std::process::exit(2);
    }
    for path in [&settings.ssl_certfile, &settings.ssl_keyfile]
        .into_iter()
        .flatten()
    {
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => {
                eprintln!("TLS certificate or key is unavailable");
                std::process::exit(2);
            }
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            eprintln!("TLS certificate and key must be regular non-symlink files");
            std::process::exit(2);
        }
    }
    if settings.ssl_certfile.is_some() {
        tracing::warn!(
            "Built-in TLS is not enabled; put HTTPS reverse proxy in front for non-localhost use"
        );
    }

    settings.create_data_dirs()?;
    let venv_python = settings.data_dir.join(".venv/bin/python");
    if venv_python.is_file() && std::env::var_os("VWA_PYTHON").is_none() {
        std::env::set_var("VWA_PYTHON", &venv_python);
    }
    let database = Database::open(settings.database_path())?;
    let engine: Arc<dyn SpeechEngine> = Arc::new(CosyVoiceEngine::new(
        settings.cosyvoice_root.clone(),
        settings.model_dir.clone(),
        &settings.project_root,
    ));
    let subtitles: Arc<dyn SubtitleExtractor> = Arc::new(FunClipExtractor::new(
        settings.funclip_root.clone(),
        settings.subtitle_timeout_seconds,
    ));
    let host = settings.host.clone();
    let port = settings.port;
    let studio = Arc::new(Studio::new(settings, database, engine, subtitles));
    let state = AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
    };
    let app = build_router(state);
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    tracing::info!("Video Work API listening on http://{addr}");
    axum::serve(listener, app).await.context("serve")?;
    Ok(())
}
