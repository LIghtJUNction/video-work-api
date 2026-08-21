const PKGBUILD: &str = include_str!("../packaging/aur/video-work-api-git/PKGBUILD");

#[test]
fn vcs_pkgver_keeps_the_semantic_prefix_without_a_tag() {
    assert!(PKGBUILD.contains("pkgver=0.1.0.r0.g0000000"));
    assert!(PKGBUILD.contains("printf '0.1.0.r%s.%s'"));
    assert!(!PKGBUILD.contains("printf 'r%s.%s'"));
}

#[test]
fn package_checks_anchor_project_helper_discovery_to_the_checkout() {
    assert!(PKGBUILD.contains("export VWA_PROJECT_ROOT=\"$srcdir/$_pkgsrc\""));
}
