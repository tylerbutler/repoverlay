//! Integration tests for the remove command.

use repoverlay::{apply_overlay, remove_overlay};
use std::fs;

use crate::common::{TestContext, create_test_overlay, envrc_overlay, nested_overlay};

#[test]
fn removes_overlay_by_name() {
    let ctx = TestContext::new().with_overlay(&nested_overlay());

    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("test-overlay".to_string()),
        None,
        false,
    )
    .unwrap();
    remove_overlay(ctx.repo_path(), Some("test-overlay".to_string()), false).unwrap();

    assert!(!ctx.file_exists(".envrc"));
    assert!(!ctx.file_exists(".vscode/settings.json"));
    assert!(!ctx.state_dir_exists());
}

#[test]
fn removes_all_overlays() {
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

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists(".env.local"));

    remove_overlay(ctx.repo_path(), None, true).unwrap();

    assert!(!ctx.file_exists(".envrc"));
    assert!(!ctx.file_exists(".env.local"));
    assert!(!ctx.state_dir_exists());
}

#[test]
fn removes_one_overlay_preserves_others() {
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

    remove_overlay(ctx.repo_path(), Some("overlay-a".to_string()), false).unwrap();

    assert!(!ctx.file_exists(".envrc"));
    assert!(ctx.file_exists(".env.local"));
    assert!(ctx.state_dir_exists());
}

#[test]
fn removes_empty_parent_directories() {
    let overlay = create_test_overlay(&[(".vscode/settings.json", r#"{"key": "value"}"#)]);
    let ctx = TestContext::new();

    apply_overlay(
        overlay.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("test".to_string()),
        None,
        false,
    )
    .unwrap();
    assert!(ctx.file_exists(".vscode"));

    remove_overlay(ctx.repo_path(), Some("test".to_string()), false).unwrap();
    assert!(!ctx.file_exists(".vscode"), ".vscode should be removed");
}

#[test]
fn preserves_non_empty_parent_directories() {
    let overlay = create_test_overlay(&[(".vscode/settings.json", r#"{"key": "value"}"#)]);
    let ctx = TestContext::new();

    // Create another file in .vscode that isn't from the overlay
    fs::create_dir_all(ctx.repo_path().join(".vscode")).unwrap();
    fs::write(ctx.repo_path().join(".vscode/other.json"), "{}").unwrap();

    apply_overlay(
        overlay.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("test".to_string()),
        None,
        false,
    )
    .unwrap();
    remove_overlay(ctx.repo_path(), Some("test".to_string()), false).unwrap();

    assert!(ctx.file_exists(".vscode"), ".vscode should remain");
    assert!(ctx.file_exists(".vscode/other.json"));
}

#[test]
fn cleans_git_exclude_for_overlay() {
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
    remove_overlay(ctx.repo_path(), Some("test".to_string()), false).unwrap();

    let content = ctx.git_exclude_content();
    assert!(!content.contains("# repoverlay:test start"));
    assert!(!content.contains(".envrc"));
    assert!(!content.contains("# repoverlay:managed"));
}

#[test]
fn fails_when_no_overlay_applied() {
    let ctx = TestContext::new();

    let result = remove_overlay(ctx.repo_path(), Some("nonexistent".to_string()), false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No overlay"));
}

#[test]
fn fails_on_unknown_overlay_name() {
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

    let result = remove_overlay(ctx.repo_path(), Some("fake-overlay".to_string()), false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn handles_already_deleted_files() {
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

    // Manually delete the file
    fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();

    // Remove should still succeed
    let result = remove_overlay(ctx.repo_path(), Some("test".to_string()), false);
    assert!(result.is_ok());
}
