//! Integration tests for the status command.

use repoverlay::{apply_overlay, show_status};

use crate::common::{TestContext, create_test_overlay, envrc_overlay};

#[test]
fn shows_no_overlay_when_none_applied() {
    let ctx = TestContext::new();
    let result = show_status(ctx.repo_path(), None);
    assert!(result.is_ok());
}

#[test]
fn shows_status_when_overlay_applied() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("test".to_string()),
        None,
        false,
    )
    .unwrap();

    let result = show_status(ctx.repo_path(), None);
    assert!(result.is_ok());
}

#[test]
fn shows_status_for_multiple_overlays() {
    let ctx = TestContext::new();
    let overlay1 = create_test_overlay(&envrc_overlay());
    let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

    apply_overlay(
        overlay1.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("overlay-a".to_string()),
        None,
        false,
    )
    .unwrap();
    apply_overlay(
        overlay2.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("overlay-b".to_string()),
        None,
        false,
    )
    .unwrap();

    let result = show_status(ctx.repo_path(), None);
    assert!(result.is_ok());
}

#[test]
fn shows_status_for_specific_overlay() {
    let ctx = TestContext::new();
    let overlay1 = create_test_overlay(&envrc_overlay());
    let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

    apply_overlay(
        overlay1.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("overlay-a".to_string()),
        None,
        false,
    )
    .unwrap();
    apply_overlay(
        overlay2.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("overlay-b".to_string()),
        None,
        false,
    )
    .unwrap();

    let result = show_status(ctx.repo_path(), Some("overlay-a".to_string()));
    assert!(result.is_ok());
}

#[test]
fn fails_on_unknown_overlay_filter() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("real".to_string()),
        None,
        false,
    )
    .unwrap();

    let result = show_status(ctx.repo_path(), Some("fake".to_string()));
    assert!(result.is_err());
}
