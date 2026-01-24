//! Integration tests for upstream detection and overlay resolution.

use std::process::Command;
use tempfile::TempDir;

use crate::common::TestContext;

/// Create a test repo that looks like a fork (has upstream remote).
fn create_fork_repo() -> (TempDir, String, String) {
    let dir = TempDir::new().unwrap();

    // Initialize repo
    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Add origin (the fork)
    Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/tylerbutler/FluidFramework.git",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Add upstream (the parent repo)
    Command::new("git")
        .args([
            "remote",
            "add",
            "upstream",
            "https://github.com/microsoft/FluidFramework.git",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    (dir, "tylerbutler".to_string(), "microsoft".to_string())
}

#[test]
fn detect_upstream_from_fork() {
    let (repo, _fork_org, upstream_org) = create_fork_repo();

    let upstream = repoverlay::detect_upstream(repo.path())
        .expect("detection should not fail")
        .expect("should detect upstream");

    assert_eq!(upstream.org, upstream_org);
    assert_eq!(upstream.repo, "FluidFramework");
    assert_eq!(upstream.remote_name, "upstream");
}

#[test]
fn no_upstream_when_only_origin() {
    let ctx = TestContext::new();

    // Add only origin remote
    Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/tylerbutler/FluidFramework.git",
        ])
        .current_dir(ctx.repo_path())
        .output()
        .unwrap();

    let upstream = repoverlay::detect_upstream(ctx.repo_path()).expect("detection should not fail");

    assert!(upstream.is_none());
}

#[test]
fn upstream_detection_with_ssh_url() {
    let dir = TempDir::new().unwrap();

    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    Command::new("git")
        .args([
            "remote",
            "add",
            "upstream",
            "git@github.com:microsoft/FluidFramework.git",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let upstream = repoverlay::detect_upstream(dir.path())
        .expect("detection should not fail")
        .expect("should detect upstream");

    assert_eq!(upstream.org, "microsoft");
    assert_eq!(upstream.repo, "FluidFramework");
}
