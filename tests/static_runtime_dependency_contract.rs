const RUNTIME_REQUIREMENTS: &str = include_str!("../scripts/requirements-runtime.txt");
const MAIN: &str = include_str!("../src/main.rs");
const SERVICE_UNIT: &str = include_str!("../systemd/video-work-api.service");
const SUBTITLES: &str = include_str!("../src/subtitles.rs");

#[test]
fn runtime_dependency_pins_and_setup_bootstrap_stay_remediated() {
    for pin in [
        "setuptools==83.0.0",
        "sentencepiece==0.2.1",
        "transformers==4.51.3",
    ] {
        assert!(
            RUNTIME_REQUIREMENTS.lines().any(|line| line.trim() == pin),
            "runtime requirements missing {pin}"
        );
    }

    assert!(!RUNTIME_REQUIREMENTS.contains("setuptools==80.10.2"));
    assert!(!RUNTIME_REQUIREMENTS.contains("sentencepiece==0.2.0"));
    assert!(!RUNTIME_REQUIREMENTS.contains("transformers==5.5.0"));
    assert!(MAIN.contains("\"setuptools==83.0.0\""));
    assert!(!MAIN.contains("\"setuptools==80.10.2\""));
}

#[test]
fn package_test_helper_discovery_uses_the_compile_time_checkout_as_a_fallback() {
    assert!(SUBTITLES.contains("#[cfg(test)]\n        roots.push(PathBuf::from(env!(\"CARGO_MANIFEST_DIR\")));"));
}

#[test]
fn service_leaves_python_selection_to_the_effective_data_dir_venv() {
    assert!(!SERVICE_UNIT.contains("Environment=VWA_PYTHON="));
    assert!(MAIN.contains("let venv_python = settings.data_dir.join(\".venv/bin/python\");"));
    assert!(MAIN.contains("std::env::var_os(\"VWA_PYTHON\").is_none()"));
}
