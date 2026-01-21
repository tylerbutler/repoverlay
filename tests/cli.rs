//! CLI integration tests using assert_cmd.

mod common;

use assert_cmd::assert::OutputAssertExt;
use common::{repoverlay_cmd, TestContext};
use predicates::prelude::*;

#[test]
fn help_displays() {
    repoverlay_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Overlay config files"));
}

#[test]
fn version_displays() {
    repoverlay_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("repoverlay"));
}

#[test]
fn apply_help_displays() {
    repoverlay_cmd()
        .args(["apply", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Apply an overlay"));
}

#[test]
fn remove_help_displays() {
    repoverlay_cmd()
        .args(["remove", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Remove"));
}

#[test]
fn status_help_displays() {
    repoverlay_cmd()
        .args(["status", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("status"));
}

#[test]
fn cache_help_displays() {
    repoverlay_cmd()
        .args(["cache", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cache"));
}

#[test]
fn restore_help_displays() {
    repoverlay_cmd()
        .args(["restore", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Restore"));
}

#[test]
fn update_help_displays() {
    repoverlay_cmd()
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Update"));
}

#[test]
fn apply_requires_source_argument() {
    repoverlay_cmd()
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn apply_and_remove_workflow() {
    let ctx = TestContext::with_files(&[(".envrc", "export FOO=bar")]);

    // Apply with explicit name
    repoverlay_cmd()
        .args(["apply", ctx.overlay_str()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Applying"));

    assert!(ctx.file_exists(".envrc"));

    // Status
    repoverlay_cmd()
        .args(["status", "--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Overlay Status"));

    // Remove by name
    repoverlay_cmd()
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
    let ctx = TestContext::with_files(&[(".envrc", "export FOO=bar")]);

    // Apply
    repoverlay_cmd()
        .args(["apply", ctx.overlay_str()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));

    // Remove with --all
    repoverlay_cmd()
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
    let ctx = TestContext::with_files(&[(".envrc", "export FOO=bar")]);

    repoverlay_cmd()
        .args(["apply", ctx.overlay_str()])
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

    repoverlay_cmd()
        .args(["status", "--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No overlay"));
}

#[test]
fn remove_when_no_overlay() {
    let ctx = TestContext::new();

    // Use --all to avoid interactive prompt
    repoverlay_cmd()
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
    repoverlay_cmd().args(["cache", "list"]).assert().success();
}

#[test]
fn cache_path_shows_location() {
    repoverlay_cmd()
        .args(["cache", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repoverlay"));
}
