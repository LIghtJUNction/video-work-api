const INDEX: &str = include_str!("../static/index.html");
const APP: &str = include_str!("../static/app.js");
const STYLES: &str = include_str!("../static/styles.css");
const DOCS: &str = include_str!("../static/docs.html");

#[test]
fn landing_page_covers_the_broader_workflow_without_a_hero_illustration() {
    assert!(INDEX.contains("让声音、字幕与视频创作，汇成可复用的工作流。"));
    assert!(INDEX.contains("批量转写录音，提取并翻译字幕，生成自然语音，再完成视频创作与交付。"));
    assert!(INDEX.contains("class=\"endpoint-card\""));
    assert!(!INDEX.contains("class=\"hero-visual\""));
    assert!(!INDEX.contains("class=\"hero-art\""));
    assert!(!INDEX.contains("id=\"voice-source\""));
    assert!(APP.contains("Turn audio, captions, and video creation into reusable workflows."));
    assert!(APP.contains("Transcribe recordings in batches, extract and translate captions"));
    assert!(!APP.contains("Turn every voice into a reusable creative asset."));
    assert!(STYLES.contains(".hero-copy { max-width: 720px; }"));
    assert!(STYLES.contains(".endpoint-card {\n  display: flex; gap: 14px; align-items: center;\n  max-width: 620px;\n  margin-top: 30px;"));
    assert!(!STYLES.contains(".hero-visual"));
    assert!(!STYLES.contains(".hero-art"));
    assert!(INDEX.contains("/static/styles.css?v=20260821-transcription-export"));
    assert!(DOCS.contains("/static/styles.css?v=20260821-transcription-export"));
    assert!(INDEX.contains("/static/app.js?v=20260821-transcription-export"));
}

#[test]
fn login_page_explains_the_bounded_device_session_in_both_languages() {
    assert!(INDEX.contains("data-i18n=\"loginSessionHint\""));
    assert!(APP.contains("登录后，此设备将保持登录 30 天；退出或重置密码会立即失效。"));
    assert!(APP.contains("This device stays signed in for 30 days. Sign out or a password reset invalidates it immediately."));
}
