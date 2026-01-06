use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Overlay config files into git repositories without committing them
#[derive(Parser)]
#[command(name = "repoverlay")]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Apply an overlay to a git repository
    Apply {
        /// Path to the overlay source directory
        source: PathBuf,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Force copy mode instead of symlinks (default on Windows)
        #[arg(long)]
        copy: bool,
    },

    /// Remove the currently applied overlay
    Remove {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Show the status of the current overlay
    Status {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },
}

/// Configuration file for an overlay source (repoverlay.toml)
#[derive(Debug, Deserialize, Serialize, Default)]
struct OverlayConfig {
    #[serde(default)]
    overlay: OverlayMeta,
    #[serde(default)]
    mappings: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct OverlayMeta {
    name: Option<String>,
    description: Option<String>,
}

/// State file tracking applied overlay (.repoverlay/state.toml)
#[derive(Debug, Deserialize, Serialize)]
struct OverlayState {
    meta: StateMeta,
    files: Vec<FileEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StateMeta {
    applied_at: DateTime<Utc>,
    source: PathBuf,
    name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FileEntry {
    source: PathBuf,
    target: PathBuf,
    #[serde(rename = "type")]
    link_type: LinkType,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum LinkType {
    Symlink,
    Copy,
}

const STATE_DIR: &str = ".repoverlay";
const STATE_FILE: &str = "state.toml";
const CONFIG_FILE: &str = "repoverlay.toml";
const GIT_EXCLUDE: &str = ".git/info/exclude";
const EXCLUDE_MARKER_START: &str = "# repoverlay start";
const EXCLUDE_MARKER_END: &str = "# repoverlay end";

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Apply {
            source,
            target,
            copy,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            apply_overlay(&source, &target, copy)?;
        }
        Commands::Remove { target } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            remove_overlay(&target)?;
        }
        Commands::Status { target } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            show_status(&target)?;
        }
    }

    Ok(())
}

fn apply_overlay(source: &Path, target: &Path, force_copy: bool) -> Result<()> {
    // Validate source exists
    let source = source
        .canonicalize()
        .with_context(|| format!("Overlay source not found: {}", source.display()))?;

    // Validate target exists and is a git repo
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

    if !target.join(".git").exists() {
        bail!("Target is not a git repository: {}", target.display());
    }

    // Check if overlay is already applied
    let state_dir = target.join(STATE_DIR);
    if state_dir.exists() {
        bail!("An overlay is already applied. Run 'repoverlay remove' first.");
    }

    // Determine link type
    let link_type = if force_copy || cfg!(windows) {
        LinkType::Copy
    } else {
        LinkType::Symlink
    };

    // Load overlay config (optional)
    let config_path = source.join(CONFIG_FILE);
    let config: OverlayConfig = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", config_path.display()))?
    } else {
        OverlayConfig::default()
    };

    let overlay_name = config.overlay.name.clone().unwrap_or_else(|| {
        source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string())
    });

    println!("{} overlay: {}", "Applying".green().bold(), overlay_name);

    // Collect files to overlay
    let mut files: Vec<FileEntry> = Vec::new();
    let mut exclude_entries: Vec<String> = Vec::new();

    for entry in WalkDir::new(&source)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel_path = entry.path().strip_prefix(&source)?;

        // Skip the config file itself
        if rel_path == Path::new(CONFIG_FILE) {
            continue;
        }

        let rel_str = rel_path.to_string_lossy().to_string();

        // Apply path mapping if defined
        let target_rel = config
            .mappings
            .get(&rel_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| rel_path.to_path_buf());

        let source_file = entry.path().to_path_buf();
        let target_file = target.join(&target_rel);

        // Check for conflicts
        if target_file.exists() {
            bail!(
                "Conflict: target file already exists: {}\nRemove it first or add a mapping to rename the overlay file.",
                target_file.display()
            );
        }

        // Create parent directories if needed
        if let Some(parent) = target_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Create symlink or copy
        match link_type {
            LinkType::Symlink => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&source_file, &target_file).with_context(|| {
                    format!("Failed to create symlink: {}", target_file.display())
                })?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(&source_file, &target_file).with_context(
                    || format!("Failed to create symlink: {}", target_file.display()),
                )?;
            }
            LinkType::Copy => {
                fs::copy(&source_file, &target_file)
                    .with_context(|| format!("Failed to copy file: {}", target_file.display()))?;
            }
        }

        println!("  {} {}", "+".green(), target_rel.display());

        files.push(FileEntry {
            source: rel_path.to_path_buf(),
            target: target_rel.clone(),
            link_type,
        });

        // Add to exclude list (use forward slashes for git)
        let exclude_path = target_rel.to_string_lossy().replace('\\', "/");
        exclude_entries.push(exclude_path);
    }

    if files.is_empty() {
        bail!("No files found in overlay source: {}", source.display());
    }

    // Add .repoverlay to exclude entries too
    exclude_entries.push(STATE_DIR.to_string());

    // Update .git/info/exclude with all entries at once
    update_git_exclude(&target, &exclude_entries, true)?;

    // Save state
    let state = OverlayState {
        meta: StateMeta {
            applied_at: Utc::now(),
            source: source.clone(),
            name: Some(overlay_name.clone()),
        },
        files,
    };

    fs::create_dir_all(&state_dir)?;
    let state_content = toml::to_string_pretty(&state)?;
    fs::write(state_dir.join(STATE_FILE), state_content)?;

    println!(
        "\n{} Applied {} file(s) from '{}'",
        "✓".green().bold(),
        state.files.len(),
        overlay_name
    );

    Ok(())
}

