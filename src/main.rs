//! `vwactl` — Video Work API control binary.

use std::env;
use std::ffi::{OsStr, OsString};
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
use video_work_api::config::{McpTokenSource, Settings};
use video_work_api::database::Database;
use video_work_api::engine::{CosyVoiceEngine, SpeechEngine};
use video_work_api::http::{build_router, AppState, LoginLimiter};
use video_work_api::importer::import_folder;
use video_work_api::model::{download_model, model_files_present, ModelDownloadManager};
use video_work_api::passkeys::CeremonyStore;
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
    /// Create data dirs, persistent MCP token, and one-time web setup token
    Init {
        /// Print only the token value (no labels; good for piping)
        #[arg(long)]
        raw: bool,
        /// Replace an existing pending token
        #[arg(long)]
        rotate: bool,
    },
    /// Show or create the one-time web setup token
    Token {
        #[command(subcommand)]
        command: Option<TokenCommands>,
        /// Print only the token value (no labels; good for piping)
        #[arg(long, global = true)]
        raw: bool,
    },
    /// Manage the persistent MCP bearer token (never prints its value)
    McpToken {
        #[command(subcommand)]
        command: McpTokenCommands,
    },
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
    /// Change the admin password (signs out all web sessions)
    Passwd,
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

#[derive(Subcommand, Debug)]
enum TokenCommands {
    /// Print the pending one-time setup token (create if missing)
    Show,
    /// Create a new one-time setup token
    Create {
        /// Replace an existing pending token
        #[arg(long)]
        rotate: bool,
    },
}

#[derive(Subcommand, Debug)]
enum McpTokenCommands {
    /// Create the persistent MCP token if it is absent
    Ensure,
    /// Atomically replace the persistent MCP token
    Rotate,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<OsString> = env::args_os().collect();
    reject_passwd_arguments(&args)?;

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let cli = Cli::parse_from(args);
    let mut settings = Settings::from_env()?;

    match cli.command {
        Commands::Init { raw, rotate } => {
            ensure_runtime_mcp_token(&mut settings)?;
            cmd_token_ensure(&settings, raw, rotate)
        }
        Commands::Token { command, raw } => match command.unwrap_or(TokenCommands::Show) {
            TokenCommands::Show => cmd_token_show(&settings, raw, /*create_if_missing*/ true),
            TokenCommands::Create { rotate } => cmd_token_ensure(&settings, raw, rotate),
        },
        Commands::McpToken { command } => match command {
            McpTokenCommands::Ensure => cmd_mcp_token_ensure(&mut settings),
            McpTokenCommands::Rotate => cmd_mcp_token_rotate(&mut settings),
        },
        Commands::Setup => cmd_setup(&settings),
        Commands::Model {
            command: ModelCommands::Download,
        } => {
            download_model(&settings)?;
            println!("Model directory: {}", settings.model_dir.display());
            Ok(())
        }
        Commands::Import {
            path,
            confirm_rights,
        } => cmd_import(&settings, &path, confirm_rights),
        Commands::Passwd => cmd_passwd(&settings),
        Commands::Status => cmd_status(&mut settings),
        Commands::Paths => cmd_paths(&mut settings),
        Commands::Serve => cmd_serve(settings).await,
    }
}

fn ensure_runtime_mcp_token(settings: &mut Settings) -> Result<()> {
    if settings.mcp_token_source == Some(McpTokenSource::Environment) {
        return Ok(());
    }
    settings.load_mcp_token()?;
    if settings.mcp_token.is_some() {
        return Ok(());
    }
    let token = video_work_api::mcp_token::ensure(&settings.mcp_token_file)?;
    settings.mcp_token = Some(token);
    settings.mcp_token_source = Some(McpTokenSource::File);
    Ok(())
}

fn cmd_mcp_token_ensure(settings: &mut Settings) -> Result<()> {
    ensure_runtime_mcp_token(settings)?;
    println!(
        "MCP token ready (source: {}). Value was not printed.",
        settings
            .mcp_token_source
            .map(McpTokenSource::label)
            .unwrap_or("unset")
    );
    Ok(())
}

fn cmd_mcp_token_rotate(settings: &mut Settings) -> Result<()> {
    if settings.mcp_token_source == Some(McpTokenSource::Environment) {
        bail!("VWA_MCP_TOKEN override is active; remove it before rotating the token file");
    }
    let token = video_work_api::mcp_token::rotate(&settings.mcp_token_file)?;
    settings.mcp_token = Some(token);
    settings.mcp_token_source = Some(McpTokenSource::File);
    println!("MCP token rotated. Value was not printed.");
    println!("Next: restart the service, sign in as administrator, and copy the NEW agent prompt.");
    println!("Rerun the chosen project/global install branch to replace the static token.");
    println!("Then restart/open a new Codex session and verify the live MCP tools.");
    Ok(())
}

