const INDEX: &str = include_str!("../static/index.html");
const DOCS: &str = include_str!("../static/docs.html");
const EDITOR: &str = include_str!("../static/editor.html");
const SCRIPT: &str = include_str!("../static/editor.js");
const STYLES: &str = include_str!("../static/editor.css");

#[test]
fn production_editor_uses_approved_workbench_and_live_inspector_structure() {
    for expected in [
        "Video Project Editor",
        "Project Explorer",
        "Text editor",
        "Live Project inspector",
        "project.vpe",
        "Current job queue",
        "Remote revision available.",
        "Create video project",
    ] {
        assert!(EDITOR.contains(expected), "missing {expected}");
    }
    assert!(EDITOR.contains("/static/editor.css?v="));
    assert!(EDITOR.contains("/static/editor.js?v="));
    assert!(INDEX.contains("href=\"/editor\""));
    assert!(DOCS.contains("href=\"/editor\""));
}

#[test]
fn production_editor_keeps_browser_secrets_and_user_text_out_of_unsafe_sinks() {
    for forbidden in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "localStorage",
        "sessionStorage",
        "Authorization",
        "access_token",
    ] {
        assert!(
            !SCRIPT.contains(forbidden),
            "unsafe editor token: {forbidden}"
        );
    }
    assert!(SCRIPT.contains("textContent"));
    assert!(SCRIPT.contains("credentials: \"same-origin\""));
    assert!(SCRIPT.contains("expected_revision"));
    assert!(SCRIPT.contains("MAX_DOCUMENT_BYTES"));
}

#[test]
fn editor_visual_contract_is_dense_flat_accessible_and_motion_safe() {
    for required in [
        "--amber:",
        "grid-template-columns:",
        "1px solid",
        "focus-visible",
        "prefers-reduced-motion",
        "minmax(",
        "--mono:",
    ] {
        assert!(STYLES.contains(required), "missing {required}");
    }
    for forbidden in [
        "linear-gradient",
        "radial-gradient",
        "backdrop-filter",
        "https://",
        "@import",
    ] {
        assert!(
            !STYLES.contains(forbidden),
            "forbidden visual token: {forbidden}"
        );
    }
}

#[test]
fn editor_actions_keep_project_vpe_as_the_only_write_target() {
    assert!(SCRIPT.contains("path: \"project.vpe\""));
    assert!(SCRIPT.contains("callEditor(\"write_file\""));
    assert!(SCRIPT.contains("callEditor(\"cancel_job\""));
    assert!(!SCRIPT.contains("delete_project"));
    assert!(!SCRIPT.contains("write_binary"));
}
