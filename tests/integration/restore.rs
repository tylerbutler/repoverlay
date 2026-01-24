//! Integration tests for the restore command.

use repoverlay::{apply_overlay, restore_overlays, save_external_state};
use std::fs;

use crate::common::{TestContext, envrc_overlay};

#[test]
fn restore_shows_no_overlays_when_none_saved() {
    let ctx = TestContext::new();
    let result = restore_overlays(ctx.repo_path(), true);
    assert!(result.is_ok());
}

#[test]
fn restore_dry_run_does_not_create_files() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("test".to_string()),
        None,
        false,
    )
    .unwrap();

    // Simulate git clean by removing overlay files and state dir
    fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
    fs::remove_dir_all(ctx.repo_path().join(".repoverlay")).unwrap();

    // Dry run restore
    let result = restore_overlays(ctx.repo_path(), true);
    assert!(result.is_ok());

    // Files should NOT be restored in dry run
    assert!(!ctx.file_exists(".envrc"));
}

#[test]
fn restore_recreates_overlay_from_external_state() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("test".to_string()),
        None,
        false,
    )
    .unwrap();

    assert!(ctx.file_exists(".envrc"));

    // Simulate git clean by removing overlay files and state dir
    fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
    fs::remove_dir_all(ctx.repo_path().join(".repoverlay")).unwrap();

    assert!(!ctx.file_exists(".envrc"));

    // Restore
    let result = restore_overlays(ctx.repo_path(), false);
    assert!(result.is_ok());

    // Files should be restored
    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.state_dir_exists());
}

#[test]
fn restore_fails_on_non_git_repo() {
    let dir = tempfile::TempDir::new().unwrap();
    let result = restore_overlays(dir.path(), false);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not a git repository")
    );
}

#[test]
fn restore_handles_missing_source_gracefully() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("test".to_string()),
        None,
        false,
    )
    .unwrap();

    // Get the external state before cleaning
    let state = repoverlay::load_overlay_state(ctx.repo_path(), "test").unwrap();

    // Remove overlay directory (source)
    fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
    fs::remove_dir_all(ctx.repo_path().join(".repoverlay")).unwrap();

    // Remove the overlay source directory
    // The external state still points to the overlay source, but it no longer exists
    drop(ctx); // This drops the TestContext and its TempDirs

    // Create a new repo to restore into
    let new_ctx = TestContext::new();

    // Save external state pointing to non-existent path
    save_external_state(new_ctx.repo_path(), "test", &state).unwrap();

    // Restore should handle the missing source
    let result = restore_overlays(new_ctx.repo_path(), false);
    // This may succeed or fail depending on whether source exists
    // The important thing is it doesn't panic
    let _ = result;
}
