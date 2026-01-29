//! Common test utilities and helpers for repoverlay tests.
//!
//! This module is only compiled when running tests.

#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// A test context that provides a temporary git repository and overlay directory.
///
/// Provides helper methods to reduce repetitive test setup and assertions.
pub struct TestContext {
    /// The temporary git repository (target)
    pub repo: TempDir,
    /// Optional overlay directory
    overlay: Option<TempDir>,
}

impl TestContext {
    /// Create a new test context with an initialized git repository.
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

    /// Get the path to the test repository.
    pub fn repo_path(&self) -> &Path {
        self.repo.path()
    }

    /// Get the path to the overlay directory (panics if no overlay was created).
    pub fn overlay_path(&self) -> &Path {
        self.overlay.as_ref().expect("No overlay created").path()
    }

    /// Create an overlay with the given files and return self for chaining.
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

    /// Check if a file exists in the test repository.
    pub fn file_exists(&self, path: &str) -> bool {
        self.repo.path().join(path).exists()
    }

    /// Check if a path is a symlink in the test repository.
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

    /// Get the overlay source string (path as string).
    pub fn overlay_source(&self) -> &str {
        self.overlay_path().to_str().expect("Invalid overlay path")
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
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

/// Create a test git repository and return the TempDir.
pub fn create_test_repo() -> TempDir {
    let dir = TempDir::new().expect("Failed to create temp dir");
    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to init git repo");
    dir
}

/// Create a test overlay directory with the given files.
pub fn create_test_overlay(files: &[(&str, &str)]) -> TempDir {
    create_overlay_dir(files)
}

/// Common overlay content for a simple .envrc file.
pub fn envrc_overlay() -> Vec<(&'static str, &'static str)> {
    vec![(".envrc", "export FOO=bar")]
}

/// Common overlay content for nested files.
pub fn nested_overlay() -> Vec<(&'static str, &'static str)> {
    vec![
        (".envrc", "export FOO=bar"),
        (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
    ]
}

/// Overlay with a custom config specifying path mappings.
pub fn mapped_overlay() -> Vec<(&'static str, &'static str)> {
    vec![
        (".envrc", "export FOO=bar"),
        (
            "repoverlay.ccl",
            r#"mappings =
  .envrc = .env
"#,
        ),
    ]
}

/// Overlay with a custom name in config.
pub fn named_overlay(name: &str) -> Vec<(String, String)> {
    vec![
        (".envrc".to_string(), "export FOO=bar".to_string()),
        (
            "repoverlay.ccl".to_string(),
            format!(
                r#"overlay =
  name = {name}
"#
            ),
        ),
    ]
}
