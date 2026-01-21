//! Integration tests for the switch command.

use repoverlay::{apply_overlay, switch_overlay};
use tempfile::TempDir;

use crate::common::{TestContext, create_test_overlay, envrc_overlay};

#[test]
fn removes_existing_overlays_before_applying() {
    let ctx = TestContext::new();
    let overlay1 = create_test_overlay(&envrc_overlay());
    let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

    apply_overlay(
        overlay1.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("first-overlay".to_string()),
        None,
        false,
    )
    .unwrap();

    assert!(ctx.file_exists(".envrc"));

    let result = switch_overlay(
        overlay2.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("second-overlay".to_string()),
        None,
    );
    assert!(result.is_ok(), "switch_overlay failed: {:?}", result);

    assert!(!ctx.file_exists(".envrc"), ".envrc should be removed");
    assert!(ctx.file_exists(".env.local"), ".env.local should exist");
}

#[test]
fn applies_to_empty_repo() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    let result = switch_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("new-overlay".to_string()),
        None,
    );
    assert!(result.is_ok());

    assert!(ctx.file_exists(".envrc"));
}

#[test]
fn fails_on_non_git_target() {
    let target = TempDir::new().unwrap();
    let overlay = create_test_overlay(&envrc_overlay());

    let result = switch_overlay(
        overlay.path().to_str().unwrap(),
        target.path(),
        false,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not a git repository")
    );
}

#[test]
fn removes_multiple_overlays_before_applying() {
    let ctx = TestContext::new();
    let overlay1 = create_test_overlay(&envrc_overlay());
    let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);
    let overlay3 = create_test_overlay(&[(".env.prod", "PROD=true")]);

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

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists(".env.local"));

    switch_overlay(
        overlay3.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("overlay-c".to_string()),
        None,
    )
    .unwrap();

    assert!(!ctx.file_exists(".envrc"));
    assert!(!ctx.file_exists(".env.local"));
    assert!(ctx.file_exists(".env.prod"));
}
