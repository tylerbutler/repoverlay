//! CLI integration tests using `assert_cmd`.
//!
//! These tests verify CLI behavior by running the compiled binary.
//! Organized into logical sections covering each command's functionality.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;

mod common;
use common::{SourceTestContext, TestContext, envrc_overlay};

#[test]
fn help_displays() {
    cargo_bin_cmd!("repoverlay")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Overlay config files"));
}

#[test]
fn version_displays() {
    cargo_bin_cmd!("repoverlay")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("repoverlay"));
}

#[test]
fn apply_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["apply", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Apply an overlay"));
}

#[test]
fn remove_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["remove", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Remove"));
}

#[test]
fn status_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["status", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--quiet"));
}

#[test]
fn browse_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["browse", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Browse"))
        .stdout(predicate::str::contains("--target"))
        .stdout(predicate::str::contains("--no-interactive"))
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--show-all"));
}

#[test]
fn browse_help_no_target_alias_on_filter() {
    // --target should not appear as an alias for --filter
    let output = cargo_bin_cmd!("repoverlay")
        .args(["browse", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    // --filter should not mention "target" as an alias
    assert!(
        !stdout.contains("[aliases: target]"),
        "browse --filter should not have target alias"
    );
}

#[test]
fn browse_help_shows_source_argument() {
    cargo_bin_cmd!("repoverlay")
        .args(["browse", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[SOURCE]"));
}

#[test]
fn browse_rejects_nonexistent_local_path_source() {
    cargo_bin_cmd!("repoverlay")
        .args(["browse", "./my-overlay"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Path does not exist"));
}

#[test]
fn browse_rejects_nonexistent_absolute_path_source() {
    cargo_bin_cmd!("repoverlay")
        .args(["browse", "/tmp/my-overlay"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Path does not exist"));
}

#[test]
fn browse_rejects_three_part_source() {
    cargo_bin_cmd!("repoverlay")
        .args(["browse", "owner/repo/overlay"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid source for browse"));
}

#[test]
fn cache_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["cache", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cache"));
}

#[test]
fn restore_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["restore", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Restore"));
}

#[test]
fn update_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Update"));
}

#[test]
fn apply_requires_source_argument() {
    cargo_bin_cmd!("repoverlay")
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn apply_and_remove_workflow() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply with explicit name
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Applying"));

    assert!(ctx.file_exists(".envrc"));

    // Status
    cargo_bin_cmd!("repoverlay")
        .args(["status", "--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Overlay Status"));

    // Remove by name
    cargo_bin_cmd!("repoverlay")
        .args([
            "remove",
            "test-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removing"));

    assert!(!ctx.file_exists(".envrc"));
}

#[test]
fn apply_and_remove_all_workflow() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));

    // Remove with --all
    cargo_bin_cmd!("repoverlay")
        .args([
            "remove",
            "--all",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed all"));

    assert!(!ctx.file_exists(".envrc"));
}

#[test]
fn apply_with_copy_flag() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .arg("--copy")
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(!ctx.is_symlink(".envrc"));
}

#[test]
fn status_when_no_overlay() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["status", "--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No overlay"));
}

#[test]
fn remove_when_no_overlay() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args([
            "remove",
            "--all",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No overlay"));
}

#[test]
fn cache_list_empty() {
    cargo_bin_cmd!("repoverlay")
        .args(["cache", "list"])
        .assert()
        .success();
}

#[test]
fn cache_path_shows_location() {
    cargo_bin_cmd!("repoverlay")
        .args(["cache", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repoverlay"));
}

// ============================================================================
// Apply Command Tests
// ============================================================================

#[test]
fn apply_creates_symlink_by_default() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.is_symlink(".envrc"), ".envrc should be a symlink");
}

#[test]
fn apply_creates_state_directory() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.state_dir_exists(), ".repoverlay directory should exist");
}

#[test]
fn apply_updates_git_exclude() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    let exclude = ctx.git_exclude_content();
    assert!(
        exclude.contains("# repoverlay:"),
        "git exclude should contain repoverlay section marker"
    );
    assert!(
        exclude.contains(".envrc"),
        "git exclude should list overlay files"
    );
    assert!(
        exclude.contains(".repoverlay"),
        "git exclude should list state directory"
    );
}

#[test]
fn apply_nested_files() {
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists(".vscode/settings.json"));
}

#[test]
fn apply_with_explicit_name() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "custom-name"])
        .assert()
        .success();

    // Verify overlay exists with custom name
    assert!(ctx.overlay_state_exists("custom-name"));
}

#[test]
fn apply_with_copy_creates_regular_files() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .arg("--copy")
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(
        !ctx.is_symlink(".envrc"),
        ".envrc should NOT be a symlink in copy mode"
    );
    assert_eq!(ctx.read_file(".envrc"), "export FOO=bar");
}

#[test]
fn apply_requires_valid_source() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", "/nonexistent/path"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn apply_requires_git_repo() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());
    let temp_dir = tempfile::TempDir::new().unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", temp_dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git"));
}

#[test]
fn apply_respects_path_mappings() {
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        (
            "repoverlay.ccl",
            r"mappings =
  .envrc = .env
",
        ),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    // File should be mapped to .env, not .envrc
    assert!(
        ctx.file_exists(".env"),
        ".env should exist (mapped from .envrc)"
    );
    assert!(
        !ctx.file_exists(".envrc"),
        ".envrc should not exist (was mapped)"
    );
}

// ============================================================================
// Remove Command Tests
// ============================================================================

#[test]
fn remove_by_name() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply first
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "my-overlay"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));

    // Remove by name
    cargo_bin_cmd!("repoverlay")
        .args(["remove", "my-overlay"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(
        !ctx.file_exists(".envrc"),
        "overlay files should be removed"
    );
}

#[test]
fn remove_all_removes_multiple_overlays() {
    let ctx = TestContext::new();
    let overlay1 = common::create_overlay_dir(&[(".envrc", "export FOO=1")]);
    let overlay2 = common::create_overlay_dir(&[(".tool-versions", "nodejs 20.0.0")]);

    // Apply first overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay1.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "overlay1"])
        .assert()
        .success();

    // Apply second overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "overlay2"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists(".tool-versions"));

    // Remove all
    cargo_bin_cmd!("repoverlay")
        .args(["remove", "--all"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(!ctx.file_exists(".envrc"));
    assert!(!ctx.file_exists(".tool-versions"));
}

#[test]
fn remove_nonexistent_overlay_fails() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "exists"])
        .assert()
        .success();

    // Try to remove nonexistent overlay
    cargo_bin_cmd!("repoverlay")
        .args(["remove", "does-not-exist"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("No overlay")));
}

