//! File detection for overlay creation.
//!
//! This module provides functionality to detect files that are good candidates
//! for overlays, including AI configuration files and gitignored/untracked files.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Categories of detected files for overlay creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileCategory {
    /// AI agent configuration files (Claude, Cursor, Copilot, etc.)
    AiConfig,
    /// Files that are gitignored but exist on disk
    Gitignored,
    /// Files that are untracked (not in git, not ignored)
    Untracked,
}

/// A detected file with its category and path.
#[derive(Debug, Clone)]
pub struct DetectedFile {
    /// Relative path from repository root
    pub path: PathBuf,
    /// Category of the file
    pub category: FileCategory,
    /// Whether this file should be pre-selected by default
    pub preselected: bool,
}

/// AI configuration file patterns.
///
/// These patterns match configuration files for various AI coding assistants.
pub const AI_CONFIG_PATTERNS: &[&str] = &[
    // Claude Code
    ".claude",
    "CLAUDE.md",
    // Cursor
    ".cursor",
    ".cursorrules",
    ".cursorules",
    // GitHub Copilot
    ".github/copilot-instructions.md",
    // Cody (Sourcegraph)
    ".cody",
    "cody.json",
    // Windsurf
    ".windsurfrules",
    // Continue.dev
    ".continue",
    // Aider
    ".aider",
    ".aiderignore",
    // Generic AI
    ".ai",
    "ai-instructions.md",
];

/// Check if a path matches any AI config pattern.
pub fn is_ai_config(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    for pattern in AI_CONFIG_PATTERNS {
        // Check exact match
        if path_str == *pattern {
            return true;
        }
        // Check if path starts with pattern (for directories like .claude/)
        if path_str.starts_with(pattern) {
            return true;
        }
        // Check if pattern is a prefix with separator
        let pattern_with_sep = format!("{}/", pattern);
        if path_str.starts_with(&pattern_with_sep) {
            return true;
        }
    }

    false
}

/// Detect AI configuration files in a repository.
///
/// Returns paths relative to the repository root.
pub fn detect_ai_configs(repo_path: &Path) -> Vec<DetectedFile> {
    let mut results = Vec::new();

    for pattern in AI_CONFIG_PATTERNS {
        let full_path = repo_path.join(pattern);
        if full_path.exists() {
            results.push(DetectedFile {
                path: PathBuf::from(pattern),
                category: FileCategory::AiConfig,
                preselected: true, // AI configs are pre-selected by default
            });
        }
    }

    results
}

