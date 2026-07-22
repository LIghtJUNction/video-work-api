use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::tempdir;
use video_work_api::database::Database;
use video_work_api::security::verify_password;

#[test]
fn help_lists_the_passwd_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("passwd"));
}

#[test]
fn passwd_help_is_still_available() {
    let output = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["passwd", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("Change the admin password"));
}

#[test]
fn installed_wrapper_reexecutes_passwd_as_the_service_account() {
    let wrapper =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/vwactl")).unwrap();

    assert!(wrapper.contains("init|token|mcp-token|setup|model|import|passwd|status|paths|serve"));
    assert!(wrapper.contains("--whitelist-environment="));
    assert!(wrapper.contains("-u video-work-api"));
    assert!(wrapper.contains("VWA_DATA_DIR=\"$data_root\" \"$0\" \"$@\""));
}

#[test]
fn package_hook_ensures_mcp_token_without_starting_the_service() {
    let hook = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/packaging/aur/video-work-api-git/video-work-api-git.install"
    ))
    .unwrap();
    assert!(hook.contains("/usr/bin/vwactl mcp-token ensure"));
    assert!(!hook.lines().any(|line| {
        let command = line.trim_start();
        command.starts_with("systemctl start ") || command.starts_with("systemctl enable ")
    }));
    assert!(!hook.contains("openssl rand"));
}

#[test]
fn mcp_token_ensure_is_stable_and_never_prints_the_value() {
    let data_dir = tempdir().unwrap();
    let first = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["mcp-token", "ensure"])
        .env("VWA_DATA_DIR", data_dir.path())
        .env_remove("VWA_MCP_TOKEN")
        .output()
        .unwrap();
    assert!(first.status.success());
    let token = fs::read_to_string(data_dir.path().join("mcp-token")).unwrap();
    assert!(!String::from_utf8_lossy(&first.stdout).contains(token.trim()));

    let second = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["mcp-token", "ensure"])
        .env("VWA_DATA_DIR", data_dir.path())
        .env_remove("VWA_MCP_TOKEN")
        .output()
        .unwrap();
    assert!(second.status.success());
    assert_eq!(
        fs::read_to_string(data_dir.path().join("mcp-token")).unwrap(),
        token
    );
}

#[test]
fn environment_token_override_does_not_create_or_print_a_token_file() {
    const OVERRIDE: &str = "compatibility-token-that-must-stay-private";
    let data_dir = tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["mcp-token", "ensure"])
        .env("VWA_DATA_DIR", data_dir.path())
        .env("VWA_MCP_TOKEN", OVERRIDE)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!data_dir.path().join("mcp-token").exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(OVERRIDE));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(OVERRIDE));
}

#[test]
fn mcp_token_rotate_changes_value_without_printing_either_value() {
    let data_dir = tempdir().unwrap();
    let ensure = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["mcp-token", "ensure"])
        .env("VWA_DATA_DIR", data_dir.path())
        .env_remove("VWA_MCP_TOKEN")
        .output()
        .unwrap();
    assert!(ensure.status.success());
    let path = data_dir.path().join("mcp-token");
    let before = fs::read_to_string(&path).unwrap();
    let rotate = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["mcp-token", "rotate"])
        .env("VWA_DATA_DIR", data_dir.path())
        .env_remove("VWA_MCP_TOKEN")
        .output()
        .unwrap();
    assert!(rotate.status.success());
    let after = fs::read_to_string(path).unwrap();
    assert_ne!(before, after);
    assert!(!String::from_utf8_lossy(&rotate.stdout).contains(before.trim()));
    assert!(!String::from_utf8_lossy(&rotate.stdout).contains(after.trim()));
    let stdout = String::from_utf8_lossy(&rotate.stdout);
    assert!(stdout.contains("restart the service"));
    assert!(stdout.contains("copy the NEW agent prompt"));
    assert!(stdout.contains("Rerun the chosen project/global install branch"));
    assert!(stdout.contains("verify the live MCP tools"));
}

#[test]
fn status_reports_only_the_mcp_token_source() {
    const OVERRIDE: &str = "status-secret-that-must-not-appear";
    let data_dir = tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .arg("status")
        .env("VWA_DATA_DIR", data_dir.path())
        .env("VWA_MCP_TOKEN", OVERRIDE)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("mcp: configured (env)"));
    assert!(!stdout.contains(OVERRIDE));
}

#[test]
fn status_loads_file_token_explicitly_without_printing_it() {
    let data_dir = tempdir().unwrap();
    let ensure = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["mcp-token", "ensure"])
        .env("VWA_DATA_DIR", data_dir.path())
        .env_remove("VWA_MCP_TOKEN")
        .output()
        .unwrap();
    assert!(ensure.status.success());
    let token = fs::read_to_string(data_dir.path().join("mcp-token")).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .arg("status")
        .env("VWA_DATA_DIR", data_dir.path())
        .env_remove("VWA_MCP_TOKEN")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status.status.success());
    assert!(stdout.contains("mcp: configured (file)"));
    assert!(!stdout.contains(token.trim()));
}