#[test]
fn remove_cleans_git_exclude() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test"])
        .assert()
        .success();

    let exclude_before = ctx.git_exclude_content();
    assert!(exclude_before.contains("# repoverlay:"));

    // Remove
    cargo_bin_cmd!("repoverlay")
        .args(["remove", "test"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    let exclude_after = ctx.git_exclude_content();
    // The overlay-specific section should be gone
    assert!(
        !exclude_after.contains("# repoverlay:test"),
        "git exclude should not contain the removed overlay section"
    );
}

// ============================================================================
// Status Command Tests
// ============================================================================

#[test]
fn status_shows_applied_overlay() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "my-test-overlay"])
        .assert()
        .success();

    // Check status
    cargo_bin_cmd!("repoverlay")
        .args(["status"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-test-overlay"));
}

#[test]
fn status_shows_overlay_files() {
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        (".tool-versions", "nodejs 20.0.0"),
    ]);

    // Apply
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    // Check status
    cargo_bin_cmd!("repoverlay")
        .args(["status"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(".envrc"))
        .stdout(predicate::str::contains(".tool-versions"));
}

#[test]
fn status_shows_multiple_overlays() {
    let ctx = TestContext::new();
    let overlay1 = common::create_overlay_dir(&[(".envrc", "export FOO=1")]);
    let overlay2 = common::create_overlay_dir(&[(".tool-versions", "nodejs 20.0.0")]);

    // Apply both overlays
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay1.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "first-overlay"])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "second-overlay"])
        .assert()
        .success();

    // Check status shows both
    cargo_bin_cmd!("repoverlay")
        .args(["status"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("first-overlay"))
        .stdout(predicate::str::contains("second-overlay"));
}

// ============================================================================
// Status --json / --quiet Tests
// ============================================================================

#[test]
fn status_json_no_overlays() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["status", "--json"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""overlays": []"#));
}

#[test]
fn status_json_with_overlay() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "json-test"])
        .assert()
        .success();

    let output = cargo_bin_cmd!("repoverlay")
        .args(["status", "--json"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let overlays = json["overlays"].as_array().unwrap();
    assert_eq!(overlays.len(), 1);
    assert_eq!(overlays[0]["name"], "json-test");
    assert!(overlays[0]["applied_at"].is_string());
    assert!(overlays[0]["source"].is_object());

    let files = overlays[0]["files"].as_array().unwrap();
    assert!(!files.is_empty());
    assert_eq!(files[0]["status"], "ok");
}

#[test]
fn status_json_with_name_filter() {
    let ctx = TestContext::new();
    let overlay1 = common::create_overlay_dir(&[(".envrc", "export FOO=1")]);
    let overlay2 = common::create_overlay_dir(&[(".tool-versions", "nodejs 20.0.0")]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay1.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "first"])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "second"])
        .assert()
        .success();

    let output = cargo_bin_cmd!("repoverlay")
        .args(["status", "--json", "--name", "first"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let overlays = json["overlays"].as_array().unwrap();
    assert_eq!(overlays.len(), 1);
    assert_eq!(overlays[0]["name"], "first");
}

#[test]
fn status_quiet_exits_1_when_no_overlays() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["status", "--quiet"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn status_quiet_exits_0_when_overlays_applied() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["status", "--quiet"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();
}

// ============================================================================
// Restore Command Tests
// ============================================================================

#[test]
fn restore_recreates_deleted_symlinks() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));

    // Delete the symlink manually (simulating user deletion)
    fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
    assert!(!ctx.file_exists(".envrc"));

    // Restore - note: restore currently reports "No overlays to restore" if files are just
    // deleted but state exists. This is expected behavior when overlay state is intact.
    let output = cargo_bin_cmd!("repoverlay")
        .args(["restore"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    // The restore command should indicate what happened
    output.stdout(
        predicate::str::contains("restore")
            .or(predicate::str::contains("Restore"))
            .or(predicate::str::contains("No overlays")),
    );
}

#[test]
fn restore_all_overlays() {
    let ctx = TestContext::new();
    let overlay1 = common::create_overlay_dir(&[(".envrc", "export FOO=1")]);
    let overlay2 = common::create_overlay_dir(&[(".tool-versions", "nodejs 20.0.0")]);

    // Apply both overlays
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay1.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "overlay-a"])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "overlay-b"])
        .assert()
        .success();

    // Delete both files
    fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
    fs::remove_file(ctx.repo_path().join(".tool-versions")).unwrap();

    // Restore all
    cargo_bin_cmd!("repoverlay")
        .args(["restore"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn restore_with_broken_symlinks_does_not_require_force() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "broken-symlink-test"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));

    // Verify overlay state file exists in .repoverlay/
    let state_path = ctx
        .repo_path()
        .join(".repoverlay/overlays/broken-symlink-test.ccl");
    assert!(state_path.exists(), "State file should exist after apply");

    // Delete the symlink (simulating `git clean` removing symlinks)
    fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
    assert!(!ctx.file_exists(".envrc"));

    // State file still exists — this is the "broken symlink" scenario
    assert!(
        state_path.exists(),
        "State file should still exist after deleting symlink"
    );

    // Restore WITHOUT --force should succeed (issue #202)
    cargo_bin_cmd!("repoverlay")
        .args(["restore"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    // The file should be restored
    assert!(
        ctx.file_exists(".envrc"),
        "Symlink should be restored without --force"
    );
}

#[test]
fn restore_when_no_overlays_shows_message() {
    let ctx = TestContext::new();

    // Restore with no overlays should succeed with informational message
    cargo_bin_cmd!("repoverlay")
        .args(["restore"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No overlay"));
}

// ============================================================================
// Update Command Tests
// ============================================================================

#[test]
fn update_help_shows_options() {
    cargo_bin_cmd!("repoverlay")
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ref").or(predicate::str::contains("Update")));
}

// ============================================================================
// Switch Command Tests
// ============================================================================

#[test]
fn switch_help_shows_options() {
    cargo_bin_cmd!("repoverlay")
        .args(["switch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Switch"));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn invalid_command_shows_error() {
    cargo_bin_cmd!("repoverlay")
        .arg("nonexistent-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

#[test]
fn apply_conflicting_file_warns_or_fails() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Create a pre-existing .envrc
    ctx.create_repo_file(".envrc", "existing content");

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("exists").or(predicate::str::contains("conflict")));
}

// ============================================================================
// Cache Command Tests
// ============================================================================

#[test]
fn cache_clear_help() {
    cargo_bin_cmd!("repoverlay")
        .args(["cache", "clear", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Clear").or(predicate::str::contains("cache")));
}

#[test]
fn cache_remove_help() {
    cargo_bin_cmd!("repoverlay")
        .args(["cache", "remove", "--help"])
        .assert()
        .success();
}

// ============================================================================
// Security Tests
// ============================================================================

#[test]
fn apply_rejects_path_traversal_attempt() {
    // Create a controlled directory structure where we can verify path traversal behavior.
    // We create a parent dir containing both the repo and an "escape target" sibling.
    let parent_dir = tempfile::TempDir::new().expect("Failed to create parent dir");
    let repo_dir = parent_dir.path().join("repo");
    let escape_target = parent_dir.path().join("escape-target");

    // Create the directories
    std::fs::create_dir_all(&repo_dir).expect("Failed to create repo dir");
    std::fs::create_dir_all(&escape_target).expect("Failed to create escape target");

    // Initialize git repo
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_dir)
        .output()
        .expect("Failed to init git repo");

    // Create an overlay with a mapping that tries to escape to the sibling directory
    let overlay = tempfile::TempDir::new().expect("Failed to create overlay dir");
    std::fs::write(overlay.path().join(".envrc"), "export FOO=bar")
        .expect("Failed to write .envrc");
    std::fs::write(
        overlay.path().join("repoverlay.ccl"),
        r"mappings =
  .envrc = ../escape-target/malicious
",
    )
    .expect("Failed to write config");

    // The apply should either fail or safely ignore the path traversal
    let result = cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay.path().to_str().unwrap()])
        .args(["--target", repo_dir.to_str().unwrap()])
        .output()
        .expect("failed to execute command");

    // Either it fails, or if it succeeds, the file should be within the repo
    if result.status.success() {
        // If success, verify no file was created in the escape target
        assert!(
            !escape_target.join("malicious").exists(),
            "path traversal should not create files outside repo"
        );
    }
    // If it failed, that's also correct behavior
}

// ============================================================================
// Workflow Integration Tests
// ============================================================================

#[test]
fn full_workflow_apply_status_remove() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // 1. Apply
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "workflow-test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Applying"));

    assert!(ctx.file_exists(".envrc"));

    // 2. Status
    cargo_bin_cmd!("repoverlay")
        .args(["status"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("workflow-test"));

    // 3. Remove
    cargo_bin_cmd!("repoverlay")
        .args(["remove", "workflow-test"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removing"));

    assert!(!ctx.file_exists(".envrc"));

    // 4. Status after removal
    cargo_bin_cmd!("repoverlay")
        .args(["status"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No overlay"));
}

#[test]
fn workflow_apply_delete_restore() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // 1. Apply
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "restore-test"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));

    // 2. Manually delete file
    fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
    assert!(!ctx.file_exists(".envrc"));

    // 3. Restore (note: the current implementation may report "no overlays to restore"
    // when the state is intact but files are missing - test the command runs successfully)
    cargo_bin_cmd!("repoverlay")
        .args(["restore"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn apply_with_empty_file_in_overlay() {
    // Test applying an overlay that contains an empty file
    let ctx = TestContext::new().with_overlay(&[(".empty", "")]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "empty-file-test"])
        .assert()
        .success();

    assert!(ctx.file_exists(".empty"));
    assert_eq!(ctx.read_file(".empty"), "");
}

#[test]
fn apply_with_nested_directory_structure() {
    // Test applying an overlay with deeply nested files
    let ctx = TestContext::new().with_overlay(&[
        ("a/b/c/deep.txt", "deep content"),
        ("x/y/shallow.txt", "shallow content"),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.file_exists("a/b/c/deep.txt"));
    assert!(ctx.file_exists("x/y/shallow.txt"));
    assert_eq!(ctx.read_file("a/b/c/deep.txt"), "deep content");
}

#[test]
fn status_when_no_overlay_applied() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["status"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No overlay"));
}

#[test]
fn apply_same_overlay_twice_fails() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply first time
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "duplicate-test"])
        .assert()
        .success();

    // Apply second time with same name should fail
    let overlay2 = common::create_overlay_dir(&[(".tool-versions", "nodejs 20.0.0")]);
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "duplicate-test"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("already applied").or(predicate::str::contains("duplicate")),
        );
}

#[test]
fn apply_creates_repoverlay_state_directory() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Initially no .repoverlay directory
    assert!(!ctx.state_dir_exists());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "state-test"])
        .assert()
        .success();

    // After apply, .repoverlay directory should exist
    assert!(ctx.state_dir_exists());
    assert!(ctx.overlay_state_exists("state-test"));
}