/// Detect gitignored files that exist on disk.
///
/// Uses `git ls-files --others --ignored --exclude-standard` to find files
/// that are ignored by git but still exist in the repository.
pub fn detect_gitignored_files(repo_path: &Path) -> Vec<DetectedFile> {
    let output = Command::new("git")
        .args(["ls-files", "--others", "--ignored", "--exclude-standard"])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .filter(|line| !line.is_empty())
                .filter(|line| !is_ai_config(Path::new(line))) // Don't duplicate AI configs
                .map(|line| DetectedFile {
                    path: PathBuf::from(line),
                    category: FileCategory::Gitignored,
                    preselected: false,
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Detect untracked files (not in git, not ignored).
///
/// Uses `git ls-files --others --exclude-standard` without --ignored
/// to find files that are neither tracked nor ignored.
pub fn detect_untracked_files(repo_path: &Path) -> Vec<DetectedFile> {
    let output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .filter(|line| !line.is_empty())
                .filter(|line| !is_ai_config(Path::new(line))) // Don't duplicate AI configs
                .map(|line| DetectedFile {
                    path: PathBuf::from(line),
                    category: FileCategory::Untracked,
                    preselected: false,
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Discover all overlay candidate files in a repository.
///
/// Returns files organized by category:
/// 1. AI configuration files (pre-selected)
/// 2. Gitignored files
/// 3. Untracked files
pub fn discover_files(repo_path: &Path) -> Vec<DetectedFile> {
    let mut all_files = Vec::new();

    // First, add AI configs (these are pre-selected)
    all_files.extend(detect_ai_configs(repo_path));

    // Then add gitignored files
    all_files.extend(detect_gitignored_files(repo_path));

    // Finally add untracked files (excluding those already found as gitignored)
    let untracked = detect_untracked_files(repo_path);
    for file in untracked {
        // Only add if not already in the list (gitignored files might overlap)
        if !all_files.iter().any(|f| f.path == file.path) {
            all_files.push(file);
        }
    }

    all_files
}

/// Group detected files by category for display.
pub fn group_by_category(files: &[DetectedFile]) -> Vec<(FileCategory, Vec<&DetectedFile>)> {
    let mut ai_configs: Vec<&DetectedFile> = Vec::new();
    let mut gitignored: Vec<&DetectedFile> = Vec::new();
    let mut untracked: Vec<&DetectedFile> = Vec::new();

    for file in files {
        match file.category {
            FileCategory::AiConfig => ai_configs.push(file),
            FileCategory::Gitignored => gitignored.push(file),
            FileCategory::Untracked => untracked.push(file),
        }
    }

    let mut groups = Vec::new();
    if !ai_configs.is_empty() {
        groups.push((FileCategory::AiConfig, ai_configs));
    }
    if !gitignored.is_empty() {
        groups.push((FileCategory::Gitignored, gitignored));
    }
    if !untracked.is_empty() {
        groups.push((FileCategory::Untracked, untracked));
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to init git repo");
        dir
    }

    #[test]
    fn test_is_ai_config_exact_match() {
        assert!(is_ai_config(Path::new(".claude")));
        assert!(is_ai_config(Path::new("CLAUDE.md")));
        assert!(is_ai_config(Path::new(".cursorrules")));
        assert!(is_ai_config(Path::new(".cursor")));
    }

    #[test]
    fn test_is_ai_config_subdirectory() {
        assert!(is_ai_config(Path::new(".claude/settings.json")));
        assert!(is_ai_config(Path::new(".cursor/rules.md")));
        assert!(is_ai_config(Path::new(".continue/config.json")));
    }

    #[test]
    fn test_is_ai_config_non_match() {
        assert!(!is_ai_config(Path::new(".envrc")));
        assert!(!is_ai_config(Path::new("package.json")));
        assert!(!is_ai_config(Path::new(".gitignore")));
    }

    #[test]
    fn test_detect_ai_configs() {
        let repo = create_test_repo();

        // Create some AI config files
        fs::create_dir_all(repo.path().join(".claude")).unwrap();
        fs::write(repo.path().join(".claude/settings.json"), "{}").unwrap();
        fs::write(repo.path().join("CLAUDE.md"), "# Claude").unwrap();
        fs::write(repo.path().join(".cursorrules"), "rules").unwrap();

        let configs = detect_ai_configs(repo.path());

        assert!(configs.iter().any(|f| f.path == Path::new(".claude")));
        assert!(configs.iter().any(|f| f.path == Path::new("CLAUDE.md")));
        assert!(configs.iter().any(|f| f.path == Path::new(".cursorrules")));

        // All should be pre-selected
        assert!(configs.iter().all(|f| f.preselected));
        assert!(configs.iter().all(|f| f.category == FileCategory::AiConfig));
    }

    #[test]
    fn test_detect_gitignored_files() {
        let repo = create_test_repo();

        // Create .gitignore
        fs::write(repo.path().join(".gitignore"), ".envrc\n*.log").unwrap();

        // Create ignored files
        fs::write(repo.path().join(".envrc"), "export FOO=bar").unwrap();
        fs::write(repo.path().join("debug.log"), "log content").unwrap();

        // Stage the gitignore so git knows about it
        Command::new("git")
            .args(["add", ".gitignore"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        let ignored = detect_gitignored_files(repo.path());

        assert!(ignored.iter().any(|f| f.path == Path::new(".envrc")));
        assert!(ignored.iter().any(|f| f.path == Path::new("debug.log")));
        assert!(
            ignored
                .iter()
                .all(|f| f.category == FileCategory::Gitignored)
        );
        assert!(ignored.iter().all(|f| !f.preselected));
    }

    #[test]
    fn test_detect_untracked_files() {
        let repo = create_test_repo();

        // Create untracked files (not ignored)
        fs::write(repo.path().join("scratch.txt"), "notes").unwrap();
        fs::write(repo.path().join("todo.md"), "# TODO").unwrap();

        let untracked = detect_untracked_files(repo.path());

        assert!(untracked.iter().any(|f| f.path == Path::new("scratch.txt")));
        assert!(untracked.iter().any(|f| f.path == Path::new("todo.md")));
        assert!(
            untracked
                .iter()
                .all(|f| f.category == FileCategory::Untracked)
        );
    }

    #[test]
    fn test_discover_files_combines_all() {
        let repo = create_test_repo();

        // Create AI config
        fs::write(repo.path().join("CLAUDE.md"), "# Claude").unwrap();

        // Create gitignored file
        fs::write(repo.path().join(".gitignore"), ".envrc").unwrap();
        fs::write(repo.path().join(".envrc"), "export FOO=bar").unwrap();
        Command::new("git")
            .args(["add", ".gitignore"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        // Create untracked file
        fs::write(repo.path().join("notes.txt"), "notes").unwrap();

        let all_files = discover_files(repo.path());

        // Should have files from all categories
        assert!(
            all_files
                .iter()
                .any(|f| f.category == FileCategory::AiConfig)
        );
        assert!(
            all_files
                .iter()
                .any(|f| f.category == FileCategory::Gitignored)
        );
        assert!(
            all_files
                .iter()
                .any(|f| f.category == FileCategory::Untracked)
        );
    }

    #[test]
    fn test_group_by_category() {
        let files = vec![
            DetectedFile {
                path: PathBuf::from(".claude"),
                category: FileCategory::AiConfig,
                preselected: true,
            },
            DetectedFile {
                path: PathBuf::from(".envrc"),
                category: FileCategory::Gitignored,
                preselected: false,
            },
            DetectedFile {
                path: PathBuf::from("notes.txt"),
                category: FileCategory::Untracked,
                preselected: false,
            },
        ];

        let groups = group_by_category(&files);

        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].0, FileCategory::AiConfig);
        assert_eq!(groups[1].0, FileCategory::Gitignored);
        assert_eq!(groups[2].0, FileCategory::Untracked);
    }
}
