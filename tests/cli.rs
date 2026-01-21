//! CLI integration tests using assert_cmd.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

mod common;
use common::{TestContext, envrc_overlay};

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
        .stdout(predicate::str::contains("status"));
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