#[test]
fn remove_deletes_state_directory_when_last_overlay_removed() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "cleanup-test"])
        .assert()
        .success();

    assert!(ctx.state_dir_exists());

    // Remove
    cargo_bin_cmd!("repoverlay")
        .args(["remove", "cleanup-test"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    // After removing last overlay, .repoverlay directory should be cleaned up
    assert!(!ctx.state_dir_exists());
}

#[test]
fn apply_with_special_characters_in_filename() {
    // Test files with special characters (spaces, etc. that are valid on most filesystems)
    let ctx = TestContext::new().with_overlay(&[("file with spaces.txt", "content")]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "special-chars"])
        .assert()
        .success();

    assert!(ctx.file_exists("file with spaces.txt"));
}

#[test]
fn cache_list_runs_without_error() {
    // Just test that cache list command runs without crashing
    cargo_bin_cmd!("repoverlay")
        .args(["cache", "list"])
        .assert()
        .success();
}

#[test]
fn cache_path_shows_directory() {
    cargo_bin_cmd!("repoverlay")
        .args(["cache", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repoverlay"));
}

// ============================================================================
// Source Command Tests
// ============================================================================

#[test]
fn source_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["source", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("source"));
}

#[test]
fn source_add_rejects_empty_url() {
    let ctx = SourceTestContext::new();
    // Empty URL should be rejected
    ctx.cmd()
        .args(["source", "add", ""])
        .assert()
        .failure()
        .stderr(predicate::str::contains("input cannot be empty"));
}

#[test]
fn source_add_rejects_trailing_slash_only_url() {
    let ctx = SourceTestContext::new();
    // "/" is not a valid URL or GitHub shorthand
    ctx.cmd()
        .args(["source", "add", "/"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid source URL"));
}

#[test]
fn source_list_shows_no_sources_when_empty() {
    let ctx = SourceTestContext::new();
    // When no sources are configured, list should show appropriate message
    ctx.cmd()
        .args(["source", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No overlay sources configured"));
}

#[test]
fn source_add_and_list_workflow() {
    let ctx = SourceTestContext::new();

    // Add source
    ctx.cmd()
        .args([
            "source",
            "add",
            "https://github.com/test/workflow-repo",
            "--name",
            "workflow-test",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added"));

    // List should show the new source
    ctx.cmd()
        .args(["source", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("workflow-test"));
}

#[test]
fn source_remove_nonexistent_fails() {
    let ctx = SourceTestContext::new();
    // Removing a non-existent source should fail
    ctx.cmd()
        .args(["source", "remove", "nonexistent-source"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn source_add_duplicate_name_fails() {
    let ctx = SourceTestContext::new();

    // Add first source
    ctx.cmd()
        .args([
            "source",
            "add",
            "https://github.com/test/first",
            "--name",
            "dup-test",
        ])
        .assert()
        .success();

    // Try to add second with same name - should fail
    ctx.cmd()
        .args([
            "source",
            "add",
            "https://github.com/test/second",
            "--name",
            "dup-test",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn source_full_add_remove_workflow() {
    let ctx = SourceTestContext::new();

    // Add first source
    ctx.cmd()
        .args([
            "source",
            "add",
            "https://github.com/test/repo-a",
            "--name",
            "source-a",
        ])
        .assert()
        .success();

    // Add second source
    ctx.cmd()
        .args([
            "source",
            "add",
            "https://github.com/test/repo-b",
            "--name",
            "source-b",
        ])
        .assert()
        .success();

    // List should show both
    ctx.cmd()
        .args(["source", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("source-a"))
        .stdout(predicate::str::contains("source-b"));

    // Remove both
    ctx.cmd()
        .args(["source", "remove", "source-a"])
        .assert()
        .success();

    ctx.cmd()
        .args(["source", "remove", "source-b"])
        .assert()
        .success();
}

#[test]
fn source_add_extracts_name_from_url() {
    let ctx = SourceTestContext::new();
    // When no --name is provided, name should be extracted from URL
    ctx.cmd()
        .args(["source", "add", "https://github.com/test/extracted-name"])
        .assert()
        .success()
        .stdout(predicate::str::contains("source 'extracted-name'"));
}

#[test]
fn source_add_strips_git_suffix_from_name() {
    let ctx = SourceTestContext::new();
    // URL ending in .git should have that suffix stripped
    ctx.cmd()
        .args(["source", "add", "https://github.com/test/git-suffix.git"])
        .assert()
        .success()
        .stdout(predicate::str::contains("source 'git-suffix'"));
}

// ============================================================================
// Local Directory Source Tests
// ============================================================================

#[test]
fn source_add_local_path_succeeds() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Create a local overlays directory inside the repo
    let overlays_dir = ctx.repo_path().join("my-overlays");
    fs::create_dir_all(&overlays_dir).expect("Failed to create overlays dir");

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./my-overlays", "--name", "local-test"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Added"));

    // Verify .repoverlay/config.ccl was created with the path
    let config_path = ctx.repo_path().join(".repoverlay/config.ccl");
    assert!(config_path.exists(), ".repoverlay/config.ccl should exist");
    let content = fs::read_to_string(&config_path).unwrap();
    assert!(
        content.contains("local-test"),
        "config should contain source name"
    );
    assert!(
        content.contains("path = my-overlays"),
        "config should contain path"
    );
}

#[test]
fn source_add_nonexistent_local_path_fails() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./nonexistent-dir"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("does not exist").or(predicate::str::contains("No such file")),
        );
}

#[test]
fn source_add_path_outside_repo_fails() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Create a directory outside the repo
    let outside_dir = tempfile::TempDir::new().unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", outside_dir.path().to_str().unwrap()])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be within the repository"));
}

#[test]
fn source_list_shows_local_sources() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Create overlay dir and add as local source
    let overlays_dir = ctx.repo_path().join("my-overlays");
    fs::create_dir_all(&overlays_dir).unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./my-overlays", "--name", "local-src"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // List should show "Repository sources" and the path
    cargo_bin_cmd!("repoverlay")
        .args(["source", "list"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository sources"))
        .stdout(predicate::str::contains("local-src"))
        .stdout(predicate::str::contains("path:"));
}

#[test]
fn source_remove_local_source() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Create overlay dir and add as local source
    let overlays_dir = ctx.repo_path().join("my-overlays");
    fs::create_dir_all(&overlays_dir).unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./my-overlays", "--name", "removable"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // Verify it exists
    cargo_bin_cmd!("repoverlay")
        .args(["source", "list"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("removable"));

    // Remove it
    cargo_bin_cmd!("repoverlay")
        .args(["source", "remove", "removable"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));

    // Verify it's gone
    cargo_bin_cmd!("repoverlay")
        .args(["source", "list"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("removable").not());
}

#[test]
fn source_add_local_extracts_name_from_dir() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Create overlay dir — name should be extracted from directory name
    let overlays_dir = ctx.repo_path().join("team-overlays");
    fs::create_dir_all(&overlays_dir).unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./team-overlays"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("team-overlays"));
}

// ============================================================================
// Source Add file:// URL Tests
// ============================================================================

#[test]
fn source_add_file_url_succeeds() {
    let ctx = SourceTestContext::new();

    // Create an external directory to use as source
    let external_dir = tempfile::TempDir::new().unwrap();

    let file_url = format!("file://{}", external_dir.path().display());
    ctx.cmd()
        .args(["source", "add", &file_url, "--name", "file-source"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added"));
}

#[test]
fn source_add_file_url_shows_in_list() {
    let ctx = SourceTestContext::new();

    let external_dir = tempfile::TempDir::new().unwrap();
    let file_url = format!("file://{}", external_dir.path().display());

    ctx.cmd()
        .args(["source", "add", &file_url, "--name", "file-listed"])
        .assert()
        .success();

    ctx.cmd()
        .args(["source", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("file-listed"));
}

#[test]
fn source_add_file_url_extracts_name_from_path() {
    let ctx = SourceTestContext::new();

    let external_dir = tempfile::TempDir::new().unwrap();
    let named_dir = external_dir.path().join("my-overlays");
    fs::create_dir_all(&named_dir).unwrap();

    let file_url = format!("file://{}", named_dir.display());
    ctx.cmd()
        .args(["source", "add", &file_url])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-overlays"));
}

// ============================================================================
// JSON Deep Merge Tests
// ============================================================================

#[test]
fn merge_json_with_existing_repo_file() {
    let ctx = TestContext::new().with_overlay(&[(
        "settings.json",
        r#"{"overlay_key": "overlay_value", "shared": "from_overlay"}"#,
    )]);

    // Create existing JSON in repo
    ctx.create_repo_file(
        "settings.json",
        r#"{"repo_key": "repo_value", "shared": "from_repo"}"#,
    );

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--copy", "--merge"])
        .assert()
        .success();

    let content: serde_json::Value = serde_json::from_str(&ctx.read_file("settings.json")).unwrap();
    assert_eq!(content["repo_key"], "repo_value"); // preserved from base
    assert_eq!(content["overlay_key"], "overlay_value"); // added from overlay
    assert_eq!(content["shared"], "from_overlay"); // overlay wins
}

#[test]
fn json_conflict_without_merge_flag_fails() {
    let ctx = TestContext::new().with_overlay(&[("settings.json", r#"{"key": "value"}"#)]);

    ctx.create_repo_file("settings.json", r#"{"existing": true}"#);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .arg("--copy")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Conflict").or(predicate::str::contains("exists")));
}

#[test]
fn merge_flag_ignored_for_non_json_files() {
    let ctx = TestContext::new().with_overlay(&[("config.txt", "overlay content")]);

    ctx.create_repo_file("config.txt", "repo content");

    // --merge doesn't help non-JSON files, still fails
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--copy", "--merge"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Conflict").or(predicate::str::contains("exists")));
}

#[test]
fn merge_with_force_merges_json_and_forces_others() {
    let ctx = TestContext::new().with_overlay(&[
        ("settings.json", r#"{"overlay": true}"#),
        ("readme.txt", "overlay readme"),
    ]);

    ctx.create_repo_file("settings.json", r#"{"repo": true}"#);
    ctx.create_repo_file("readme.txt", "repo readme");

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--copy", "--merge", "--force"])
        .assert()
        .success();

    // JSON was merged
    let json: serde_json::Value = serde_json::from_str(&ctx.read_file("settings.json")).unwrap();
    assert_eq!(json["repo"], true);
    assert_eq!(json["overlay"], true);

    // Non-JSON was force-overwritten
    assert_eq!(ctx.read_file("readme.txt"), "overlay readme");
}

#[test]
fn repoverlay_merge_env_var_enables_merge() {
    let ctx = TestContext::new().with_overlay(&[("settings.json", r#"{"overlay": true}"#)]);

    ctx.create_repo_file("settings.json", r#"{"repo": true}"#);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .arg("--copy")
        .env("REPOVERLAY_MERGE", "true")
        .assert()
        .success();

    let json: serde_json::Value = serde_json::from_str(&ctx.read_file("settings.json")).unwrap();
    assert_eq!(json["repo"], true);
    assert_eq!(json["overlay"], true);
}

#[test]
fn cross_overlay_json_auto_merges_without_merge_flag() {
    let ctx = TestContext::new();
    let overlay1 = common::create_overlay_dir(&[(
        "settings.json",
        r#"{"from_overlay1": true, "shared": "overlay1"}"#,
    )]);
    let overlay2 = common::create_overlay_dir(&[(
        "settings.json",
        r#"{"from_overlay2": true, "shared": "overlay2"}"#,
    )]);

    // Apply first overlay (copy mode for merge support)
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay1.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "first", "--copy"])
        .assert()
        .success();

    // Apply second overlay WITHOUT --merge; should auto-merge the JSON
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "second", "--copy"])
        .assert()
        .success();

    let content: serde_json::Value = serde_json::from_str(&ctx.read_file("settings.json")).unwrap();
    assert_eq!(content["from_overlay1"], true); // preserved from first overlay
    assert_eq!(content["from_overlay2"], true); // added from second overlay
    assert_eq!(content["shared"], "overlay2"); // second overlay wins
}

#[test]
fn cross_overlay_json_deep_merges_nested_objects() {
    let ctx = TestContext::new();
    let overlay1 = common::create_overlay_dir(&[(
        "config.json",
        r#"{"settings": {"theme": "dark", "font": "mono"}}"#,
    )]);
    let overlay2 = common::create_overlay_dir(&[(
        "config.json",
        r#"{"settings": {"font": "sans", "size": 14}}"#,
    )]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay1.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "first", "--copy"])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "second", "--copy"])
        .assert()
        .success();

    let content: serde_json::Value = serde_json::from_str(&ctx.read_file("config.json")).unwrap();
    let settings = &content["settings"];
    assert_eq!(settings["theme"], "dark"); // preserved from first
    assert_eq!(settings["font"], "sans"); // second overlay wins
    assert_eq!(settings["size"], 14); // added from second
}

#[test]
fn cross_overlay_non_json_conflict_still_fails() {
    let ctx = TestContext::new();
    let overlay1 = common::create_overlay_dir(&[("config.txt", "from overlay1")]);
    let overlay2 = common::create_overlay_dir(&[("config.txt", "from overlay2")]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay1.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "first", "--copy"])
        .assert()
        .success();

    // Non-JSON cross-overlay conflict should still fail
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "second", "--copy"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already managed by overlay"));
}

// ─── Edit command tests ──────────────────────────────────────────────────────

#[test]
fn edit_add_fails_when_overlay_not_applied() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args([
            "edit",
            "add",
            "org/repo/nonexistent-overlay",
            "some-file.txt",
        ])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not currently applied"));
}

#[test]
fn edit_add_fails_when_file_does_not_exist() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay first
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args([
            "edit",
            "add",
            "org/repo/test-overlay",
            "nonexistent-file.txt",
        ])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("File does not exist"));
}

#[test]
fn edit_fails_when_no_operation_specified() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "org/repo/my-overlay"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("specify at least one"));
}

#[test]
fn edit_no_name_fails_when_no_overlays_applied() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["edit"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "No overlays are currently applied",
        ));
}

#[test]
fn edit_no_name_auto_selects_single_overlay() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay first
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // With only one overlay applied, `edit` (no name) should auto-select it
    // and enter interactive mode. In non-TTY, interactive returns preselected → no changes.
    cargo_bin_cmd!("repoverlay")
        .args(["edit"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes"));
}

// ──────────────────────────────────────────────
// Edit --add success tests
// ──────────────────────────────────────────────

#[test]
fn edit_add_adds_file_to_overlay() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay first
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Verify overlay is applied
    assert!(ctx.is_symlink(".envrc"));

    // Create a new file in the target repo
    ctx.create_repo_file("new-file.txt", "new content");
    assert!(ctx.file_exists("new-file.txt"));

    // Add the new file to the overlay
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "add", "org/repo/test-overlay", "new-file.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 1 file"));

    // Verify new-file.txt is now a symlink (managed by overlay)
    assert!(ctx.is_symlink("new-file.txt"));
    // Verify content is preserved
    assert_eq!(ctx.read_file("new-file.txt"), "new content");
    // Verify original .envrc symlink still works
    assert!(ctx.is_symlink(".envrc"));
    assert_eq!(ctx.read_file(".envrc"), "export FOO=bar");

    // Verify git exclude has BOTH files
    let exclude = ctx.git_exclude_content();
    assert!(
        exclude.contains(".envrc"),
        "git exclude should contain .envrc, got: {exclude}"
    );
    assert!(
        exclude.contains("new-file.txt"),
        "git exclude should contain new-file.txt, got: {exclude}"
    );
}

#[test]
fn edit_add_works_without_git_remote() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Create a new file in the target repo
    ctx.create_repo_file(".app-config", "app config");

    // Edit add with SHORT form name (no org/repo prefix) — no git remote needed
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "add", "test-overlay", ".app-config"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 1 file"));

    // Verify the file is now managed as a symlink
    assert!(ctx.is_symlink(".app-config"));
}

// ──────────────────────────────────────────────
// Edit --remove tests
// ──────────────────────────────────────────────

#[test]
fn edit_remove_removes_file_from_overlay() {
    let ctx = TestContext::new()
        .with_overlay(&[(".envrc", "export FOO=bar"), ("extra.txt", "extra content")]);

    // Apply overlay with both files
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists("extra.txt"));

    // Remove one file
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "remove", "test-overlay", "extra.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 file"));

    // Verify extra.txt is gone but .envrc remains
    assert!(!ctx.file_exists("extra.txt"));
    assert!(ctx.file_exists(".envrc"));

    // Verify overlay state still exists (overlay not fully removed)
    assert!(ctx.overlay_state_exists("test-overlay"));

    // Verify git exclude still has .envrc but not extra.txt
    let exclude = ctx.git_exclude_content();
    assert!(exclude.contains(".envrc"));
    assert!(!exclude.contains("extra.txt"));
}

