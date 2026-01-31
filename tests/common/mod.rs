//! Common test utilities for CLI tests.

#![allow(dead_code)]

use assert_cmd::Command as AssertCommand;
use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Context for testing source commands with isolated config.
///
/// Creates a temporary directory for `XDG_CONFIG_HOME` so tests don't
/// interfere with each other or the user's real config.
pub struct SourceTestContext {
    config_dir: TempDir,
}

impl SourceTestContext {
    pub fn new() -> Self {
        Self {
            config_dir: TempDir::new().expect("Failed to create temp config dir"),
        }
    }

    /// Create a command with the isolated config directory.
    pub fn cmd(&self) -> AssertCommand {
        let mut cmd = cargo_bin_cmd!("repoverlay");
        cmd.env("XDG_CONFIG_HOME", self.config_dir.path());
        cmd
    }
}

/// A test context that provides a temporary git repository and overlay directory.
pub struct TestContext {
    pub repo: TempDir,
    overlay: Option<TempDir>,
}

impl TestContext {
    pub fn new() -> Self {
        let repo = TempDir::new().expect("Failed to create temp dir");
        Command::new("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .expect("Failed to init git repo");

        Self {
            repo,
            overlay: None,
        }
    }

    pub fn repo_path(&self) -> &Path {
        self.repo.path()
    }

    pub fn overlay_path(&self) -> &Path {
        self.overlay.as_ref().expect("No overlay created").path()
    }

    pub fn with_overlay(mut self, files: &[(&str, &str)]) -> Self {
        self.overlay = Some(create_overlay_dir(files));
        self
    }

    /// Create a file in the test repository.
    pub fn create_repo_file(&self, path: &str, content: &str) {
        let file_path = self.repo.path().join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent dirs");
        }
        fs::write(file_path, content).expect("Failed to write file");
    }

    pub fn file_exists(&self, path: &str) -> bool {
        self.repo.path().join(path).exists()
    }

    pub fn is_symlink(&self, path: &str) -> bool {
        self.repo.path().join(path).is_symlink()
    }

    /// Read file content from the test repository.
    pub fn read_file(&self, path: &str) -> String {
        fs::read_to_string(self.repo.path().join(path)).expect("Failed to read file")
    }

    /// Get the content of .git/info/exclude.
    pub fn git_exclude_content(&self) -> String {
        let exclude_path = self.repo.path().join(".git/info/exclude");
        if exclude_path.exists() {
            fs::read_to_string(exclude_path).expect("Failed to read exclude")
        } else {
            String::new()
        }
    }

    /// Check if the .repoverlay state directory exists.
    pub fn state_dir_exists(&self) -> bool {
        self.repo.path().join(".repoverlay").exists()
    }

    /// Check if an overlay state file exists.
    pub fn overlay_state_exists(&self, name: &str) -> bool {
        self.repo
            .path()
            .join(format!(".repoverlay/overlays/{name}.ccl"))
            .exists()
    }

    pub fn overlay_source(&self) -> &str {
        self.overlay_path().to_str().expect("Invalid overlay path")
    }
}

/// Create a test overlay directory with the given files.
pub fn create_overlay_dir(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("Failed to create temp dir");
    for (path, content) in files {
        let file_path = dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent dirs");
        }
        fs::write(file_path, content).expect("Failed to write file");
    }
    dir
}

pub fn envrc_overlay() -> Vec<(&'static str, &'static str)> {
    vec![(".envrc", "export FOO=bar")]
}
