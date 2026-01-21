//! Integration tests for the apply command.

use crate::common::{create_overlay_dir, envrc_overlay, nested_overlay, TestContext};
use repoverlay::apply_overlay;
use tempfile::TempDir;

#[test]
fn applies_single_file() {
    let ctx = TestContext::with_files(&[(".envrc", "export FOO=bar")]);

    apply_overlay(ctx.overlay_str(), ctx.repo_path(), false, None, None, false).unwrap();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.is_symlink(".envrc"));
    assert_eq!(ctx.read_file(".envrc"), "export FOO=bar");
    assert!(ctx.state_dir_exists());
}

#[test]
fn applies_nested_files() {
    let ctx = TestContext::new();
    let overlay = nested_overlay();

    apply_overlay(
        overlay.path().to_str().unwrap(),
        ctx.repo_path(),
        false,
        None,
        None,
        false,
    )
    .unwrap();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists(".vscode/settings.json"));
}

#[test]
fn applies_with_copy_mode() {
    let ctx = TestContext::with_files(&[(".envrc", "export FOO=bar")]);

    apply_overlay(ctx.overlay_str(), ctx.repo_path(), true, None, None, false).unwrap();

    assert!(ctx.file_exists(".envrc"));
    assert!(!ctx.is_symlink(".envrc"), "should NOT be a symlink in copy mode");
}

#[test]
fn updates_git_exclude_with_overlay_section() {
    let ctx = TestContext::with_files(&[(".envrc", "export FOO=bar")]);

    apply_overlay(ctx.overlay_str(), ctx.repo_path(), false, None, None, false).unwrap();

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
    let ctx = TestContext::with_files(&[
        (".envrc", "export FOO=bar"),
        (
            "repoverlay.toml",
            r#"
[mappings]
".envrc" = ".env"
"#,
        ),
    ]);

    apply_overlay(ctx.overlay_str(), ctx.repo_path(), false, None, None, false).unwrap();

    assert!(!ctx.file_exists(".envrc"), ".envrc should not exist");
    assert!(ctx.file_exists(".env"), ".env should exist");
}

#[test]
fn uses_overlay_name_from_config() {
    let ctx = TestContext::with_files(&[
        (".envrc", "export FOO=bar"),
        (
            "repoverlay.toml",
            r#"
[overlay]
name = "my-custom-overlay"
"#,
        ),
    ]);

    apply_overlay(ctx.overlay_str(), ctx.repo_path(), false, None, None, false).unwrap();

    assert!(
        ctx.overlay_state_exists("my-custom-overlay"),
        "state file should use overlay name"
    );
}

#[test]
fn uses_name_override() {
    let ctx = TestContext::with_files(&[(".envrc", "export FOO=bar")]);

    apply_overlay(
        ctx.overlay_str(),
        ctx.repo_path(),
        false,
        Some("custom-name".to_string()),
        None,
        false,
    )
    .unwrap();

    assert!(
        ctx.overlay_state_exists("custom-name"),
        "state file should use override name"
    );
}

#[test]
fn fails_on_non_git_directory() {
    let dir = TempDir::new().unwrap();
    let overlay = envrc_overlay();

    let result = apply_overlay(
        overlay.path().to_str().unwrap(),
        dir.path(),
        false,
        None,
        None,
        false,
    );

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not a git repository"));
}

#[test]
fn fails_on_duplicate_overlay_name() {
    let ctx = TestContext::with_files(&[(".envrc", "export FOO=bar")]);
    let overlay2 = create_overlay_dir(&[(".env.local", "LOCAL=true")]);

    apply_overlay(
        ctx.overlay_str(),
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
    let ctx = TestContext::with_files(&[(".envrc", "new content")]);
    ctx.write_repo_file(".envrc", "existing content");

    let result = apply_overlay(ctx.overlay_str(), ctx.repo_path(), false, None, None, false);

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Conflict"));
}

#[test]
fn fails_on_file_conflict_between_overlays() {
    let ctx = TestContext::with_files(&[(".envrc", "first")]);
    let overlay2 = create_overlay_dir(&[(".envrc", "second")]);

    apply_overlay(
        ctx.overlay_str(),
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
    let ctx = TestContext::new();

    let result = apply_overlay(
        ctx.overlay.path().to_str().unwrap(),
        ctx.repo_path(),
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
    let ctx = TestContext::new();

    let result = apply_overlay("/nonexistent/path", ctx.repo_path(), false, None, None, false);

    assert!(result.is_err());
}