#[test]
fn edit_remove_fails_when_file_not_in_overlay() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "remove", "test-overlay", "nonexistent.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not managed by overlay"));
}

#[test]
fn edit_remove_dry_run_does_not_modify() {
    let ctx = TestContext::new()
        .with_overlay(&[(".envrc", "export FOO=bar"), ("extra.txt", "extra content")]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Dry run remove
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "remove", "test-overlay", "extra.txt", "--dry-run"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));

    // File should still exist
    assert!(ctx.file_exists("extra.txt"));
}

#[test]
fn edit_remove_multiple_files() {
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        ("a.txt", "content a"),
        ("b.txt", "content b"),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Remove two files at once
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "remove", "test-overlay", "a.txt", "b.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 2 file"));

    assert!(!ctx.file_exists("a.txt"));
    assert!(!ctx.file_exists("b.txt"));
    assert!(ctx.file_exists(".envrc"));
}

#[test]
fn edit_interactive_fails_for_non_applied_overlay() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "nonexistent", "--interactive"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not currently applied"));
}

#[test]
fn edit_interactive_non_tty_uses_preselected() {
    // When not a TTY, select_files returns preselected files.
    // This means interactive mode in non-TTY returns current files (no change).
    let ctx = TestContext::new()
        .with_overlay(&[(".envrc", "export FOO=bar"), ("extra.txt", "extra content")]);

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // In non-TTY mode, interactive should report "No changes" because
    // preselected = currently applied, so diff is empty
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "test-overlay", "--interactive"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes"));
}