fn reject_passwd_arguments(args: &[OsString]) -> Result<()> {
    if args.get(1).map(OsString::as_os_str) != Some(OsStr::new("passwd")) {
        return Ok(());
    }
    let allowed = match args.get(2..) {
        Some([]) => true,
        Some([argument]) => argument == OsStr::new("-h") || argument == OsStr::new("--help"),
        _ => false,
    };
    if !allowed {
        bail!("passwd accepts no arguments; enter the password interactively");
    }
    Ok(())
}

/// `vwactl init` / `vwactl token create`: ensure a pending token exists and print it.
fn cmd_token_ensure(settings: &Settings, raw: bool, rotate: bool) -> Result<()> {
    settings.create_data_dirs()?;
    let db = Database::open(settings.database_path())?;
    if db.configured()? {
        if raw {
            bail!("already configured; setup token is not available");
        }
        println!("Already configured; first-time setup is complete.");
        println!("Sign in on the web UI with the admin password (no setup token).");
        println!("Tip: vwactl token show   # confirms no pending token after setup");
        return Ok(());
    }

    if rotate && settings.setup_token_file.exists() {
        let meta = fs::symlink_metadata(&settings.setup_token_file)?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            bail!("Refusing unsafe setup-token path");
        }
        fs::remove_file(&settings.setup_token_file).with_context(|| {
            format!(
                "remove pending setup token {}",
                settings.setup_token_file.display()
            )
        })?;
    }

    match create_token(&settings.setup_token_file) {
        Ok(token) => {
            print_token(&token, raw, /*created*/ true, settings);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let token = read_token_file(&settings.setup_token_file)?;
            print_token(&token, raw, /*created*/ false, settings);
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// `vwactl token` / `vwactl token show`.
fn cmd_token_show(settings: &Settings, raw: bool, create_if_missing: bool) -> Result<()> {
    settings.create_data_dirs()?;
    let db = Database::open(settings.database_path())?;
    if db.configured()? {
        if raw {
            bail!("already configured; setup token is not available");
        }
        println!("Already configured; no pending setup token.");
        println!("Use the admin password on the login page.");
        return Ok(());
    }

    if token_file_present(&settings.setup_token_file)? {
        let token = read_token_file(&settings.setup_token_file)?;
        print_token(&token, raw, /*created*/ false, settings);
        return Ok(());
    }

    if create_if_missing {
        let token = create_token(&settings.setup_token_file)?;
        print_token(&token, raw, /*created*/ true, settings);
        return Ok(());
    }

    if raw {
        bail!("no pending setup token");
    }
    println!("No pending setup token.");
    println!("Create one with: vwactl token create");
    Ok(())
}

fn print_token(token: &str, raw: bool, created: bool, settings: &Settings) {
    if raw {
        // Token only on stdout for scripts / clipboard tools.
        println!("{token}");
        return;
    }
    if created {
        println!("One-time setup token (store it privately):");
    } else {
        println!("Pending one-time setup token:");
    }
    println!("{token}");
    eprintln!();
    eprintln!("Paste it in the web UI first-time setup form, then set a 12+ char admin password.");
    eprintln!("Token file: {}", settings.setup_token_file.display());
    eprintln!("Re-print later:  vwactl token");
    eprintln!("Rotate:          vwactl token create --rotate");
    eprintln!("Scripting:       vwactl token --raw");
}

/// `vwactl passwd`: interactive password change; clears web sessions.
fn cmd_passwd(settings: &Settings) -> Result<()> {
    use std::io::IsTerminal;

    settings.create_data_dirs()?;
    let db = Database::open(settings.database_path())?;
    if !db.configured()? {
        bail!("administrator is not configured; complete first-time web setup first");
    }
    if !std::io::stdin().is_terminal() {
        bail!("password input requires a terminal; run `vwactl passwd` in a terminal");
    }

    let first = rpassword::prompt_password("New admin password (12+ chars): ")?;
    if !video_work_api::security::is_admin_password_valid(&first) {
        bail!("password must be at least 12 characters");
    }
    let second = rpassword::prompt_password("Confirm new password: ")?;
    if first != second {
        bail!("passwords do not match");
    }
    let hash = video_work_api::security::hash_password(&first)?;
    db.change_admin_password_and_clear_sessions(&hash)?;
    println!("Password updated; all web sessions signed out.");
    Ok(())
}

fn token_file_present(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                bail!("Refusing unsafe setup-token path");
            }
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn read_token_file(path: &Path) -> Result<String> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        bail!("Refusing unsafe setup-token path");
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        eprintln!(
            "warning: setup token file permissions are {:o} (prefer 0600)",
            mode
        );
    }
    let token = fs::read_to_string(path)
        .with_context(|| format!("read setup token {}", path.display()))?
        .trim()
        .to_string();
    if token.is_empty() {
        bail!("Setup token file is empty");
    }
    Ok(token)
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
            // Re-run setup must replace an empty/broken prior venv.
            "--clear",
        ])
        .status()
        .context("uv venv")?;
    if !status.success() {
        bail!("uv venv failed");
    }
    let python = venv.join("bin").join("python");
    // Prefer inference-only pins (no TensorRT / training stack). Fall back to
    // the vendored CosyVoice requirements when the runtime file is absent.
    let runtime_req = settings
        .project_root
        .join("scripts")
        .join("requirements-runtime.txt");
    let vendor_req = settings.cosyvoice_root.join("requirements.txt");
    let cosy_req = if runtime_req.is_file() {
        runtime_req
    } else {
        vendor_req
    };
    if cosy_req.is_file() {
        // The legacy whisper sdist imports pkg_resources without declaring setuptools.
        let status = Command::new("uv")
            .args([
                "pip",
                "install",
                "--python",
                python.to_str().unwrap(),
                "setuptools==80.10.2",
            ])
            .status()
            .context("install legacy whisper build dependency")?;
        if !status.success() {
            bail!("install legacy whisper build dependency failed");
        }

        // CosyVoice pins use multiple extra indexes (PyTorch + onnxruntime-gpu).
        // uv's default first-index wins and can miss protobuf on the CUDA index.
        let status = Command::new("uv")
            .args([
                "pip",
                "install",
                "--python",
                python.to_str().unwrap(),
                "--index-strategy",
                "unsafe-best-match",
                // Reuse the bootstrapped setuptools while building legacy whisper.
                "--no-build-isolation-package",
                "openai-whisper",
                "-r",
                cosy_req.to_str().unwrap(),
            ])
            .status()
            .context("install CosyVoice runtime deps")?;
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
                "--index-strategy",
                "unsafe-best-match",
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

fn cmd_status(settings: &mut Settings) -> Result<()> {
    settings.load_mcp_token()?;
    let configured = if settings.database_path().is_file() {
        Database::open(settings.database_path())?.configured()?
    } else {
        false
    };
    let extractor = FunClipExtractor::new(
        settings.funclip_root.clone(),
        settings.subtitle_timeout_seconds,
    );
    let model_present = model_files_present(settings);
    let runtime_ready = python_runtime_ready(settings) && settings.cosyvoice_root.is_dir();
    let setup_token = if configured {
        "consumed"
    } else if token_file_present(&settings.setup_token_file).unwrap_or(false) {
        "pending"
    } else {
        "missing"
    };
    let python = resolve_python(settings);

    println!("configured: {}", if configured { "yes" } else { "no" });
    println!("setup_token: {setup_token}");
    if !configured && setup_token == "pending" {
        println!("  tip: vwactl token          # print the one-time token");
        println!("  tip: vwactl token --raw    # token only (for copy/pipe)");
    } else if !configured && setup_token == "missing" {
        println!("  tip: vwactl init           # create a one-time token");
    }
    println!(
        "model_present: {}",
        if model_present { "yes" } else { "no" }
    );
    println!(
        "model_runtime: {}",
        if runtime_ready { "yes" } else { "no" }
    );
    println!(
        "model_ready: {}",
        if model_present && runtime_ready {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "python: {}",
        python
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "missing".into())
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
        if extractor.ready() {
            "ready"
        } else {
            "missing"
        }
    );
    println!(
        "mcp: {}",
        settings
            .mcp_token_source
            .map(|source| format!("configured ({})", source.label()))
            .unwrap_or_else(|| "unset".into())
    );
    println!("listen: {}:{}", settings.host, settings.port);
    Ok(())
}

fn python_runtime_ready(settings: &Settings) -> bool {
    resolve_python(settings).is_some()
}

fn resolve_python(settings: &Settings) -> Option<PathBuf> {
    if let Ok(p) = env::var("VWA_PYTHON") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    let venv = settings.data_dir.join(".venv/bin/python");
    if venv.is_file() {
        return Some(venv);
    }
    None
}

fn cmd_paths(settings: &mut Settings) -> Result<()> {
    settings.load_mcp_token()?;
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
        settings
            .mcp_token_source
            .map(|source| format!("configured ({})", source.label()))
            .unwrap_or_else(|| "unset".into())
    );
    Ok(())
}

async fn cmd_serve(mut settings: Settings) -> Result<()> {
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
    ensure_runtime_mcp_token(&mut settings)?;
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
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_download: Arc::new(ModelDownloadManager::new()),
    };
    let app = build_router(state);
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    tracing::info!("Video Work API listening on http://{addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("serve")?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let ctrl_c = tokio::signal::ctrl_c();
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut signal) => {
                    signal.recv().await;
                }
                Err(error) => {
                    tracing::error!(%error, "install SIGTERM handler");
                    std::future::pending::<()>().await;
                }
            }
        };
        tokio::select! {
            result = ctrl_c => {
                if let Err(error) = result {
                    tracing::error!(%error, "listen for Ctrl-C");
                }
            }
            _ = terminate => {}
        }
    }

    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "listen for Ctrl-C");
    }
}
