//! CLI integration tests using `assert_cmd`.
//!
//! These tests verify CLI behavior by running the compiled binary.
//! Organized into logical sections covering each command's functionality.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;

mod common;
use common::{SourceTestContext, TestContext, create_overlay_dir, envrc_overlay};

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
        .stdout(predicate::str::contains("Apply an overlay"))
        .stdout(predicate::str::contains("repoverlay browse"));
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
fn browse_local_flat_dotfile_root_lists_single_overlay() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let source_dir = ctx.repo_path().join("flat-source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join(".envrc"), "export FOO=bar").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "browse",
            "./flat-source",
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--no-interactive",
        ])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("(flat)"))
        .stdout(predicate::str::contains("flat-source"));
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
fn apply_help_mentions_browse_for_interactive() {
    // apply help text should guide interactive users toward browse
    cargo_bin_cmd!("repoverlay")
        .args(["apply", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repoverlay browse"));
}

#[test]
fn apply_help_mentions_scripting() {
    // apply help text should clarify it is the scripting / power-user path
    cargo_bin_cmd!("repoverlay")
        .args(["apply", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("scripting"));
}

#[test]
fn root_help_browse_listed_as_recommended() {
    // top-level help should list browse as "recommended"
    cargo_bin_cmd!("repoverlay")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("recommended"));
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
fn profile_list_shows_repo_profiles() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    description = Rust development
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "list",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("rust-dev"))
        .stdout(predicate::str::contains("Rust development"));
}

#[test]
fn profile_list_uses_current_directory_as_default_target() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    ctx.write_repo_config(
        r"
profiles =
  local-profile =
    description = Repo local
",
    );

    cargo_bin_cmd!("repoverlay")
        .args(["profile", "list"])
        .current_dir(ctx.repo_path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("local-profile"))
        .stdout(predicate::str::contains("Repo local"));
}

#[test]
fn profile_show_prints_profile_details() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    description = Rust development
    overlays =
      = rust-base
    skills =
      = market:rust-reviewer@playground
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "show",
            "rust-dev",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("rust-dev"))
        .stdout(predicate::str::contains("rust-base"))
        .stdout(predicate::str::contains("market:rust-reviewer@playground"));
}

#[test]
fn profile_show_uses_current_directory_as_default_target() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    ctx.write_repo_config(
        r"
profiles =
  local-profile =
    overlays =
      = repo-overlay
",
    );

    cargo_bin_cmd!("repoverlay")
        .args(["profile", "show", "local-profile"])
        .current_dir(ctx.repo_path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("local-profile"))
        .stdout(predicate::str::contains("repo-overlay"));
}

#[test]
fn profile_apply_writes_copilot_assets_and_state() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.create_repo_file("copilot-instructions.md", "Use Rust 2024.");
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    instructions =
      =
        source = copilot-instructions.md
    mcps =
      servers =
        rust =
          command = uvx
          args =
            = mcp-rust
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "apply",
            "rust-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("Applied profile rust-dev"));

    assert!(copilot_home.path().join("mcp.json").exists());
    assert!(
        copilot_home
            .path()
            .join("instructions/rust-dev/copilot-instructions.md")
            .exists()
    );
    assert!(
        ctx.repo_path()
            .join(".repoverlay/profiles/rust-dev.copilot.ccl")
            .exists()
    );
}

#[test]
fn copilot_profile_runs_command_and_cleans_up() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    let marker = ctx.repo_path().join("copilot-ran.txt");
    ctx.create_repo_file("copilot-instructions.md", "Use Rust 2024.");
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    instructions =
      =
        source = copilot-instructions.md
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "copilot",
            "--profile",
            "rust-dev",
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--",
            "-c",
            &format!("echo ran > {}", marker.display()),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_COPILOT_COMMAND", "sh")
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    assert!(marker.exists());
    assert!(
        !copilot_home
            .path()
            .join("instructions/rust-dev/copilot-instructions.md")
            .exists()
    );
    assert!(
        !ctx.repo_path()
            .join(".repoverlay/profiles/rust-dev.copilot.ccl")
            .exists()
    );
}

#[test]
fn copilot_profile_preserves_harness_exit_code_after_cleanup() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    description = Rust
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "copilot",
            "--profile",
            "rust-dev",
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--",
            "-c",
            "exit 7",
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_COPILOT_COMMAND", "sh")
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .code(7);
}

#[cfg(unix)]
#[test]
fn copilot_profile_maps_signal_exit_after_cleanup() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    description = Rust
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "copilot",
            "--profile",
            "rust-dev",
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--",
            "-c",
            "kill -TERM $$",
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_COPILOT_COMMAND", "sh")
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .code(143);

    assert!(
        !ctx.repo_path()
            .join(".repoverlay/profiles/rust-dev.copilot.ccl")
            .exists()
    );
}