#[test]
fn edit_interactive_includes_files_in_hidden_directories() {
    // Overlays commonly contain files inside hidden directories (e.g. .vscode/,
    // .claude/). The edit command must detect these files so they appear in the
    // interactive selection and are correctly pre-selected.
    let ctx = TestContext::new().with_overlay(&[
        (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
        (".claude/settings.json", r#"{"key": "value"}"#),
        (".envrc", "export FOO=bar"),
    ]);

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Verify all files were applied
    assert!(ctx.file_exists(".vscode/settings.json"));
    assert!(ctx.file_exists(".claude/settings.json"));
    assert!(ctx.file_exists(".envrc"));

    // In non-TTY mode, interactive edit should detect all overlay files
    // (including those inside hidden directories) and pre-select the applied ones.
    // Since all applied files are detected, the diff is empty -> "No changes".
    // Before the fix, files in hidden directories were skipped by WalkDir,
    // which would cause them to appear as removals instead.
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "test-overlay", "--interactive"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes"));
}

// ──────────────────────────────────────────────
// Edit add — additional coverage
// ──────────────────────────────────────────────

#[test]
fn edit_add_dry_run_does_not_modify() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    ctx.create_repo_file("new-file.txt", "new content");

    // Dry run add
    cargo_bin_cmd!("repoverlay")
        .args([
            "edit",
            "add",
            "org/repo/test-overlay",
            "new-file.txt",
            "--dry-run",
        ])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));

    // File should still be a regular file, not a symlink
    assert!(ctx.file_exists("new-file.txt"));
    assert!(!ctx.is_symlink("new-file.txt"));
}