fn remove_overlay(target: &Path) -> Result<()> {
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

    let state_dir = target.join(STATE_DIR);
    let state_file = state_dir.join(STATE_FILE);

    if !state_file.exists() {
        bail!("No overlay is currently applied in: {}", target.display());
    }

    // Load state
    let state_content = fs::read_to_string(&state_file)?;
    let state: OverlayState = toml::from_str(&state_content)?;

    let overlay_name = state.meta.name.as_deref().unwrap_or("unnamed");
    println!("{} overlay: {}", "Removing".red().bold(), overlay_name);

    let mut exclude_entries: Vec<String> = Vec::new();

    // Remove files
    for entry in &state.files {
        let file_path = target.join(&entry.target);

        if file_path.exists() || file_path.is_symlink() {
            fs::remove_file(&file_path)
                .with_context(|| format!("Failed to remove: {}", file_path.display()))?;
            println!("  {} {}", "-".red(), entry.target.display());

            // Remove empty parent directories (but not the target itself)
            let mut parent = file_path.parent();
            while let Some(dir) = parent {
                if dir == target {
                    break;
                }
                if dir
                    .read_dir()
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
                {
                    fs::remove_dir(dir).ok();
                    parent = dir.parent();
                } else {
                    break;
                }
            }
        }

        let exclude_path = entry.target.to_string_lossy().replace('\\', "/");
        exclude_entries.push(exclude_path);
    }

    // Add .repoverlay to list and remove all from .git/info/exclude at once
    exclude_entries.push(STATE_DIR.to_string());
    update_git_exclude(&target, &exclude_entries, false)?;

    // Remove state directory
    fs::remove_dir_all(&state_dir)?;

    println!(
        "\n{} Removed {} file(s)",
        "✓".green().bold(),
        state.files.len()
    );

    Ok(())
}

