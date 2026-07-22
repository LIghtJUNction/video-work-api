const INDEX: &str = include_str!("../static/index.html");
const APP: &str = include_str!("../static/app.js");
const STYLES: &str = include_str!("../static/styles.css");

#[test]
fn batch_workspaces_expose_queue_structure() {
    for id in [
        "targetText",
        "generationProgressText",
        "generationProgress",
        "generationJobs",
        "retryGenerationFailures",
        "videoPaths",
        "subtitleProgressText",
        "subtitleProgress",
        "subtitleJobs",
        "retrySubtitleFailures",
    ] {
        assert!(INDEX.contains(&format!("id=\"{id}\"")), "missing #{id}");
    }
    assert!(INDEX.contains("name=\"video\" type=\"file\" multiple"));
    assert!(!INDEX.contains("id=\"generationBatch\" class=\"batch-results hidden\" aria-live"));
    assert!(!INDEX.contains("id=\"subtitleBatch\" class=\"batch-results hidden\" aria-live"));
    assert_eq!(
        INDEX
            .matches("role=\"status\" aria-live=\"polite\" aria-atomic=\"true\"")
            .count(),
        2
    );
}

#[test]
fn batch_assets_are_loaded_in_order_with_matching_versions() {
    let core = INDEX.find("/static/batch-core.js?v=20260722c").unwrap();
    let prompt = INDEX.find("/static/agent-prompt.js?v=20260722-1").unwrap();
    let app = INDEX.find("/static/app.js?v=20260722d").unwrap();
    assert!(core < prompt && prompt < app);
    assert!(INDEX.contains("/static/styles.css?v=20260722c"));
}

#[test]
fn agent_prompt_is_auth_gated_and_token_is_ephemeral() {
    assert!(
        INDEX.contains("id=\"copyAgentPrompt\" class=\"secondary compact endpoint-copy hidden\"")
    );
    assert!(APP.contains("setAgentPromptAvailable(status.mcp?.configured === true)"));
    assert!(APP.contains("if (response.status === 401) setAgentPromptAvailable(false)"));
    assert!(APP.contains("createPromptAccessController"));
    assert!(APP.contains("signal: attempt.signal"));
    assert!(APP.contains("if (!agentPromptAccess.isCurrent(attempt.id)) return"));
    assert!(APP.contains("/api/auth/mcp-token"));
}

#[test]
fn batch_styles_keep_accessibility_media_contracts() {
    assert!(STYLES.contains(":focus-visible"));
    assert!(STYLES.contains("@media (prefers-reduced-motion: reduce)"));
    assert!(!STYLES.contains("--ring: var(--ring)"));
    assert!(!STYLES.contains("--shadow-pop: var(--shadow-pop)"));
}

#[test]
fn page_lifecycle_uses_pagehide_for_resource_cleanup() {
    assert!(APP.contains("window.addEventListener(\"beforeunload\""));
    assert!(APP.contains("window.addEventListener(\"pagehide\""));
    assert!(APP.contains("if (event.persisted) return;"));
}
