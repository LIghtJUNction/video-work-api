const INDEX: &str = include_str!("../static/index.html");
const DOCS: &str = include_str!("../static/docs.html");
const EDITOR: &str = include_str!("../static/editor.html");
const FAVICON: &str = include_str!("../static/favicon.svg");

const LOGO_MARK: &str = r##"<svg class="brand-logo" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#BCD1CA"/><path d="M17 13.5v37L49 32 17 13.5Z" fill="#FAF9F5"/><g fill="#141413"><rect x="23" y="26" width="4" height="12" rx="2"/><rect x="30" y="21" width="4" height="22" rx="2"/><rect x="37" y="26" width="4" height="12" rx="2"/></g></svg>"##;
const FAVICON_URL: &str = "/static/favicon.svg?v=20260726-logo";

#[test]
fn static_surfaces_share_one_video_audio_brand_mark() {
    assert_eq!(INDEX.matches(LOGO_MARK).count(), 1);
    assert_eq!(DOCS.matches(LOGO_MARK).count(), 1);
    assert_eq!(EDITOR.matches(LOGO_MARK).count(), 2);
    assert!(!EDITOR.contains(">VP</span>"));
}

#[test]
fn logo_palette_and_favicon_cache_version_stay_in_sync() {
    for color in ["#BCD1CA", "#FAF9F5", "#141413"] {
        assert!(FAVICON.contains(color), "favicon missing {color}");
        assert!(LOGO_MARK.contains(color), "inline mark missing {color}");
    }
    for page in [INDEX, DOCS, EDITOR] {
        assert!(page.contains(FAVICON_URL));
    }
}