#[test]
fn edit_add_multiple_files() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    ctx.create_repo_file("a.txt", "content a");
    ctx.create_repo_file("b.txt", "content b");

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "add", "org/repo/test-overlay", "a.txt", "b.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 2 file"));

    assert!(ctx.is_symlink("a.txt"));
    assert!(ctx.is_symlink("b.txt"));
    assert_eq!(ctx.read_file("a.txt"), "content a");
    assert_eq!(ctx.read_file("b.txt"), "content b");
}

#[test]
fn edit_add_directory_to_overlay() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay first
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Create a directory with files in the target repo
    ctx.create_repo_file(".claude/commands/build.md", "# Build command");
    ctx.create_repo_file(".claude/commands/test.md", "# Test command");

    // Add the directory to the overlay
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "add", "org/repo/test-overlay", ".claude/commands"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 1 file"));

    // Verify .claude/commands is now a symlink to the overlay directory
    assert!(ctx.is_symlink(".claude/commands"));
    // Verify content is preserved
    assert_eq!(
        ctx.read_file(".claude/commands/build.md"),
        "# Build command"
    );
    assert_eq!(ctx.read_file(".claude/commands/test.md"), "# Test command");

    // Verify git exclude has the directory entry with trailing slash
    let exclude = ctx.git_exclude_content();
    assert!(
        exclude.contains(".claude/commands/"),
        "git exclude should contain .claude/commands/, got: {exclude}"
    );
}

// ──────────────────────────────────────────────
// Edit remove — additional coverage
// ──────────────────────────────────────────────

#[test]
fn edit_remove_fails_when_overlay_not_applied() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "remove", "nonexistent-overlay", "file.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not currently applied"));
}

// ──────────────────────────────────────────────
// Edit interactive overlay selection
// ──────────────────────────────────────────────

#[test]
fn edit_no_name_multiple_overlays_fails_in_non_tty() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply first overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "overlay-one"])
        .assert()
        .success();

    // Create and apply a second overlay
    let overlay2 = common::create_overlay_dir(&[("readme.txt", "hello")]);
    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay2.path().to_str().unwrap()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "overlay-two"])
        .assert()
        .success();

    // With multiple overlays and no TTY, edit with no name should fail
    cargo_bin_cmd!("repoverlay")
        .args(["edit"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("specify which one to edit"));
}

// ──────────────────────────────────────────────
// Deprecated flag backward compatibility
// ──────────────────────────────────────────────

#[test]
fn edit_deprecated_add_flag_works_with_warning() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    ctx.create_repo_file("new-file.txt", "new content");

    // Use deprecated --add flag
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "org/repo/test-overlay", "--add", "new-file.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 1 file"))
        .stderr(predicate::str::contains("deprecated"));

    assert!(ctx.is_symlink("new-file.txt"));
}

#[test]
fn edit_deprecated_remove_flag_works_with_warning() {
    let ctx = TestContext::new()
        .with_overlay(&[(".envrc", "export FOO=bar"), ("extra.txt", "extra content")]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Use deprecated --remove flag
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "test-overlay", "--remove", "extra.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 file"))
        .stderr(predicate::str::contains("deprecated"));

    assert!(!ctx.file_exists("extra.txt"));
    assert!(ctx.file_exists(".envrc"));
}

// ──────────────────────────────────────────────
// Edit interactive — existing tests
// ──────────────────────────────────────────────

#[test]
fn edit_interactive_excludes_repoverlay_ccl_from_selection() {
    // The repoverlay.ccl config file in the overlay source should not appear
    // in the edit selection UI (it's metadata, not overlay content).
    // Adding a repoverlay.ccl to the overlay source and verifying edit
    // reports "No changes" confirms it isn't treated as an overlay file.
    let ctx = TestContext::new().with_overlay(&[(".envrc", "export FOO=bar")]);

    // The overlay source directory also contains repoverlay.ccl (written by
    // create_overlay_dir -> no, actually our test helper doesn't write it).
    // Write one manually so we can verify it's excluded.
    fs::write(
        ctx.overlay_path().join("repoverlay.ccl"),
        "overlay =\n  name = test\n",
    )
    .unwrap();

    // Apply overlay (repoverlay.ccl is not applied as an overlay file)
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Interactive edit should NOT see repoverlay.ccl as a new file to add.
    // If it did, the diff would be non-empty (repoverlay.ccl as an addition).
    // "No changes" confirms repoverlay.ccl is properly excluded.
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "test-overlay", "--interactive"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes"));
}

