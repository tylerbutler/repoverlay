//! Shared test utilities and helpers for repoverlay tests.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// A test context that manages a temporary git repository and overlay directory.
pub struct TestContext {
    pub repo: TempDir,
    pub overlay: TempDir,
}

impl TestContext {
    /// Create a new test context with an empty git repo and empty overlay.
    pub fn new() -> Self {
        Self {
            repo: create_git_repo(),
            overlay: TempDir::new().unwrap(),
        }
    }

    /// Create a test context with overlay files.
    pub fn with_files(files: &[(&str, &str)]) -> Self {
        Self {
            repo: create_git_repo(),
            overlay: create_overlay_dir(files),
        }
    }

    /// Get the repository path.
    pub fn repo_path(&self) -> &Path {
        self.repo.path()
    }

    /// Get the overlay source path as a string.
    pub fn overlay_str(&self) -> &str {
        self.overlay.path().to_str().unwrap()
    }

    /// Check if a file exists in the repo.
    pub fn file_exists(&self, path: &str) -> bool {
        self.repo.path().join(path).exists()
    }

    /// Check if a path is a symlink in the repo.
    pub fn is_symlink(&self, path: &str) -> bool {
        self.repo.path().join(path).is_symlink()
    }

    /// Read a file from the repo.
    pub fn read_file(&self, path: &str) -> String {
        fs::read_to_string(self.repo.path().join(path)).unwrap()
    }

    /// Write a file to the repo (for conflict testing).
    pub fn write_repo_file(&self, path: &str, content: &str) {
        let file_path = self.repo.path().join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(file_path, content).unwrap();
    }

    /// Get the git exclude file content.
    pub fn git_exclude_content(&self) -> String {
        let exclude_path = self.repo.path().join(".git/info/exclude");
        fs::read_to_string(&exclude_path).unwrap_or_default()
    }

    /// Check if the .repoverlay state directory exists.
    pub fn state_dir_exists(&self) -> bool {
        self.repo.path().join(".repoverlay").exists()
    }

    /// Check if a specific overlay state file exists.
    pub fn overlay_state_exists(&self, name: &str) -> bool {
        self.repo
            .path()
            .join(format!(".repoverlay/overlays/{}.toml", name))
            .exists()
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a temporary git repository.
pub fn create_git_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to init git repo");
    dir
}

/// Create a temporary overlay directory with the specified files.
pub fn create_overlay_dir(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (path, content) in files {
        let file_path = dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(file_path, content).unwrap();
    }
    dir
}

/// Standard test overlay with a single .envrc file.
pub fn envrc_overlay() -> TempDir {
    create_overlay_dir(&[(".envrc", "export FOO=bar")])
}

/// Standard test overlay with nested files.
pub fn nested_overlay() -> TempDir {
    create_overlay_dir(&[
        (".envrc", "export FOO=bar"),
        (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
    ])
}

/// Helper to get a repoverlay Command for CLI tests.
pub fn repoverlay_cmd() -> Command {
    let path = std::env::var("CARGO_BIN_EXE_repoverlay").unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/debug/repoverlay")
            .to_string_lossy()
            .to_string()
    });
    Command::new(path)
}
