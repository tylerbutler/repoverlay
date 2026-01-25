//! Common test utilities for CLI tests.

#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

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

    pub fn file_exists(&self, path: &str) -> bool {
        self.repo.path().join(path).exists()
    }

    pub fn is_symlink(&self, path: &str) -> bool {
        self.repo.path().join(path).is_symlink()
    }

    pub fn overlay_source(&self) -> &str {
        self.overlay_path().to_str().expect("Invalid overlay path")
    }
}

fn create_overlay_dir(files: &[(&str, &str)]) -> TempDir {
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