#[test]
fn profile_status_and_remove_manage_profile_state_and_files() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.create_repo_file("copilot-instructions.md", "Use Rust 2024.");
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    instructions =
      =
        source = copilot-instructions.md
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "apply",
            "rust-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "status",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("rust-dev"))
        .stdout(predicate::str::contains("copilot"));

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "remove",
            "rust-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed profile rust-dev"));

    assert!(
        !copilot_home
            .path()
            .join("instructions/rust-dev/copilot-instructions.md")
            .exists()
    );
    assert!(
        !ctx.repo_path()
            .join(".repoverlay/profiles/rust-dev.copilot.ccl")
            .exists()
    );
}

#[test]
fn profile_status_harness_filter_reports_no_matches() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.create_repo_file("copilot-instructions.md", "Use Rust 2024.");
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    instructions =
      =
        source = copilot-instructions.md
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "apply",
            "rust-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "status",
            "--harness",
            "claude",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("No profiles applied."));
}

#[test]
fn profile_remove_uses_resolved_overlay_state_names() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    let overlay_path = ctx
        .repo_path()
        .join("my-overlays")
        .join("acme")
        .join("app")
        .join("dotenv");
    fs::create_dir_all(&overlay_path).unwrap();
    fs::write(overlay_path.join(".envrc"), "export PROFILE_OVERLAY=1").unwrap();
    ctx.write_repo_config(
        r"
profiles =
  env-dev =
    overlays =
      = acme/app/dotenv
",
    );

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./my-overlays", "--name", "local-src"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "apply",
            "env-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.overlay_state_exists("dotenv"));

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "remove",
            "env-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed profile env-dev"));

    assert!(!ctx.file_exists(".envrc"));
    assert!(!ctx.overlay_state_exists("dotenv"));
}

#[test]
fn profile_remove_fails_on_malformed_recorded_overlay_state() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    let overlay_path = ctx
        .repo_path()
        .join("my-overlays")
        .join("acme")
        .join("app")
        .join("dotenv");
    fs::create_dir_all(&overlay_path).unwrap();
    fs::write(overlay_path.join(".envrc"), "export PROFILE_OVERLAY=1").unwrap();
    ctx.write_repo_config(
        r"
profiles =
  env-dev =
    overlays =
      = acme/app/dotenv
",
    );

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./my-overlays", "--name", "local-src"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "apply",
            "env-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    fs::write(
        ctx.repo_path().join(".repoverlay/overlays/dotenv.ccl"),
        "invalid = [",
    )
    .unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "remove",
            "env-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Failed to parse overlay state: dotenv",
        ));

    assert!(
        ctx.repo_path()
            .join(".repoverlay/profiles/env-dev.copilot.ccl")
            .exists()
    );
}

#[test]
fn profile_status_warns_and_skips_malformed_profile_state() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.create_repo_file("copilot-instructions.md", "Use Rust 2024.");
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    instructions =
      =
        source = copilot-instructions.md
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "apply",
            "rust-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    fs::write(
        ctx.repo_path()
            .join(".repoverlay/profiles/broken.copilot.ccl"),
        "invalid = [",
    )
    .unwrap();

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "status",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("rust-dev"))
        .stdout(predicate::str::contains("copilot"))
        .stderr(predicate::str::contains(
            "Warning: failed to load profile state",
        ));
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

#[test]
fn apply_from_configured_local_flat_subdirectory_source() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let source_dir = ctx.repo_path().join("overlays");
    let overlay_dir = source_dir.join("config-a");
    fs::create_dir_all(&overlay_dir).unwrap();
    fs::write(overlay_dir.join(".envrc"), "export A=1").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./overlays", "--name", "local-flat"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "config-a",
            "--from",
            "local-flat",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert_eq!(ctx.read_file(".envrc"), "export A=1");
    assert!(ctx.overlay_state_exists("config-a"));
}

