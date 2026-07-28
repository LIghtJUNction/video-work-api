const RUNTIME_REQUIREMENTS: &str = include_str!("../scripts/requirements-runtime.txt");
const MAIN: &str = include_str!("../src/main.rs");

#[test]
fn runtime_dependency_pins_and_setup_bootstrap_stay_remediated() {
    for pin in ["setuptools==83.0.0", "sentencepiece==0.2.1"] {
        assert!(
            RUNTIME_REQUIREMENTS.lines().any(|line| line.trim() == pin),
            "runtime requirements missing {pin}"
        );
    }

    assert!(!RUNTIME_REQUIREMENTS.contains("setuptools==80.10.2"));
    assert!(!RUNTIME_REQUIREMENTS.contains("sentencepiece==0.2.0"));
    assert!(MAIN.contains("\"setuptools==83.0.0\""));
    assert!(!MAIN.contains("\"setuptools==80.10.2\""));
}