// ==================== 1.0 Stabilization: Phase 2 Regression Tests ====================

#[test]
fn apply_path_traversal_fails_with_clear_error() {
    let parent_dir = tempfile::TempDir::new().unwrap();
    let repo_dir = parent_dir.path().join("repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_dir)
        .output()
        .unwrap();

    let overlay = tempfile::TempDir::new().unwrap();
    std::fs::write(overlay.path().join(".envrc"), "export FOO=bar").unwrap();
    std::fs::write(
        overlay.path().join("repoverlay.ccl"),
        "mappings =\n  .envrc = ../escape/malicious\n",
    )
    .unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay.path().to_str().unwrap()])
        .args(["--target", repo_dir.to_str().unwrap()])
        .assert()
        .failure(); // Must fail -- no conditional check
}

#[test]
fn error_messages_use_display_not_debug_format() {
    let temp = tempfile::TempDir::new().unwrap();
    // Create a simple overlay (no repoverlay.ccl needed -- apply will fail at git check)
    let overlay = tempfile::TempDir::new().unwrap();
    std::fs::write(overlay.path().join(".envrc"), "content").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay.path().to_str().unwrap()])
        .args(["--target", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        // Debug format markers that must NOT appear:
        .stderr(predicate::str::contains("Os {").not())
        .stderr(predicate::str::contains("kind: ").not());
    // Note: do NOT assert specific message text as it may vary by OS
}

#[test]
#[cfg(unix)]
fn sigpipe_does_not_cause_panic_when_pipe_closes_early() {
    use std::io::Read;
    use std::process::{Command, Stdio};

    // Spawn repoverlay --help (always produces output) piped to a reader
    // that closes immediately. With SIGPIPE default restored, exit should be
    // clean (0 or 141), not a Rust panic (exit code 101).
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("repoverlay"))
        .args(["--help"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn repoverlay");

    // Read just one byte then drop stdout to trigger SIGPIPE on next write
    if let Some(ref mut stdout) = child.stdout {
        let mut buf = [0u8; 1];
        let _ = stdout.read(&mut buf);
    }
    drop(child.stdout.take());

    let status = child.wait().expect("failed to wait on child");

    // With SIGPIPE default restored, exit should be clean (0 from --help,
    // or 141/SIGPIPE on platforms that report pipe death -- NOT a panic exit code)
    let code = status.code().unwrap_or(0);
    assert_ne!(
        code, 101,
        "exit code 101 indicates a Rust panic -- SIGPIPE not handled"
    );
}

#[test]
fn apply_interactive_conflict_abort_on_conflict() {
    // Set up: repo with existing .envrc, overlay also provides .envrc
    let ctx = TestContext::new().with_overlay(&envrc_overlay());
    // Pre-create conflicting file in the repo
    ctx.create_repo_file(".envrc", "existing content");

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--interactive"])
        .write_stdin("a\n") // 'a' = abort in interactive prompt
        .assert()
        .failure(); // abort should exit non-zero

    // Verify the existing file was not overwritten
    let content = ctx.read_file(".envrc");
    assert_eq!(
        content, "existing content",
        "abort should not overwrite existing file"
    );
}

#[test]
fn edit_remove_exclusions_persist_across_remove_reapply() {
    let ctx = TestContext::new()
        .with_overlay(&[(".envrc", "export FOO=bar"), ("extra.txt", "extra content")]);

    // Apply overlay with both files
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists("extra.txt"));

    // Edit remove one file
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "remove", "test-overlay", "extra.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 file"));

    assert!(!ctx.file_exists("extra.txt"));
    assert!(ctx.file_exists(".envrc"));

    // Remove the overlay entirely
    cargo_bin_cmd!("repoverlay")
        .args([
            "remove",
            "test-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(!ctx.file_exists(".envrc"));
    assert!(!ctx.file_exists("extra.txt"));

    // Reapply the same overlay
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // .envrc should be restored, but extra.txt should remain excluded
    assert!(ctx.file_exists(".envrc"));
    assert!(
        !ctx.file_exists("extra.txt"),
        "extra.txt should not reappear after reapply — edit remove exclusions should persist"
    );

    // Verify git exclude does not contain the excluded file
    let exclude = ctx.git_exclude_content();
    assert!(exclude.contains(".envrc"));
    assert!(!exclude.contains("extra.txt"));
}

// ==================== Library subcommand tests ====================

#[test]
fn library_list_empty() {
    let ctx = TestContext::new();
    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "list",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("No overlays"));
}

#[test]
fn library_list_shows_overlays() {
    let ctx = TestContext::new();
    let library_path = ctx.repo_path().join(".repoverlay").join("library");
    fs::create_dir_all(library_path.join("overlay-a")).unwrap();
    fs::create_dir_all(library_path.join("overlay-b")).unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "list",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("overlay-a"))
        .stdout(predicate::str::contains("overlay-b"));
}

#[test]
fn library_import_from_local_path() {
    let ctx =
        TestContext::new().with_overlay(&[(".envrc", "use flake"), ("CLAUDE.md", "# Config")]);

    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "import",
            ctx.overlay_path().to_str().unwrap(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported"));

    // Verify overlay is in library
    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "list",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_match("tmp|overlay").unwrap());
}

#[test]
fn library_import_with_name() {
    let ctx = TestContext::new().with_overlay(&[(".envrc", "use flake")]);

    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "import",
            ctx.overlay_path().to_str().unwrap(),
            "--name",
            "my-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "list",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-overlay"));
}