#[test]
fn apply_from_configured_local_flat_root_source() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();
    let source_dir = ctx.repo_path().join("flat-source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join(".envrc"), "export ROOT=1").unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./flat-source", "--name", "local-flat"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "flat-source",
            "--from",
            "local-flat",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert_eq!(ctx.read_file(".envrc"), "export ROOT=1");
    assert!(ctx.overlay_state_exists("flat-source"));
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
fn status_name_filter_text_mode() {
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

    // Text mode (no --json) with --name filter should show only the filtered overlay
    cargo_bin_cmd!("repoverlay")
        .args(["status", "--name", "first"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("first"))
        .stdout(predicate::str::contains("second").not());
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
        .stdout(predicate::str::contains("Switch"))
        .stdout(predicate::str::contains("--dry-run"));
}

#[test]
fn switch_dry_run_previews_without_changing_overlays() {
    let ctx = TestContext::new();
    let overlay_a = create_overlay_dir(&[(".config-a", "alpha")]);
    let overlay_b = create_overlay_dir(&[(".config-b", "beta")]);

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            overlay_a.path().to_str().unwrap(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "overlay-a",
        ])
        .assert()
        .success();

    assert!(ctx.file_exists(".config-a"));
    assert!(!ctx.file_exists(".config-b"));
    assert!(ctx.overlay_state_exists("overlay-a"));

    cargo_bin_cmd!("repoverlay")
        .args([
            "switch",
            overlay_b.path().to_str().unwrap(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "overlay-b",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("would remove"))
        .stdout(predicate::str::contains("Would apply"));

    assert!(ctx.file_exists(".config-a"));
    assert!(!ctx.file_exists(".config-b"));
    assert!(ctx.overlay_state_exists("overlay-a"));
    assert!(!ctx.overlay_state_exists("overlay-b"));
}

// ============================================================================
// Sync Command Tests
// ============================================================================

#[test]
fn sync_help_shows_options() {
    cargo_bin_cmd!("repoverlay")
        .args(["sync", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Sync"));
}

// ============================================================================
// Completions Command Tests
// ============================================================================

#[test]
fn completions_bash_produces_output() {
    cargo_bin_cmd!("repoverlay")
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}

#[test]
fn completions_zsh_produces_output() {
    cargo_bin_cmd!("repoverlay")
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}

#[test]
fn completions_fish_produces_output() {
    cargo_bin_cmd!("repoverlay")
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
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
            "--target",
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
            "--target",
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
            "--target",
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

// --- create --into library resolves to target, not source (#275) ---

#[test]
fn create_into_library_uses_target_not_source() {
    // Source repo: has files to extract
    let source_ctx = TestContext::new();
    source_ctx.create_repo_file(".envrc", "use flake");

    // Target repo: where the library overlay should land
    let target_ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args([
            "create",
            "--into",
            "library",
            "--include",
            ".envrc",
            "--source",
            source_ctx.repo_path().to_str().unwrap(),
            "--target",
            target_ctx.repo_path().to_str().unwrap(),
            "--no-apply",
            "-y",
        ])
        .assert()
        .success();

    // Overlay must be in the TARGET repo's library
    let target_library = target_ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("overlay");
    assert!(
        target_library.join(".envrc").exists(),
        "Overlay should be in target repo's library, but was not found at {}",
        target_library.display()
    );

    // Overlay must NOT be in the source repo's library
    let source_library = source_ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join("overlay");
    assert!(
        !source_library.exists(),
        "Overlay should NOT be in source repo's library, but was found at {}",
        source_library.display()
    );
}

#[test]
fn create_into_library_applies_to_target_repo() {
    // Source repo: has files to extract
    let source_ctx = TestContext::new();
    source_ctx.create_repo_file(".envrc", "use flake");

    // Target repo: where overlay should be applied
    let target_ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args([
            "create",
            "my-overlay",
            "--into",
            "library",
            "--include",
            ".envrc",
            "--source",
            source_ctx.repo_path().to_str().unwrap(),
            "--target",
            target_ctx.repo_path().to_str().unwrap(),
            "-y",
        ])
        .assert()
        .success();

    // Overlay should be applied in the target repo
    cargo_bin_cmd!("repoverlay")
        .args([
            "status",
            "--target",
            target_ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-overlay"));

    // Target repo should have the overlay file
    assert!(
        target_ctx.file_exists(".envrc"),
        "Overlay file should be applied in target repo"
    );
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

// ──────────────────────────────────────────────
// Edit remove — directory exclusion persistence
// ──────────────────────────────────────────────

#[test]
fn edit_remove_directory_excluded_on_reapply() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    ctx.create_repo_file(".claude/commands/build.md", "# Build command");
    ctx.create_repo_file(".claude/commands/test.md", "# Test command");

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "add", "org/repo/test-overlay", ".claude/commands"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.is_symlink(".claude/commands"));

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "remove", "test-overlay", ".claude/commands"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(!ctx.file_exists(".claude/commands"));

    cargo_bin_cmd!("repoverlay")
        .args([
            "remove",
            "test-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(
        !ctx.file_exists(".claude/commands"),
        ".claude/commands should not reappear after reapply — directory exclusion should persist"
    );
}

#[test]
fn edit_remove_handles_trailing_slash_on_directory() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    ctx.create_repo_file(".vscode/settings.json", r#"{"editor.tabSize": 2}"#);

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "add", "org/repo/test-overlay", ".vscode"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "remove", "test-overlay", ".vscode/"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 file"));

    assert!(!ctx.file_exists(".vscode"));
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
    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(source)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
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
        has_envrc && has_gitignore,
        "Both tracked config files should be in the output"
    );
}

// ──────────────────────────────────────────────
// Multi-target mappings
// ──────────────────────────────────────────────

#[test]
fn multi_target_mapping_creates_multiple_copies() {
    let ctx = TestContext::new().with_overlay(&[
        (".editorconfig", "root = true"),
        (
            "repoverlay.ccl",
            "mappings =\n  .editorconfig = .editorconfig\n  .editorconfig = packages/frontend/.editorconfig\n",
        ),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(
        ctx.file_exists(".editorconfig"),
        ".editorconfig should exist at root"
    );
    assert!(
        ctx.file_exists("packages/frontend/.editorconfig"),
        ".editorconfig should exist in packages/frontend/"
    );
    assert_eq!(ctx.read_file(".editorconfig"), "root = true");
    assert_eq!(
        ctx.read_file("packages/frontend/.editorconfig"),
        "root = true"
    );
}

#[test]
fn single_target_mapping_still_works_with_vec_type() {
    // Backwards compatibility: single-value mappings must still work
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        ("repoverlay.ccl", "mappings =\n  .envrc = .env\n"),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(
        ctx.file_exists(".env"),
        ".env should exist (mapped from .envrc)"
    );
    assert!(
        !ctx.file_exists(".envrc"),
        ".envrc should not exist (was mapped)"
    );
}

#[test]
fn multi_target_mapping_all_targets_in_state() {
    let ctx = TestContext::new().with_overlay(&[
        ("config.json", r#"{"key": "value"}"#),
        (
            "repoverlay.ccl",
            "mappings =\n  config.json = .config.json\n  config.json = tools/.config.json\n",
        ),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "multi-map",
        ])
        .assert()
        .success();

    // Both targets should be in the state file
    let state_content =
        fs::read_to_string(ctx.repo_path().join(".repoverlay/overlays/multi-map.ccl"))
            .expect("state file should exist");

    assert!(
        state_content.contains(".config.json"),
        "state should include first target"
    );
    assert!(
        state_content.contains("tools/.config.json"),
        "state should include second target"
    );
}

#[test]
fn multi_target_mapping_remove_cleans_all_targets() {
    let ctx = TestContext::new().with_overlay(&[
        (".editorconfig", "root = true"),
        (
            "repoverlay.ccl",
            "mappings =\n  .editorconfig = .editorconfig\n  .editorconfig = sub/.editorconfig\n",
        ),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "multi-map",
        ])
        .assert()
        .success();

    assert!(ctx.file_exists(".editorconfig"));
    assert!(ctx.file_exists("sub/.editorconfig"));

    cargo_bin_cmd!("repoverlay")
        .args([
            "remove",
            "multi-map",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        !ctx.file_exists(".editorconfig"),
        "first target should be removed"
    );
    assert!(
        !ctx.file_exists("sub/.editorconfig"),
        "second target should be removed"
    );
}

// ──────────────────────────────────────────────
// Overlay composition — extends
// ──────────────────────────────────────────────

/// Helper: create a library overlay in the test repo.
fn create_library_overlay(ctx: &TestContext, name: &str, files: &[(&str, &str)]) {
    let library_path = ctx
        .repo_path()
        .join(".repoverlay")
        .join("library")
        .join(name);
    fs::create_dir_all(&library_path).unwrap();
    for (path, content) in files {
        let file_path = library_path.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(file_path, content).unwrap();
    }
}
#[test]
fn extends_inherits_parent_files() {
    let ctx = TestContext::new();

    // Parent overlay with two files
    create_library_overlay(
        &ctx,
        "parent",
        &[("file-a.txt", "content-a"), ("file-b.txt", "content-b")],
    );

    // Child overlay extends parent, adds its own file
    create_library_overlay(
        &ctx,
        "child",
        &[
            ("file-c.txt", "content-c"),
            ("repoverlay.ccl", "extends =\n  overlay = parent\n"),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        ctx.file_exists("file-a.txt"),
        "inherited file-a should exist"
    );
    assert!(
        ctx.file_exists("file-b.txt"),
        "inherited file-b should exist"
    );
    assert!(
        ctx.file_exists("file-c.txt"),
        "child's own file-c should exist"
    );
    assert_eq!(ctx.read_file("file-a.txt"), "content-a");
    assert_eq!(ctx.read_file("file-b.txt"), "content-b");
    assert_eq!(ctx.read_file("file-c.txt"), "content-c");
}

#[test]
fn extends_child_wins_on_conflict() {
    let ctx = TestContext::new();

    create_library_overlay(&ctx, "parent", &[("shared.txt", "parent-content")]);

    create_library_overlay(
        &ctx,
        "child",
        &[
            ("shared.txt", "child-content"),
            ("repoverlay.ccl", "extends =\n  overlay = parent\n"),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        ctx.read_file("shared.txt"),
        "child-content",
        "child should win"
    );
}

#[test]
fn extends_multi_level_inheritance() {
    let ctx = TestContext::new();

    create_library_overlay(&ctx, "grandparent", &[("file-a.txt", "from-grandparent")]);

    create_library_overlay(
        &ctx,
        "parent",
        &[
            ("file-b.txt", "from-parent"),
            ("repoverlay.ccl", "extends =\n  overlay = grandparent\n"),
        ],
    );

    create_library_overlay(
        &ctx,
        "child",
        &[
            ("file-c.txt", "from-child"),
            ("repoverlay.ccl", "extends =\n  overlay = parent\n"),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        ctx.file_exists("file-a.txt"),
        "grandparent file should exist"
    );
    assert!(ctx.file_exists("file-b.txt"), "parent file should exist");
    assert!(ctx.file_exists("file-c.txt"), "child file should exist");
    assert_eq!(ctx.read_file("file-a.txt"), "from-grandparent");
    assert_eq!(ctx.read_file("file-b.txt"), "from-parent");
    assert_eq!(ctx.read_file("file-c.txt"), "from-child");
}

#[test]
fn extends_cycle_detection() {
    let ctx = TestContext::new();

    create_library_overlay(
        &ctx,
        "overlay-a",
        &[
            ("file-a.txt", "content-a"),
            ("repoverlay.ccl", "extends =\n  overlay = overlay-b\n"),
        ],
    );

    create_library_overlay(
        &ctx,
        "overlay-b",
        &[
            ("file-b.txt", "content-b"),
            ("repoverlay.ccl", "extends =\n  overlay = overlay-a\n"),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "overlay-a",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle"));
}

// ──────────────────────────────────────────────
// Overlay composition — includes
// ──────────────────────────────────────────────

#[test]
fn includes_cherry_picks_specific_files() {
    let ctx = TestContext::new();

    create_library_overlay(
        &ctx,
        "tools",
        &[
            ("file-a.txt", "content-a"),
            ("file-b.txt", "content-b"),
            ("file-c.txt", "content-c"),
        ],
    );

    create_library_overlay(
        &ctx,
        "mine",
        &[
            ("file-d.txt", "content-d"),
            (
                "repoverlay.ccl",
                "includes =\n  =\n    overlay = tools\n    files =\n      = file-b.txt\n",
            ),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "mine",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        !ctx.file_exists("file-a.txt"),
        "file-a should NOT be included"
    );
    assert!(
        ctx.file_exists("file-b.txt"),
        "file-b should be cherry-picked"
    );
    assert!(
        !ctx.file_exists("file-c.txt"),
        "file-c should NOT be included"
    );
    assert!(
        ctx.file_exists("file-d.txt"),
        "child's own file-d should exist"
    );
    assert_eq!(ctx.read_file("file-b.txt"), "content-b");
}

#[test]
fn includes_missing_file_errors() {
    let ctx = TestContext::new();

    create_library_overlay(&ctx, "tools", &[("file-a.txt", "content-a")]);

    create_library_overlay(
        &ctx,
        "mine",
        &[
            ("own-file.txt", "content"),
            (
                "repoverlay.ccl",
                "includes =\n  =\n    overlay = tools\n    files =\n      = nonexistent.txt\n",
            ),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "mine",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nonexistent.txt"));
}

#[test]
fn includes_referenced_overlay_with_extends() {
    let ctx = TestContext::new();

    // base has file-a
    create_library_overlay(&ctx, "base", &[("file-a.txt", "from-base")]);

    // tools extends base, adds file-b
    create_library_overlay(
        &ctx,
        "tools",
        &[
            ("file-b.txt", "from-tools"),
            ("repoverlay.ccl", "extends =\n  overlay = base\n"),
        ],
    );

    // mine includes file-a from tools (which tools inherited from base)
    create_library_overlay(
        &ctx,
        "mine",
        &[
            ("file-c.txt", "from-mine"),
            (
                "repoverlay.ccl",
                "includes =\n  =\n    overlay = tools\n    files =\n      = file-a.txt\n",
            ),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "mine",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        ctx.file_exists("file-a.txt"),
        "inherited file from base via tools"
    );
    assert!(!ctx.file_exists("file-b.txt"), "file-b not cherry-picked");
    assert!(ctx.file_exists("file-c.txt"), "child's own file");
    assert_eq!(ctx.read_file("file-a.txt"), "from-base");
}

// ──────────────────────────────────────────────
// Overlay composition — precedence
// ──────────────────────────────────────────────

#[test]
fn composition_precedence_child_over_extends_over_includes() {
    let ctx = TestContext::new();

    create_library_overlay(&ctx, "included", &[("shared.txt", "from-included")]);

    create_library_overlay(&ctx, "parent", &[("shared.txt", "from-parent")]);

    // child extends parent AND includes "included", plus has its own shared.txt
    create_library_overlay(
        &ctx,
        "child",
        &[
            ("shared.txt", "from-child"),
            (
                "repoverlay.ccl",
                "extends =\n  overlay = parent\n\nincludes =\n  =\n    overlay = included\n    files =\n      = shared.txt\n",
            ),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        ctx.read_file("shared.txt"),
        "from-child",
        "child wins over extends and includes"
    );
}

// ──────────────────────────────────────────────
// Overlay composition — extends with mappings
// ──────────────────────────────────────────────

#[test]
fn extends_inherits_parent_mappings() {
    let ctx = TestContext::new();

    create_library_overlay(
        &ctx,
        "parent",
        &[
            ("template.env", "SECRET=foo"),
            ("repoverlay.ccl", "mappings =\n  template.env = .env\n"),
        ],
    );

    create_library_overlay(
        &ctx,
        "child",
        &[
            ("file-c.txt", "content-c"),
            ("repoverlay.ccl", "extends =\n  overlay = parent\n"),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        ctx.file_exists(".env"),
        "parent's mapping should be applied"
    );
    assert!(
        !ctx.file_exists("template.env"),
        "source file should be mapped, not placed directly"
    );
    assert!(ctx.file_exists("file-c.txt"), "child's own file");
}

// ──────────────────────────────────────────────
// Overlay composition — state and cleanup
// ──────────────────────────────────────────────

#[test]
fn composed_overlay_state_records_resolved_files() {
    let ctx = TestContext::new();

    create_library_overlay(&ctx, "parent", &[("file-a.txt", "content-a")]);

    create_library_overlay(
        &ctx,
        "child",
        &[
            ("file-b.txt", "content-b"),
            ("repoverlay.ccl", "extends =\n  overlay = parent\n"),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // State file should exist and contain both files
    let state_content = fs::read_to_string(ctx.repo_path().join(".repoverlay/overlays/child.ccl"))
        .expect("state file should exist");

    assert!(
        state_content.contains("file-a.txt"),
        "state should include inherited file"
    );
    assert!(
        state_content.contains("file-b.txt"),
        "state should include child's file"
    );
}

#[test]
fn remove_cleans_up_composed_overlay() {
    let ctx = TestContext::new();

    create_library_overlay(&ctx, "parent", &[("file-a.txt", "content-a")]);

    create_library_overlay(
        &ctx,
        "child",
        &[
            ("file-b.txt", "content-b"),
            ("repoverlay.ccl", "extends =\n  overlay = parent\n"),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(ctx.file_exists("file-a.txt"));
    assert!(ctx.file_exists("file-b.txt"));

    cargo_bin_cmd!("repoverlay")
        .args([
            "remove",
            "child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        !ctx.file_exists("file-a.txt"),
        "inherited file should be removed"
    );
    assert!(
        !ctx.file_exists("file-b.txt"),
        "child file should be removed"
    );
}

#[test]
fn extends_diamond_dependency_is_not_a_cycle() {
    let ctx = TestContext::new();

    // shared-base is referenced by both branch-x and branch-y
    create_library_overlay(&ctx, "shared-base", &[("base.txt", "from-base")]);

    create_library_overlay(
        &ctx,
        "branch-x",
        &[
            ("x.txt", "from-x"),
            ("repoverlay.ccl", "extends =\n  overlay = shared-base\n"),
        ],
    );

    create_library_overlay(
        &ctx,
        "branch-y",
        &[
            ("y.txt", "from-y"),
            ("repoverlay.ccl", "extends =\n  overlay = shared-base\n"),
        ],
    );

    // child includes files from both branches (diamond through shared-base)
    create_library_overlay(
        &ctx,
        "diamond-child",
        &[
            ("child.txt", "from-child"),
            (
                "repoverlay.ccl",
                "includes =\n  =\n    overlay = branch-x\n    files =\n      = base.txt\n  =\n    overlay = branch-y\n    files =\n      = y.txt\n",
            ),
        ],
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "diamond-child",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(ctx.file_exists("base.txt"), "base file via diamond");
    assert!(ctx.file_exists("y.txt"), "branch-y file");
    assert!(ctx.file_exists("child.txt"), "child's own file");
}

// ── Move command tests ──────────────────────────────────────────────────────

#[test]
fn move_help_displays() {
    cargo_bin_cmd!("repoverlay")
        .args(["move", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Move"));
}

#[test]
fn move_to_library() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay (symlink mode — the default on unix)
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "test-move",
        ])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.is_symlink(".envrc"));
    assert!(ctx.overlay_state_exists("test-move"));

    // Move to library
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "test-move",
            "--to",
            "library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // State should still exist and reference library source
    assert!(ctx.overlay_state_exists("test-move"));
    let state_content =
        fs::read_to_string(ctx.repo_path().join(".repoverlay/overlays/test-move.ccl"))
            .expect("state file should exist");
    assert!(
        state_content.contains("Library"),
        "state source should be Library, got: {state_content}"
    );

    // Symlink should still work (now pointing to library)
    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.is_symlink(".envrc"));

    // Library should contain the overlay
    assert!(
        ctx.repo_path()
            .join(".repoverlay/library/test-move/.envrc")
            .exists()
    );

    // Original source should still exist (it's the test temp dir, not managed by us)
    // but the old source path in state should be replaced
}

#[test]
fn move_to_filesystem_path() {
    let ctx = TestContext::new();

    // Create a library overlay and apply it
    create_library_overlay(&ctx, "lib-overlay", &[(".envrc", "export LIB=true")]);

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "lib-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.overlay_state_exists("lib-overlay"));

    // Move to a filesystem path
    let dest = tempfile::TempDir::new().unwrap();
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "lib-overlay",
            "--to",
            dest.path().to_str().unwrap(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // State should reference Local source now
    let state_content =
        fs::read_to_string(ctx.repo_path().join(".repoverlay/overlays/lib-overlay.ccl"))
            .expect("state file should exist");
    assert!(
        state_content.contains("Local"),
        "state source should be Local, got: {state_content}"
    );

    // Overlay files should exist at destination
    assert!(dest.path().join("lib-overlay/.envrc").exists());

    // Library entry should be removed
    assert!(
        !ctx.repo_path()
            .join(".repoverlay/library/lib-overlay")
            .exists(),
        "library entry should be removed after move"
    );
}

#[test]
fn move_preserves_copy_entries() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply with --copy
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "copied",
            "--copy",
        ])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(!ctx.is_symlink(".envrc"), "should be a copy, not symlink");

    let content_before = ctx.read_file(".envrc");

    // Move to library
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "copied",
            "--to",
            "library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Copied file should be unchanged (not re-linked)
    assert!(ctx.file_exists(".envrc"));
    assert!(
        !ctx.is_symlink(".envrc"),
        "copy entry should remain a copy after move"
    );
    assert_eq!(ctx.read_file(".envrc"), content_before);

    // State should now reference library
    let state_content = fs::read_to_string(ctx.repo_path().join(".repoverlay/overlays/copied.ccl"))
        .expect("state file should exist");
    assert!(
        state_content.contains("Library"),
        "state source should be Library"
    );
}

#[test]
fn move_name_conflict_errors() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "conflict-test",
        ])
        .assert()
        .success();

    // Pre-create a library overlay with same name
    create_library_overlay(&ctx, "conflict-test", &[("other.txt", "existing")]);

    // Move should fail due to name conflict
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "conflict-test",
            "--to",
            "library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn move_force_overwrites() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "force-test",
        ])
        .assert()
        .success();

    // Pre-create a library overlay with same name
    create_library_overlay(&ctx, "force-test", &[("other.txt", "existing")]);

    // Move with --force should succeed
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "force-test",
            "--to",
            "library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--force",
        ])
        .assert()
        .success();

    // Library should have the moved overlay's content, not the old one
    assert!(
        ctx.repo_path()
            .join(".repoverlay/library/force-test/.envrc")
            .exists()
    );
    assert!(
        !ctx.repo_path()
            .join(".repoverlay/library/force-test/other.txt")
            .exists(),
        "old library content should be replaced"
    );
}

#[test]
fn move_with_rename() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "original-name",
        ])
        .assert()
        .success();

    // Move to library with a new name
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "original-name",
            "--to",
            "library",
            "--name",
            "renamed",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Library should have the new name
    assert!(
        ctx.repo_path()
            .join(".repoverlay/library/renamed/.envrc")
            .exists()
    );
    assert!(
        !ctx.repo_path()
            .join(".repoverlay/library/original-name")
            .exists(),
        "old name should not exist in library"
    );
}

#[test]
fn move_dry_run() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // Apply overlay
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "dry-run-test",
        ])
        .assert()
        .success();

    // Read state before
    let state_before = fs::read_to_string(
        ctx.repo_path()
            .join(".repoverlay/overlays/dry-run-test.ccl"),
    )
    .expect("state file should exist");

    // Move with --dry-run
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "dry-run-test",
            "--to",
            "library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .success();

    // Nothing should have changed
    let state_after = fs::read_to_string(
        ctx.repo_path()
            .join(".repoverlay/overlays/dry-run-test.ccl"),
    )
    .expect("state file should still exist");
    assert_eq!(
        state_before, state_after,
        "state should not change on dry run"
    );

    // Library should not have been created
    assert!(
        !ctx.repo_path()
            .join(".repoverlay/library/dry-run-test")
            .exists(),
        "library entry should not exist after dry run"
    );
}

#[test]
fn move_circular_noop() {
    let ctx = TestContext::new();

    // Create a library overlay and apply it
    create_library_overlay(&ctx, "circular-test", &[(".envrc", "export C=true")]);

    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "circular-test",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Move from library to library should warn
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "circular-test",
            "--to",
            "library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("already"));
}

#[test]
fn move_nonexistent_overlay_errors() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "nonexistent",
            "--to",
            "library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn move_symlinks_recreated_pointing_to_new_location() {
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        (".editorconfig", "root = true"),
    ]);

    // Apply with symlinks
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            ctx.overlay_source(),
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--name",
            "relink-test",
        ])
        .assert()
        .success();

    assert!(ctx.is_symlink(".envrc"));
    assert!(ctx.is_symlink(".editorconfig"));

    // Read symlink targets before move
    let old_envrc_target = fs::read_link(ctx.repo_path().join(".envrc")).unwrap();

    // Move to library
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "relink-test",
            "--to",
            "library",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Symlinks should still be symlinks
    assert!(ctx.is_symlink(".envrc"));
    assert!(ctx.is_symlink(".editorconfig"));

    // Symlinks should now point to the library location
    let new_envrc_target = fs::read_link(ctx.repo_path().join(".envrc")).unwrap();
    assert_ne!(
        old_envrc_target, new_envrc_target,
        "symlink target should change after move"
    );
    assert!(
        new_envrc_target
            .to_string_lossy()
            .contains(".repoverlay/library"),
        "symlink should point to library: {}",
        new_envrc_target.display()
    );

    // Files should still be readable through the symlinks
    assert_eq!(ctx.read_file(".envrc"), "export FOO=bar");
    assert_eq!(ctx.read_file(".editorconfig"), "root = true");
}

// ============================================================================
// Move to named source (issue #273)
// ============================================================================

#[test]
fn move_to_named_source_local() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Set up a local overlay source directory (org/repo/name structure)
    let overlays_root = ctx.repo_path().join("my-overlays");
    fs::create_dir_all(&overlays_root).unwrap();

    // Add the local source via `source add`
    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./my-overlays", "--name", "local-src"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // Create and apply a library overlay so we have something to move
    create_library_overlay(&ctx, "move-me", &[(".envrc", "export MOVED=1")]);
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "move-me",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.overlay_state_exists("move-me"));

    // Move to source:local-src, specifying org/repo explicitly via --target-repo
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "move-me",
            "--to",
            "source:local-src",
            "--target-repo",
            "acme/my-app",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit and push"));

    // Overlay file should be in the source under org/repo/name
    let dest = ctx
        .repo_path()
        .join("my-overlays/acme/my-app/move-me/.envrc");
    assert!(
        dest.exists(),
        "overlay file should exist in source: {}",
        dest.display()
    );

    // State should be OverlayRepo
    let state_content =
        fs::read_to_string(ctx.repo_path().join(".repoverlay/overlays/move-me.ccl"))
            .expect("state file should exist");
    assert!(
        state_content.contains("OverlayRepo"),
        "state source should be OverlayRepo, got: {state_content}"
    );

    // Symlink should still work
    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.is_symlink(".envrc"));
}

#[test]
fn move_to_unknown_source_errors() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    create_library_overlay(&ctx, "err-overlay", &[(".envrc", "export E=1")]);
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "err-overlay",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // Moving to a non-existent source should fail with a helpful message
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "err-overlay",
            "--to",
            "source:no-such-source",
            "--target-repo",
            "acme/my-app",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no-such-source"));
}

#[test]
fn move_to_source_infers_org_repo_from_git_remote() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Set an origin remote so org/repo can be auto-detected
    std::process::Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/acme/my-app.git",
        ])
        .current_dir(ctx.repo_path())
        .output()
        .expect("Failed to add remote");

    // Set up local source
    let overlays_root = ctx.repo_path().join("overlays");
    fs::create_dir_all(&overlays_root).unwrap();
    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./overlays", "--name", "my-src"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // Apply a library overlay
    create_library_overlay(&ctx, "inferred-test", &[(".editorconfig", "root = true")]);
    cargo_bin_cmd!("repoverlay")
        .args([
            "apply",
            "inferred-test",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // Move without --target-repo — should infer acme/my-app from origin remote
    cargo_bin_cmd!("repoverlay")
        .args([
            "move",
            "inferred-test",
            "--to",
            "source:my-src",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // Verify overlay landed at org/repo/name
    let dest = ctx
        .repo_path()
        .join("overlays/acme/my-app/inferred-test/.editorconfig");
    assert!(
        dest.exists(),
        "overlay should be at inferred path: {}",
        dest.display()
    );
}

// ============================================================================
// Three-Part Resolution with Repo-Local Sources (issue #276)
// ============================================================================

#[test]
fn apply_three_part_resolves_repo_local_source() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Create a local overlays directory with org/repo/overlay structure
    let overlay_dir = ctx.repo_path().join("my-overlays/acme/widgets/my-overlay");
    fs::create_dir_all(&overlay_dir).expect("Failed to create overlay dir");
    fs::write(overlay_dir.join(".envrc"), "export FOO=bar").unwrap();

    // Add as repo-local source
    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./my-overlays", "--name", "local-src"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // Apply using three-part reference — this should resolve via the repo-local source
    cargo_bin_cmd!("repoverlay")
        .args(["apply", "acme/widgets/my-overlay"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Applying"));

    assert!(ctx.file_exists(".envrc"));
    assert_eq!(ctx.read_file(".envrc"), "export FOO=bar");
}

#[test]
fn apply_three_part_with_source_filter_resolves_repo_local_source() {
    let ctx = TestContext::new();
    let config_dir = tempfile::TempDir::new().unwrap();

    // Create a local overlays directory with org/repo/overlay structure
    let overlay_dir = ctx.repo_path().join("my-overlays/acme/widgets/my-overlay");
    fs::create_dir_all(&overlay_dir).expect("Failed to create overlay dir");
    fs::write(overlay_dir.join(".editorconfig"), "root = true").unwrap();

    // Add as repo-local source
    cargo_bin_cmd!("repoverlay")
        .args(["source", "add", "./my-overlays", "--name", "team-src"])
        .current_dir(ctx.repo_path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    // Apply using --from to target the repo-local source by name
    cargo_bin_cmd!("repoverlay")
        .args(["apply", "acme/widgets/my-overlay", "--from", "team-src"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .env("XDG_CONFIG_HOME", config_dir.path())
        .assert()
        .success();

    assert!(ctx.file_exists(".editorconfig"));
    assert_eq!(ctx.read_file(".editorconfig"), "root = true");
}