#[cfg(unix)]
#[test]
fn lexical_mcp_token_symlink_is_rejected_by_ensure_and_rotate() {
    use std::os::unix::fs::symlink;

    const ORIGINAL: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n";
    let data_dir = tempdir().unwrap();
    let target = data_dir.path().join("target-token");
    fs::write(&target, ORIGINAL).unwrap();
    let link = data_dir.path().join("linked-token");
    symlink(&target, &link).unwrap();

    for operation in ["ensure", "rotate"] {
        let output = Command::new(env!("CARGO_BIN_EXE_vwactl"))
            .args(["mcp-token", operation])
            .env("VWA_DATA_DIR", data_dir.path())
            .env("VWA_MCP_TOKEN_FILE", &link)
            .env_remove("VWA_MCP_TOKEN")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(&target).unwrap(), ORIGINAL);
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn default_token_path_does_not_canonicalize_a_symlinked_data_directory() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let actual = root.path().join("actual-data");
    fs::create_dir(&actual).unwrap();
    let linked = root.path().join("linked-data");
    symlink(&actual, &linked).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["mcp-token", "ensure"])
        .env("VWA_DATA_DIR", &linked)
        .env_remove("VWA_MCP_TOKEN_FILE")
        .env_remove("VWA_MCP_TOKEN")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!actual.join("mcp-token").exists());
}

#[cfg(unix)]
#[test]
fn rotate_recovers_corrupt_token_file_and_unrelated_passwd_still_runs() {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = tempdir().unwrap();
    let token_path = data_dir.path().join("mcp-token");
    fs::write(&token_path, "corrupt\n").unwrap();
    fs::set_permissions(&token_path, fs::Permissions::from_mode(0o644)).unwrap();
    let db = Database::open(data_dir.path().join("studio.sqlite3")).unwrap();
    assert!(db.set_admin("old-hash").unwrap());

    let passwd = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .arg("passwd")
        .env("VWA_DATA_DIR", data_dir.path())
        .env("VWA_MCP_TOKEN_FILE", &token_path)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&passwd.stderr).contains("password input requires a terminal"));

    let rotate = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["mcp-token", "rotate"])
        .env("VWA_DATA_DIR", data_dir.path())
        .env("VWA_MCP_TOKEN_FILE", &token_path)
        .env_remove("VWA_MCP_TOKEN")
        .output()
        .unwrap();
    assert!(rotate.status.success());
    assert_eq!(
        fs::metadata(token_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn passwd_rejects_extra_arguments_without_echoing_them() {
    const SECRET: &str = "THIS-MUST-NOT-APPEAR";
    let data_dir = tempdir().unwrap();
    let db = Database::open(data_dir.path().join("studio.sqlite3")).unwrap();
    assert!(db.set_admin("old-hash").unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .args(["passwd", SECRET])
        .env("VWA_DATA_DIR", data_dir.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(SECRET));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
    assert!(String::from_utf8_lossy(&output.stderr).contains("passwd accepts no arguments"));
    assert_eq!(
        db.admin_password_hash().unwrap().as_deref(),
        Some("old-hash")
    );
}

#[test]
fn passwd_rejects_non_terminal_input_without_changing_the_password() {
    let data_dir = tempdir().unwrap();
    let db = Database::open(data_dir.path().join("studio.sqlite3")).unwrap();
    assert!(db.set_admin("old-hash").unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_vwactl"))
        .arg("passwd")
        .env("VWA_DATA_DIR", data_dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("run `vwactl passwd` in a terminal"));
    assert_eq!(
        db.admin_password_hash().unwrap().as_deref(),
        Some("old-hash")
    );
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
fn run_passwd_in_pty(data_dir: &Path, input: &str) -> std::process::Output {
    let prerequisite = Command::new("script").arg("--version").output();
    assert!(
        prerequisite.is_ok_and(|output| output.status.success()),
        "PTY tests require the util-linux `script` command"
    );
    let command = format!("{} passwd", shell_quote(env!("CARGO_BIN_EXE_vwactl")));
    let mut child = Command::new("script")
        .args([
            "--quiet",
            "--return",
            "--echo",
            "never",
            "--command",
            &command,
            "/dev/null",
        ])
        .env("VWA_DATA_DIR", data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[cfg(unix)]
#[test]
fn passwd_changes_password_and_clears_sessions_in_a_pty() {
    const PASSWORD: &str = "密码密码密码密码密码密码";
    let data_dir = tempdir().unwrap();
    let db = Database::open(data_dir.path().join("studio.sqlite3")).unwrap();
    assert!(db.set_admin("old-hash").unwrap());
    db.create_session("session-one").unwrap();

    let output = run_passwd_in_pty(data_dir.path(), &format!("{PASSWORD}\n{PASSWORD}\n"));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(PASSWORD));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(PASSWORD));
    let hash = db.admin_password_hash().unwrap().unwrap();
    assert!(verify_password(PASSWORD, &hash));
    assert!(!db.session_exists("session-one").unwrap());
}

#[cfg(unix)]
#[test]
fn passwd_mismatch_in_a_pty_preserves_the_old_hash() {
    const FIRST: &str = "密码密码密码密码密码密码";
    const SECOND: &str = "口令口令口令口令口令口令";
    let data_dir = tempdir().unwrap();
    let db = Database::open(data_dir.path().join("studio.sqlite3")).unwrap();
    assert!(db.set_admin("old-hash").unwrap());

    let output = run_passwd_in_pty(data_dir.path(), &format!("{FIRST}\n{SECOND}\n"));

    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(FIRST));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(SECOND));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(FIRST));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECOND));
    assert_eq!(
        db.admin_password_hash().unwrap().as_deref(),
        Some("old-hash")
    );
}
