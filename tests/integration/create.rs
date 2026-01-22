//! Integration tests for the create command.

use repoverlay::create_overlay;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use crate::common::create_test_repo;

#[test]
fn creates_overlay_with_single_file() {
    let source = create_test_repo();
    let output = TempDir::new().unwrap();

    fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

    let result = create_overlay(
        source.path(),
        Some(output.path().join("test-overlay")),
        &[PathBuf::from(".envrc")],
        None,
        false,
        false,
    );
    assert!(result.is_ok(), "create_overlay failed: {:?}", result);

    let overlay_file = output.path().join("test-overlay/.envrc");
    assert!(overlay_file.exists(), ".envrc should exist in overlay");

    let content = fs::read_to_string(&overlay_file).unwrap();
    assert_eq!(content, "export FOO=bar");

    let config_file = output.path().join("test-overlay/repoverlay.ccl");
    assert!(config_file.exists(), "repoverlay.ccl should exist");
}

#[test]
fn creates_overlay_with_directory() {
    let source = create_test_repo();
    let output = TempDir::new().unwrap();

    fs::create_dir_all(source.path().join(".claude")).unwrap();
    fs::write(
        source.path().join(".claude/settings.json"),
        r#"{"key": "value"}"#,
    )
    .unwrap();
    fs::write(source.path().join(".claude/commands.md"), "# Commands").unwrap();

    let result = create_overlay(
        source.path(),
        Some(output.path().join("test-overlay")),
        &[PathBuf::from(".claude")],
        None,
        false,
        false,
    );
    assert!(result.is_ok(), "create_overlay failed: {:?}", result);

    let overlay_dir = output.path().join("test-overlay/.claude");
    assert!(overlay_dir.exists(), ".claude directory should exist");
    assert!(overlay_dir.join("settings.json").exists());
    assert!(overlay_dir.join("commands.md").exists());
}

#[test]
fn generates_repoverlay_ccl_with_name() {
    let source = create_test_repo();
    let output = TempDir::new().unwrap();

    fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

    let result = create_overlay(
        source.path(),
        Some(output.path().join("test-overlay")),
        &[PathBuf::from(".envrc")],
        Some("my-custom-name".to_string()),
        false,
        false,
    );
    assert!(result.is_ok());

    let config_content =
        fs::read_to_string(output.path().join("test-overlay/repoverlay.ccl")).unwrap();
    assert!(config_content.contains("my-custom-name"));
}

#[test]
fn dry_run_does_not_create_files() {
    let source = create_test_repo();
    let output = TempDir::new().unwrap();

    fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

    let result = create_overlay(
        source.path(),
        Some(output.path().join("test-overlay")),
        &[PathBuf::from(".envrc")],
        None,
        true, // dry_run
        false,
    );
    assert!(result.is_ok());

    assert!(!output.path().join("test-overlay").exists());
}

#[test]
fn fails_when_no_files_specified_and_none_discovered() {
    let source = create_test_repo();
    let output = TempDir::new().unwrap();

    let result = create_overlay(
        source.path(),
        Some(output.path().join("test-overlay")),
        &[],
        None,
        false,
        false,
    );
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("No files") || err_msg.contains("--include"),
        "Expected error about no files, got: {}",
        err_msg
    );
}

#[test]
fn dry_run_shows_discovered_files() {
    let source = create_test_repo();

    fs::create_dir_all(source.path().join(".claude")).unwrap();
    fs::write(source.path().join(".claude/settings.json"), "{}").unwrap();
    fs::write(source.path().join("CLAUDE.md"), "# Claude").unwrap();

    let result = create_overlay(source.path(), None, &[], None, true, false);
    assert!(result.is_ok());
}

#[test]
fn fails_on_nonexistent_include_path() {
    let source = create_test_repo();
    let output = TempDir::new().unwrap();

    let result = create_overlay(
        source.path(),
        Some(output.path().join("test-overlay")),
        &[PathBuf::from("nonexistent.txt")],
        None,
        false,
        false,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));
}

#[test]
fn fails_on_non_git_source() {
    let source = TempDir::new().unwrap();
    let output = TempDir::new().unwrap();

    fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

    let result = create_overlay(
        source.path(),
        Some(output.path().join("test-overlay")),
        &[PathBuf::from(".envrc")],
        None,
        false,
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
fn non_tty_fallback_uses_preselected_ai_configs() {
    // In a non-TTY environment (like test runner), when no --include is specified
    // and --yes is not set, the selection UI should fall back to preselected files
    // (AI configs) without requiring interactive input.
    let source = create_test_repo();
    let output = TempDir::new().unwrap();

    // Create AI config files (these should be preselected)
    fs::write(source.path().join("CLAUDE.md"), "# Claude config").unwrap();
    fs::create_dir_all(source.path().join(".claude")).unwrap();
    fs::write(source.path().join(".claude/settings.json"), "{}").unwrap();

    // Create a non-AI file (should not be included in fallback)
    fs::write(source.path().join(".gitignore"), ".envrc").unwrap();
    fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

    // Run with yes=false (interactive mode), no includes specified
    // In non-TTY, this should use preselected AI configs
    let result = create_overlay(
        source.path(),
        Some(output.path().join("test-overlay")),
        &[], // no explicit includes
        None,
        false, // not dry-run
        false, // not --yes (would trigger interactive mode, falls back in non-TTY)
    );

    assert!(result.is_ok(), "create_overlay failed: {:?}", result);

    // Verify AI configs were included
    let overlay_dir = output.path().join("test-overlay");
    assert!(
        overlay_dir.join("CLAUDE.md").exists(),
        "CLAUDE.md should be included"
    );
    assert!(
        overlay_dir.join(".claude/settings.json").exists(),
        ".claude/settings.json should be included"
    );

    // Verify non-preselected files were NOT included
    assert!(
        !overlay_dir.join(".envrc").exists(),
        ".envrc should NOT be included (not preselected)"
    );
}
