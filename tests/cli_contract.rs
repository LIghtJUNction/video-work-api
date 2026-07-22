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

    assert!(wrapper.contains("init|token|setup|model|import|passwd|status|paths|serve)"));
    assert!(wrapper.contains("--whitelist-environment="));
    assert!(wrapper.contains("-u video-work-api"));
    assert!(wrapper.contains("VWA_DATA_DIR=\"$data_root\" \"$0\" \"$@\""));
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
