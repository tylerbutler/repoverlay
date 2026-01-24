//! Integration tests for the apply command.

use repoverlay::apply_overlay;
use tempfile::TempDir;

use crate::common::{
    TestContext, create_test_overlay, create_test_repo, envrc_overlay, nested_overlay,
};

#[test]
fn applies_single_file() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    let result = apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        None,
        None,
        false,
    );
    assert!(result.is_ok(), "apply_overlay failed: {:?}", result);

    assert!(ctx.file_exists(".envrc"), ".envrc should exist");
    assert!(ctx.is_symlink(".envrc"), ".envrc should be a symlink");
    assert_eq!(ctx.read_file(".envrc"), "export FOO=bar");
    assert!(ctx.state_dir_exists(), "state dir should exist");
}

#[test]
fn applies_nested_files() {
    let ctx = TestContext::new().with_overlay(&nested_overlay());

    let result = apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        None,
        None,
        false,
    );
    assert!(result.is_ok());

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists(".vscode/settings.json"));
}

#[test]
fn applies_with_copy_mode() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    let result = apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        true,
        None,
        None,
        false,
    );
    assert!(result.is_ok());

    assert!(ctx.file_exists(".envrc"));
    assert!(
        !ctx.is_symlink(".envrc"),
        ".envrc should NOT be a symlink in copy mode"
    );
}

#[test]
fn updates_git_exclude_with_overlay_section() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        None,
        None,
        false,
    )
    .unwrap();

    let content = ctx.git_exclude_content();
    assert!(content.contains("# repoverlay:"));
    assert!(content.contains(" start"));
    assert!(content.contains(".envrc"));
    assert!(content.contains(" end"));
    assert!(content.contains("# repoverlay:managed start"));
    assert!(content.contains(".repoverlay"));
}

#[test]
fn respects_path_mappings() {
    let overlay = create_test_overlay(&[
        (".envrc", "export FOO=bar"),
        ("repoverlay.ccl", "mappings =\n  .envrc = .env\n"),
    ]);
    let ctx = TestContext::new();

    apply_overlay(
        overlay.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        None,
        None,
        false,
    )
    .unwrap();

    assert!(!ctx.file_exists(".envrc"), ".envrc should not exist");
    assert!(ctx.file_exists(".env"), ".env should exist");
}

#[test]
fn uses_overlay_name_from_config() {
    let overlay = create_test_overlay(&[
        (".envrc", "export FOO=bar"),
        ("repoverlay.ccl", "overlay =\n  name = my-custom-overlay\n"),
    ]);
    let ctx = TestContext::new();

    apply_overlay(
        overlay.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        None,
        None,
        false,
    )
    .unwrap();

    assert!(ctx.overlay_state_exists("my-custom-overlay"));
}

#[test]
fn uses_name_override() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("custom-name".to_string()),
        None,
        false,
    )
    .unwrap();

    assert!(ctx.overlay_state_exists("custom-name"));
}

#[test]
fn fails_on_non_git_directory() {
    let dir = TempDir::new().unwrap();
    let overlay = create_test_overlay(&envrc_overlay());

    let result = apply_overlay(
        overlay.path().to_str().unwrap(),
        dir.path(),
        false,
        None,
        None,
        false,
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
fn fails_on_duplicate_overlay_name() {
    let ctx = TestContext::new();
    let overlay1 = create_test_overlay(&envrc_overlay());
    let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

    apply_overlay(
        overlay1.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("my-overlay".to_string()),
        None,
        false,
    )
    .unwrap();

    let result = apply_overlay(
        overlay2.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("my-overlay".to_string()),
        None,
        false,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already applied"));
}

#[test]
fn fails_on_file_conflict_with_repo() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());
    ctx.create_repo_file(".envrc", "existing content");

    let result = apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        None,
        None,
        false,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Conflict"));
}

#[test]
fn fails_on_file_conflict_between_overlays() {
    let ctx = TestContext::new();
    let overlay1 = create_test_overlay(&[(".envrc", "first")]);
    let overlay2 = create_test_overlay(&[(".envrc", "second")]);

    apply_overlay(
        overlay1.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("first".to_string()),
        None,
        false,
    )
    .unwrap();

    let result = apply_overlay(
        overlay2.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("second".to_string()),
        None,
        false,
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Conflict") || err.contains("already managed"));
}

#[test]
fn fails_on_empty_overlay() {
    let repo = create_test_repo();
    let overlay = TempDir::new().unwrap();

    let result = apply_overlay(
        overlay.path().to_str().unwrap(),
        repo.path(),
        false,
        None,
        None,
        false,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No files found"));
}

#[test]
fn fails_on_nonexistent_source() {
    let repo = create_test_repo();
    let result = apply_overlay("/nonexistent/path", repo.path(), false, None, None, false);
    assert!(result.is_err());
}

#[test]
fn skips_repoverlay_ccl_config_file() {
    let overlay = create_test_overlay(&[
        (".envrc", "export FOO=bar"),
        ("repoverlay.ccl", "overlay =\n  name = test\n"),
    ]);
    let ctx = TestContext::new();

    apply_overlay(
        overlay.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        None,
        None,
        false,
    )
    .unwrap();

    // repoverlay.ccl should NOT be copied to target
    assert!(!ctx.file_exists("repoverlay.ccl"));
    // But .envrc should be
    assert!(ctx.file_exists(".envrc"));
}

#[test]
fn skips_git_directory_in_overlay() {
    let ctx = TestContext::new();

    // Create an overlay that contains a .git directory with unique files (shouldn't be copied)
    let overlay = TempDir::new().unwrap();
    std::fs::create_dir_all(overlay.path().join(".git/overlay-test-marker")).unwrap();
    std::fs::write(
        overlay.path().join(".git/overlay-test-marker/unique.txt"),
        "from overlay",
    )
    .unwrap();
    std::fs::write(overlay.path().join(".envrc"), "export FOO=bar").unwrap();

    apply_overlay(
        overlay.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("test".to_string()),
        None,
        false,
    )
    .unwrap();

    // The unique file from overlay's .git should NOT be copied
    assert!(!ctx.file_exists(".git/overlay-test-marker/unique.txt"));
    // But .envrc should be
    assert!(ctx.file_exists(".envrc"));
}

#[test]
fn creates_state_directory_structure() {
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

    // Check the full state directory structure
    assert!(ctx.repo_path().join(".repoverlay").exists());
    assert!(ctx.repo_path().join(".repoverlay/overlays").exists());
    assert!(
        ctx.repo_path()
            .join(".repoverlay/overlays/test.ccl")
            .exists()
    );
    assert!(ctx.repo_path().join(".repoverlay/meta.ccl").exists());
}

#[test]
fn normalizes_overlay_name() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    apply_overlay(
        ctx.overlay_source(),
        ctx.repo_path(),
        false,
        Some("My Test Overlay".to_string()),
        None,
        false,
    )
    .unwrap();

    // Name should be normalized to lowercase with hyphens
    assert!(ctx.overlay_state_exists("my-test-overlay"));
}

#[test]
fn applies_overlay_with_deeply_nested_files() {
    let overlay = create_test_overlay(&[
        ("a/b/c/d/deep.txt", "deep content"),
        ("x/y/z/another.txt", "another content"),
    ]);
    let ctx = TestContext::new();

    apply_overlay(
        overlay.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        Some("deep-test".to_string()),
        None,
        false,
    )
    .unwrap();

    assert!(ctx.file_exists("a/b/c/d/deep.txt"));
    assert!(ctx.file_exists("x/y/z/another.txt"));
    assert_eq!(ctx.read_file("a/b/c/d/deep.txt"), "deep content");
}