fn show_status(target: &Path) -> Result<()> {
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

    let state_file = target.join(STATE_DIR).join(STATE_FILE);

    if !state_file.exists() {
        println!("{} No overlay is currently applied.", "Status:".bold());
        return Ok(());
    }

    let state_content = fs::read_to_string(&state_file)?;
    let state: OverlayState = toml::from_str(&state_content)?;

    let overlay_name = state.meta.name.as_deref().unwrap_or("unnamed");

    println!("{}", "Overlay Status".bold());
    println!("  Name:    {}", overlay_name.cyan());
    println!("  Source:  {}", state.meta.source.display());
    println!(
        "  Applied: {}",
        state.meta.applied_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("  Files:   {}", state.files.len());
    println!();

    for entry in &state.files {
        let target_path = target.join(&entry.target);
        let status = if target_path.exists() || target_path.is_symlink() {
            "✓".green()
        } else {
            "✗".red()
        };

        let type_str = match entry.link_type {
            LinkType::Symlink => "symlink",
            LinkType::Copy => "copy",
        };

        println!(
            "  {} {} ({})",
            status,
            entry.target.display(),
            type_str.dimmed()
        );
    }

    Ok(())
}

fn update_git_exclude(target: &Path, entries: &[String], add: bool) -> Result<()> {
    let exclude_path = target.join(GIT_EXCLUDE);

    // Ensure the .git/info directory exists
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = fs::read_to_string(&exclude_path).unwrap_or_default();

    if add {
        // Remove any existing repoverlay section first
        content = remove_repoverlay_section(&content);

        // Add new section
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        content.push_str(EXCLUDE_MARKER_START);
        content.push('\n');
        for entry in entries {
            content.push_str(entry);
            content.push('\n');
        }
        content.push_str(EXCLUDE_MARKER_END);
        content.push('\n');
    } else {
        content = remove_repoverlay_section(&content);
    }

    fs::write(&exclude_path, content)?;
    Ok(())
}

fn remove_repoverlay_section(content: &str) -> String {
    let mut result = String::new();
    let mut in_section = false;

    for line in content.lines() {
        if line.trim() == EXCLUDE_MARKER_START {
            in_section = true;
            continue;
        }
        if line.trim() == EXCLUDE_MARKER_END {
            in_section = false;
            continue;
        }
        if !in_section {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Remove trailing newlines if the section was at the end
    while result.ends_with("\n\n") {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    // Helper to create a test git repository
    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to init git repo");
        dir
    }

    // Helper to create a test overlay directory with files
    fn create_test_overlay(files: &[(&str, &str)]) -> TempDir {
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

    // Unit tests for remove_repoverlay_section
    mod remove_section {
        use super::*;

        #[test]
        fn empty_content() {
            let result = remove_repoverlay_section("");
            assert_eq!(result, "");
        }

        #[test]
        fn no_section_present() {
            let content = "*.log\n.DS_Store\n";
            let result = remove_repoverlay_section(content);
            assert_eq!(result, "*.log\n.DS_Store\n");
        }

        #[test]
        fn section_at_end() {
            let content = "*.log\n# repoverlay start\n.envrc\n.repoverlay\n# repoverlay end\n";
            let result = remove_repoverlay_section(content);
            assert_eq!(result, "*.log\n");
        }

        #[test]
        fn section_at_beginning() {
            let content = "# repoverlay start\n.envrc\n# repoverlay end\n*.log\n";
            let result = remove_repoverlay_section(content);
            assert_eq!(result, "*.log\n");
        }

        #[test]
        fn section_in_middle() {
            let content = "*.log\n# repoverlay start\n.envrc\n# repoverlay end\n.DS_Store\n";
            let result = remove_repoverlay_section(content);
            assert_eq!(result, "*.log\n.DS_Store\n");
        }

        #[test]
        fn only_section() {
            let content = "# repoverlay start\n.envrc\n.repoverlay\n# repoverlay end\n";
            let result = remove_repoverlay_section(content);
            assert_eq!(result, "");
        }

        #[test]
        fn multiple_entries_in_section() {
            let content = "# repoverlay start\n.envrc\n.vscode/settings.json\n.repoverlay\n# repoverlay end\n";
            let result = remove_repoverlay_section(content);
            assert_eq!(result, "");
        }
    }

    // Integration tests for apply command
    mod apply {
        use super::*;

        #[test]
        fn applies_single_file() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(overlay.path(), repo.path(), false);
            assert!(result.is_ok(), "apply_overlay failed: {:?}", result);

            // Check symlink was created
            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists(), ".envrc should exist");
            assert!(target_file.is_symlink(), ".envrc should be a symlink");

            // Check content is correct
            let content = fs::read_to_string(&target_file).unwrap();
            assert_eq!(content, "export FOO=bar");

            // Check state was saved
            let state_file = repo.path().join(".repoverlay/state.toml");
            assert!(state_file.exists(), "state.toml should exist");
        }

        #[test]
        fn applies_nested_files() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
            ]);

            let result = apply_overlay(overlay.path(), repo.path(), false);
            assert!(result.is_ok());

            assert!(repo.path().join(".envrc").exists());
            assert!(repo.path().join(".vscode/settings.json").exists());
        }

        #[test]
        fn applies_with_copy_mode() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(overlay.path(), repo.path(), true);
            assert!(result.is_ok());

            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists());
            assert!(
                !target_file.is_symlink(),
                ".envrc should NOT be a symlink in copy mode"
            );
        }

        #[test]
        fn updates_git_exclude() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(content.contains("# repoverlay start"));
            assert!(content.contains(".envrc"));
            assert!(content.contains(".repoverlay"));
            assert!(content.contains("# repoverlay end"));
        }

        #[test]
        fn respects_path_mappings() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (
                    "repoverlay.toml",
                    r#"
[mappings]
".envrc" = ".env"
"#,
                ),
            ]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();

            assert!(
                !repo.path().join(".envrc").exists(),
                ".envrc should not exist"
            );
            assert!(repo.path().join(".env").exists(), ".env should exist");
        }

        #[test]
        fn uses_overlay_name_from_config() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (
                    "repoverlay.toml",
                    r#"
[overlay]
name = "my-custom-overlay"
"#,
                ),
            ]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();

            let state_content =
                fs::read_to_string(repo.path().join(".repoverlay/state.toml")).unwrap();
            assert!(state_content.contains("my-custom-overlay"));
        }

        #[test]
        fn fails_on_non_git_directory() {
            let dir = TempDir::new().unwrap();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(overlay.path(), dir.path(), false);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not a git repository")
            );
        }

        #[test]
        fn fails_on_existing_overlay() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();

            let result = apply_overlay(overlay.path(), repo.path(), false);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("already applied"));
        }

        #[test]
        fn fails_on_file_conflict() {
            let repo = create_test_repo();
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let overlay = create_test_overlay(&[(".envrc", "new content")]);

            let result = apply_overlay(overlay.path(), repo.path(), false);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Conflict"));
        }

        #[test]
        fn fails_on_empty_overlay() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            let result = apply_overlay(overlay.path(), repo.path(), false);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No files found"));
        }

        #[test]
        fn fails_on_nonexistent_source() {
            let repo = create_test_repo();
            let result = apply_overlay(Path::new("/nonexistent/path"), repo.path(), false);
            assert!(result.is_err());
        }
    }

    // Integration tests for remove command
    mod remove {
        use super::*;

        #[test]
        fn removes_applied_overlay() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (".vscode/settings.json", r#"{"key": "value"}"#),
            ]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();
            remove_overlay(repo.path()).unwrap();

            assert!(!repo.path().join(".envrc").exists());
            assert!(!repo.path().join(".vscode/settings.json").exists());
            assert!(!repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn removes_empty_parent_directories() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".vscode/settings.json", r#"{"key": "value"}"#)]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();
            assert!(repo.path().join(".vscode").exists());

            remove_overlay(repo.path()).unwrap();
            assert!(
                !repo.path().join(".vscode").exists(),
                ".vscode should be removed"
            );
        }

        #[test]
        fn preserves_non_empty_parent_directories() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".vscode/settings.json", r#"{"key": "value"}"#)]);

            // Create another file in .vscode that isn't from the overlay
            fs::create_dir_all(repo.path().join(".vscode")).unwrap();
            fs::write(repo.path().join(".vscode/other.json"), "{}").unwrap();

            apply_overlay(overlay.path(), repo.path(), false).unwrap();
            remove_overlay(repo.path()).unwrap();

            assert!(
                repo.path().join(".vscode").exists(),
                ".vscode should remain"
            );
            assert!(repo.path().join(".vscode/other.json").exists());
        }

        #[test]
        fn cleans_git_exclude() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();
            remove_overlay(repo.path()).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(!content.contains("# repoverlay start"));
            assert!(!content.contains(".envrc"));
            assert!(!content.contains("# repoverlay end"));
        }

        #[test]
        fn fails_when_no_overlay_applied() {
            let repo = create_test_repo();

            let result = remove_overlay(repo.path());
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No overlay"));
        }

        #[test]
        fn handles_already_deleted_files() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();

            // Manually delete the file
            fs::remove_file(repo.path().join(".envrc")).unwrap();

            // Remove should still succeed
            let result = remove_overlay(repo.path());
            assert!(result.is_ok());
        }
    }

    // Integration tests for status command
    mod status {
        use super::*;

        #[test]
        fn shows_no_overlay_when_none_applied() {
            let repo = create_test_repo();
            let result = show_status(repo.path());
            assert!(result.is_ok());
        }

        #[test]
        fn shows_status_when_overlay_applied() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false).unwrap();

            let result = show_status(repo.path());
            assert!(result.is_ok());
        }
    }

    // CLI integration tests using assert_cmd
    mod cli {
        use super::*;
        use assert_cmd::assert::OutputAssertExt;
        use predicates::prelude::*;

        fn repoverlay_cmd() -> std::process::Command {
            let path = std::env::var("CARGO_BIN_EXE_repoverlay").unwrap_or_else(|_| {
                env!("CARGO_MANIFEST_DIR").to_string() + "/target/debug/repoverlay"
            });
            std::process::Command::new(path)
        }

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
        fn apply_requires_source_argument() {
            repoverlay_cmd()
                .arg("apply")
                .assert()
                .failure()
                .stderr(predicate::str::contains("required"));
        }

        #[test]
        fn apply_and_remove_workflow() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            // Apply
            repoverlay_cmd()
                .args(["apply", overlay.path().to_str().unwrap()])
                .args(["--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("Applying"));

            assert!(repo.path().join(".envrc").exists());

            // Status
            repoverlay_cmd()
                .args(["status", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("Overlay Status"));

            // Remove
            repoverlay_cmd()
                .args(["remove", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("Removing"));

            assert!(!repo.path().join(".envrc").exists());
        }

        #[test]
        fn apply_with_copy_flag() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            repoverlay_cmd()
                .args(["apply", overlay.path().to_str().unwrap()])
                .args(["--target", repo.path().to_str().unwrap()])
                .arg("--copy")
                .assert()
                .success();

            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists());
            assert!(!target_file.is_symlink());
        }

        #[test]
        fn status_when_no_overlay() {
            let repo = create_test_repo();

            repoverlay_cmd()
                .args(["status", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("No overlay"));
        }

        #[test]
        fn remove_when_no_overlay() {
            let repo = create_test_repo();

            repoverlay_cmd()
                .args(["remove", "--target", repo.path().to_str().unwrap()])
                .assert()
                .failure()
                .stderr(predicate::str::contains("No overlay"));
        }
    }
}