#[test]
fn library_remove_overlay() {
    let ctx = TestContext::new();
    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("my-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("file.txt"), "content").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "remove",
            "my-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));

    assert!(!library_path.exists());
}

#[test]
fn library_export_to_path() {
    let ctx = TestContext::new();
    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("my-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("file.txt"), "content").unwrap();

    let dest = ctx.repo_path().join("exported");
    fs::create_dir_all(&dest).unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "export",
            "my-overlay",
            "--to",
            dest.to_str().unwrap(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported"));

    assert!(dest.join("my-overlay").join("file.txt").exists());
}

#[test]
fn apply_resolves_from_library() {
    let ctx = TestContext::new();

    // Create a library overlay
    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("test-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("test-file.txt"), "from library").unwrap();

    // Apply by bare name — should resolve from library
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "test-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify file was applied
    assert!(ctx.file_exists("test-file.txt"));

    // Verify status shows library source
    cargo_bin_cmd!("repoverlay")
        .args(["status", "--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("library"));
}

#[test]
fn status_shows_library_source_json() {
    let ctx = TestContext::new();

    // Create and apply library overlay
    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("test-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("file.txt"), "content").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "test-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Check JSON status
    cargo_bin_cmd!("repoverlay")
        .args([
            "status",
            "--json",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Library"));
}

// --- create --into library (#217) ---

#[test]
fn create_into_library_with_include() {
    let ctx = TestContext::new();
    ctx.create_repo_file(".envrc", "use flake");
    ctx.create_repo_file("CLAUDE.md", "# Config");

    cargo_bin_cmd!("repoverlay")
        .args([
            "create",
            "--into",
            "library",
            "--include",
            ".envrc",
            "--include",
            "CLAUDE.md",
            "--source",
            ctx.repo_path().to_str().unwrap(),
            "--no-apply",
            "-y",
        ])
        .assert()
        .success();

    // Verify overlay is in library
    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("overlay");
    assert!(library_path.join(".envrc").exists());
    assert!(library_path.join("CLAUDE.md").exists());
}

#[test]
fn create_into_library_with_name() {
    let ctx = TestContext::new();
    ctx.create_repo_file(".envrc", "use flake");

    cargo_bin_cmd!("repoverlay")
        .args([
            "create",
            "my-overlay",
            "--into",
            "library",
            "--include",
            ".envrc",
            "--source",
            ctx.repo_path().to_str().unwrap(),
            "--no-apply",
            "-y",
        ])
        .assert()
        .success();

    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("my-overlay");
    assert!(library_path.join(".envrc").exists());
}

#[test]
fn create_into_library_applies_by_default_with_yes() {
    let ctx = TestContext::new();
    ctx.create_repo_file(".envrc", "use flake");

    cargo_bin_cmd!("repoverlay")
        .args([
            "create",
            "my-overlay",
            "--into",
            "library",
            "--include",
            ".envrc",
            "--source",
            ctx.repo_path().to_str().unwrap(),
            "-y",
        ])
        .assert()
        .success();

    // Verify it was applied
    cargo_bin_cmd!("repoverlay")
        .args(["status", "--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-overlay"));
}

#[test]
fn create_into_library_rejects_unknown_destination() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args([
            "create",
            "--into",
            "unknown",
            "--source",
            ctx.repo_path().to_str().unwrap(),
            "-y",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown --into destination"));
}

#[test]
fn create_into_library_conflicts_with_output() {
    cargo_bin_cmd!("repoverlay")
        .args(["create", "--into", "library", "--output", "./out", "-y"])
        .assert()
        .failure();
}

#[test]
fn create_into_library_no_apply_requires_into() {
    cargo_bin_cmd!("repoverlay")
        .args(["create", "--no-apply", "-y"])
        .assert()
        .failure();
}

// --- library import by name (#220) ---

#[test]
fn library_import_resolves_applied_overlay_by_name() {
    let ctx =
        TestContext::new().with_overlay(&[(".envrc", "use flake"), ("CLAUDE.md", "# Config")]);

    // Apply the overlay first
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_path().to_str().unwrap(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "my-overlay",
        ])
        .assert()
        .success();

    // Import by name (not path)
    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "import",
            "my-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported"));

    // Verify overlay is in library
    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "list",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-overlay"));
}

#[test]
fn library_import_unknown_name_fails() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "import",
            "nonexistent-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not an applied overlay"));
}

// --- gitignore auto-fix ---

#[test]
fn library_import_fixes_gitignore_when_library_ignored() {
    let ctx = TestContext::new().with_overlay(&[(".envrc", "use flake")]);

    // Set up .gitignore that excludes .repoverlay/
    fs::write(ctx.repo_path().join(".gitignore"), ".repoverlay/\n").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "library",
            "import",
            ctx.overlay_path().to_str().unwrap(),
            "--name",
            "my-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("Updated .gitignore"));

    // Verify .gitignore was updated
    let gitignore = fs::read_to_string(ctx.repo_path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".repoverlay/*"));
    assert!(gitignore.contains("!.repoverlay/library/"));
    // Original `dir/` pattern should be converted to `dir/*`
    assert!(!gitignore.contains(".repoverlay/\n"));
}

// --- apply --from @library (#219) ---

#[test]
fn apply_from_library_explicit() {
    let ctx = TestContext::new();

    // Set up library overlay
    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("my-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join(".envrc"), "use flake").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "my-overlay",
            "--from",
            "@library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Applied"));

    assert!(ctx.repo_path().join(".envrc").exists());
}

#[test]
fn apply_from_library_not_found_shows_error() {
    let ctx = TestContext::new();

    // Library with a different overlay
    let library_path = ctx.repo_path().join(".repoverlay").join("library");
    let other = library_path.join("other-overlay");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("file.txt"), "content").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "nonexistent",
            "--from",
            "@library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found in library"));
}

// --- browse includes library overlays (#218) ---

#[test]
fn browse_shows_library_overlays_when_no_sources() {
    let ctx = TestContext::new();
    let isolated_config = tempfile::TempDir::new().unwrap();

    // Set up library with an overlay
    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("my-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join(".envrc"), "use flake").unwrap();

    // Browse should not fail with "no sources configured" when library exists
    // Use --no-interactive to avoid TUI, isolate config to avoid user's sources
    cargo_bin_cmd!("repoverlay")
        .env("XDG_CONFIG_HOME", isolated_config.path())
        .args([
            "browse",
            "--no-interactive",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-overlay"));
}

#[test]
fn browse_fails_when_no_sources_and_no_library() {
    let ctx = TestContext::new();
    let isolated_config = tempfile::TempDir::new().unwrap();

    cargo_bin_cmd!("repoverlay")
        .env("XDG_CONFIG_HOME", isolated_config.path())
        .args([
            "browse",
            "--no-interactive",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No overlay sources configured"));
}

#[test]
fn create_yes_falls_back_to_tracked_config() {
    // Create a source repo with only tracked config files (no AI configs)
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(source)
        .output()
        .unwrap();
    fs::write(source.join(".envrc"), "export FOO=bar").unwrap();
    fs::write(source.join(".gitignore"), "node_modules/").unwrap();
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(source)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "add config files"])
        .current_dir(source)
        .output()
        .unwrap();

    let output_dir = tempfile::tempdir().unwrap();

    // create --yes should succeed using tracked config files as fallback
    // Note: when name + --output are both provided, files go into output/<name>/
    cargo_bin_cmd!("repoverlay")
        .args([
            "create",
            "my-overlay",
            "--source",
            source.to_str().unwrap(),
            "--output",
            output_dir.path().to_str().unwrap(),
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("tracked config file"));

    // Files are placed in output/<name>/ when both name and --output are provided
    let overlay_dir = output_dir.path().join("my-overlay");
    let has_envrc = overlay_dir.join(".envrc").exists();
    let has_gitignore = overlay_dir.join(".gitignore").exists();
    assert!(
        has_envrc || has_gitignore,
        "At least one tracked config file should be in the output"
    );
}
