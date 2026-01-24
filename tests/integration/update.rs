//! Integration tests for the update command.

use repoverlay::{apply_overlay, update_overlays};

use crate::common::{TestContext, envrc_overlay};

#[test]
fn update_fails_when_no_overlays_applied() {
    let ctx = TestContext::new();
    let result = update_overlays(ctx.repo_path(), None, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No overlays"));
}

#[test]
fn update_fails_on_unknown_overlay_name() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("real-overlay".to_string()),
        None,
        false,
    )
    .unwrap();

    let result = update_overlays(ctx.repo_path(), Some("fake-overlay".to_string()), false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not applied"));
}

#[test]
fn update_dry_run_does_not_modify_files() {
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

    // Record content before update
    let content_before = ctx.read_file(".envrc");

    // Dry run update (local overlays report as "not updatable")
    let result = update_overlays(ctx.repo_path(), None, true);
    assert!(result.is_ok());

    // Content should be unchanged
    let content_after = ctx.read_file(".envrc");
    assert_eq!(content_before, content_after);
}

#[test]
fn update_local_overlay_reports_not_updatable() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("local-test".to_string()),
        None,
        false,
    )
    .unwrap();

    // Local overlays can't be updated, but the command should succeed
    let result = update_overlays(ctx.repo_path(), Some("local-test".to_string()), false);
    assert!(result.is_ok());
}

#[test]
fn update_all_overlays_succeeds_with_local() {
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

    // Updating all overlays should succeed even with only local overlays
    let result = update_overlays(ctx.repo_path(), None, false);
    assert!(result.is_ok());
}
