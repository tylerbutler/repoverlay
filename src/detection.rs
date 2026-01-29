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
    /// AI agent configuration directories (like .claude/)
    AiConfigDirectory,
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

/// AI configuration directories that should be symlinked as units.
///
/// These are directories containing AI agent configuration that should
/// be linked as a whole rather than having their contents walked.
#[allow(dead_code)] // Will be used for directory detection in create command
pub const AI_CONFIG_DIRECTORIES: &[&str] =
    &[".claude", ".cursor", ".continue", ".cody", ".aider", ".ai"];

/// Check if a path matches any AI config pattern.
pub fn is_ai_config(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    for pattern in AI_CONFIG_PATTERNS {
        // Check exact match (e.g., ".claude" == ".claude")
        if path_str == *pattern {
            return true;
        }
        // Check if path is inside a pattern directory (e.g., ".claude/file" matches ".claude")
        // Must use separator to avoid false matches like ".claude-backup" matching ".claude"
        let pattern_with_sep = format!("{pattern}/");
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

/// Detect AI configuration directories in a repository.
///
/// Returns directories that should be symlinked as units.
#[allow(dead_code)] // Will be used for directory detection in create command
pub fn detect_ai_config_directories(repo_path: &Path) -> Vec<DetectedFile> {
    let mut results = Vec::new();

    for dir_name in AI_CONFIG_DIRECTORIES {
        let full_path = repo_path.join(dir_name);
        if full_path.exists() && full_path.is_dir() {
            results.push(DetectedFile {
                path: PathBuf::from(dir_name),
                category: FileCategory::AiConfigDirectory,
                preselected: true, // AI config directories are pre-selected by default
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
    let mut ai_config_dirs: Vec<&DetectedFile> = Vec::new();
    let mut gitignored: Vec<&DetectedFile> = Vec::new();
    let mut untracked: Vec<&DetectedFile> = Vec::new();

    for file in files {
        match file.category {
            FileCategory::AiConfig => ai_configs.push(file),
            FileCategory::AiConfigDirectory => ai_config_dirs.push(file),
            FileCategory::Gitignored => gitignored.push(file),
            FileCategory::Untracked => untracked.push(file),
        }
    }

    let mut groups = Vec::new();
    if !ai_configs.is_empty() {
        groups.push((FileCategory::AiConfig, ai_configs));
    }
    if !ai_config_dirs.is_empty() {
        groups.push((FileCategory::AiConfigDirectory, ai_config_dirs));
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

    #[test]
    fn test_group_by_category_empty() {
        let files: Vec<DetectedFile> = vec![];
        let groups = group_by_category(&files);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_group_by_category_single_category() {
        let files = vec![
            DetectedFile {
                path: PathBuf::from(".claude"),
                category: FileCategory::AiConfig,
                preselected: true,
            },
            DetectedFile {
                path: PathBuf::from("CLAUDE.md"),
                category: FileCategory::AiConfig,
                preselected: true,
            },
        ];

        let groups = group_by_category(&files);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, FileCategory::AiConfig);
        assert_eq!(groups[0].1.len(), 2);
    }

    #[test]
    fn test_is_ai_config_github_copilot() {
        assert!(is_ai_config(Path::new(".github/copilot-instructions.md")));
    }

    #[test]
    fn test_is_ai_config_windsurf() {
        assert!(is_ai_config(Path::new(".windsurfrules")));
    }

    #[test]
    fn test_is_ai_config_aider() {
        assert!(is_ai_config(Path::new(".aider")));
        assert!(is_ai_config(Path::new(".aiderignore")));
    }

    #[test]
    fn test_is_ai_config_continue() {
        assert!(is_ai_config(Path::new(".continue")));
        assert!(is_ai_config(Path::new(".continue/config.json")));
    }

    #[test]
    fn test_is_ai_config_cody() {
        assert!(is_ai_config(Path::new(".cody")));
        assert!(is_ai_config(Path::new("cody.json")));
    }

    #[test]
    fn test_is_ai_config_generic() {
        assert!(is_ai_config(Path::new(".ai")));
        assert!(is_ai_config(Path::new("ai-instructions.md")));
    }

    #[test]
    fn test_detect_ai_configs_empty_repo() {
        let repo = create_test_repo();
        let configs = detect_ai_configs(repo.path());
        assert!(configs.is_empty());
    }

    #[test]
    fn test_detect_gitignored_excludes_ai_configs() {
        let repo = create_test_repo();

        // Create gitignore that includes an AI config pattern
        fs::write(repo.path().join(".gitignore"), ".claude\n.envrc").unwrap();

        // Create both AI config and regular ignored file
        fs::create_dir_all(repo.path().join(".claude")).unwrap();
        fs::write(repo.path().join(".claude/settings.json"), "{}").unwrap();
        fs::write(repo.path().join(".envrc"), "export FOO=bar").unwrap();

        Command::new("git")
            .args(["add", ".gitignore"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        let ignored = detect_gitignored_files(repo.path());

        // Should NOT include .claude/settings.json (it's an AI config)
        assert!(!ignored.iter().any(|f| f.path.starts_with(".claude")));
        // Should include .envrc
        assert!(ignored.iter().any(|f| f.path == Path::new(".envrc")));
    }

    #[test]
    fn test_detect_untracked_excludes_ai_configs() {
        let repo = create_test_repo();

        // Create AI config file and regular file
        fs::write(repo.path().join("CLAUDE.md"), "# Claude").unwrap();
        fs::write(repo.path().join("notes.txt"), "notes").unwrap();

        let untracked = detect_untracked_files(repo.path());

        // Should NOT include CLAUDE.md
        assert!(!untracked.iter().any(|f| f.path == Path::new("CLAUDE.md")));
        // Should include notes.txt
        assert!(untracked.iter().any(|f| f.path == Path::new("notes.txt")));
    }

    #[test]
    fn test_discover_files_deduplicates() {
        let repo = create_test_repo();

        // Create a file that might appear in both gitignored and untracked
        fs::write(repo.path().join("test.txt"), "test").unwrap();

        let all_files = discover_files(repo.path());

        // Count how many times test.txt appears
        let count = all_files
            .iter()
            .filter(|f| f.path == Path::new("test.txt"))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_file_category_equality() {
        assert_eq!(FileCategory::AiConfig, FileCategory::AiConfig);
        assert_eq!(FileCategory::Gitignored, FileCategory::Gitignored);
        assert_eq!(FileCategory::Untracked, FileCategory::Untracked);
        assert_ne!(FileCategory::AiConfig, FileCategory::Gitignored);
    }

    #[test]
    fn test_detected_file_clone() {
        let file = DetectedFile {
            path: PathBuf::from("test.txt"),
            category: FileCategory::AiConfig,
            preselected: true,
        };
        let cloned = file.clone();
        assert_eq!(cloned.path, file.path);
        assert_eq!(cloned.category, file.category);
        assert_eq!(cloned.preselected, file.preselected);
    }

    #[test]
    fn test_detect_ai_config_directories() {
        let repo = create_test_repo();

        // Create AI config directories
        fs::create_dir_all(repo.path().join(".claude")).unwrap();
        fs::write(repo.path().join(".claude/settings.json"), "{}").unwrap();
        fs::create_dir_all(repo.path().join(".cursor")).unwrap();
        fs::write(repo.path().join(".cursor/rules.md"), "# Rules").unwrap();

        let dirs = detect_ai_config_directories(repo.path());

        assert!(dirs.iter().any(|f| f.path == Path::new(".claude")));
        assert!(dirs.iter().any(|f| f.path == Path::new(".cursor")));
        assert!(
            dirs.iter()
                .all(|f| f.category == FileCategory::AiConfigDirectory)
        );
        assert!(dirs.iter().all(|f| f.preselected));
    }

    #[test]
    fn test_detect_ai_config_directories_empty_repo() {
        let repo = create_test_repo();
        let dirs = detect_ai_config_directories(repo.path());
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_detect_ai_config_directories_skips_files() {
        let repo = create_test_repo();

        // Create a file that matches a directory pattern name but is not a directory
        fs::write(repo.path().join(".claude"), "not a directory").unwrap();

        let dirs = detect_ai_config_directories(repo.path());

        // Should not detect .claude as a directory since it's a file
        assert!(!dirs.iter().any(|f| f.path == Path::new(".claude")));
    }

    #[test]
    fn test_ai_config_directory_category_equality() {
        assert_eq!(
            FileCategory::AiConfigDirectory,
            FileCategory::AiConfigDirectory
        );
        assert_ne!(FileCategory::AiConfigDirectory, FileCategory::AiConfig);
    }

    #[test]
    fn test_group_by_category_with_directories() {
        let files = vec![
            DetectedFile {
                path: PathBuf::from(".claude"),
                category: FileCategory::AiConfigDirectory,
                preselected: true,
            },
            DetectedFile {
                path: PathBuf::from("CLAUDE.md"),
                category: FileCategory::AiConfig,
                preselected: true,
            },
            DetectedFile {
                path: PathBuf::from(".envrc"),
                category: FileCategory::Gitignored,
                preselected: false,
            },
        ];

        let groups = group_by_category(&files);

        // Should have 3 groups (AiConfig, AiConfigDirectory, Gitignored)
        assert_eq!(groups.len(), 3);

        // Find the AiConfigDirectory group
        let dir_group = groups
            .iter()
            .find(|(cat, _)| *cat == FileCategory::AiConfigDirectory);
        assert!(dir_group.is_some());
        assert_eq!(dir_group.unwrap().1.len(), 1);
    }

    #[test]
    fn test_detect_gitignored_files_non_git_directory() {
        // Test fallback when git command fails (non-git directory)
        let temp = TempDir::new().unwrap();
        // Don't initialize git - this should trigger the fallback
        fs::write(temp.path().join(".envrc"), "export FOO=bar").unwrap();

        let ignored = detect_gitignored_files(temp.path());
        // Should return empty vec when git fails
        assert!(ignored.is_empty());
    }

    #[test]
    fn test_detect_untracked_files_non_git_directory() {
        // Test fallback when git command fails (non-git directory)
        let temp = TempDir::new().unwrap();
        // Don't initialize git - this should trigger the fallback
        fs::write(temp.path().join("notes.txt"), "notes").unwrap();

        let untracked = detect_untracked_files(temp.path());
        // Should return empty vec when git fails
        assert!(untracked.is_empty());
    }
}
