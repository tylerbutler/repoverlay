use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::detection;
use crate::selection;

/// Expand glob patterns in include paths relative to the source directory.
///
/// Paths that contain glob metacharacters (`*`, `?`, `[`) are expanded using
/// [`glob::glob`]. Paths without metacharacters are passed through unchanged.
/// Returns an error if a glob pattern matches no files or if a literal path
/// does not exist.
pub(crate) fn expand_include_globs(source: &Path, include: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut expanded = Vec::new();
    for path in include {
        let path_str = path.to_string_lossy();
        if path_str.contains('*') || path_str.contains('?') || path_str.contains('[') {
            let pattern = source.join(path).to_string_lossy().to_string();
            let matches: Vec<PathBuf> = glob::glob(&pattern)
                .with_context(|| format!("Invalid glob pattern: {path_str}"))?
                .filter_map(Result::ok)
                .collect();
            if matches.is_empty() {
                bail!("Glob pattern matched no files: {path_str}");
            }
            for matched in matches {
                let rel = matched
                    .strip_prefix(source)
                    .unwrap_or(&matched)
                    .to_path_buf();
                expanded.push(rel);
            }
        } else {
            let full_path = source.join(path);
            if !full_path.exists() {
                bail!("Include path does not exist: {}", path.display());
            }
            expanded.push(path.clone());
        }
    }
    Ok(expanded)
}

/// Create a new overlay from files in a repository.
///
/// # Modes
///
/// - **Discovery mode** (no `--include`): Scans repository for candidate files
///   (AI configs, gitignored, untracked) and presents interactive selection
/// - **Explicit mode** (`--include` flags): Copies specified files directly
///
/// # Output Directory Resolution
///
/// When `output` is `None`, the output directory is determined as follows:
/// 1. If an overlay source is configured (`source add` was run), the overlay is
///    created directly in the overlay repo at `<org>/<repo>/<name>/`, where
///    org/repo is detected from the source repository's git remote origin.
/// 2. If no overlay repo is configured (or git remote detection fails), falls
///    back to `~/.local/share/repoverlay/overlays/<repo-name>`.
///
/// # Workflow
///
/// 1. Validate source is a git repository
/// 2. If no includes specified, discover candidate files
/// 3. Interactive selection or use pre-selected AI configs (with `--yes`)
/// 4. Copy selected files to output directory
/// 5. Generate `repoverlay.ccl` config file
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn create_overlay(
    source: &Path,
    output: Option<PathBuf>,
    include: &[PathBuf],
    name: Option<String>,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    // Verify source is a git repository
    if !source.join(".git").exists() {
        bail!(
            "Source directory is not a git repository: {}",
            source.display()
        );
    }

    // Determine output directory
    // Priority: explicit --local > local fallback
    let output_dir: PathBuf = if let Some(p) = &output {
        p.clone()
    } else {
        let repo_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("overlay");
        let proj_dirs = directories::ProjectDirs::from("", "", "repoverlay")
            .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;
        proj_dirs.data_dir().join("overlays").join(repo_name)
    };

    // If no includes specified, run discovery mode
    if include.is_empty() {
        // Discover files in the repository
        print!(
            "{} Scanning for overlay candidates...",
            "Discovery:".cyan().bold()
        );
        std::io::Write::flush(&mut std::io::stdout())?;

        let discovered = detection::discover_files(source);

        // Show discovery summary
        let ai_count = discovered
            .iter()
            .filter(|f| f.category == detection::FileCategory::AiConfig)
            .count();
        let tc_count = discovered
            .iter()
            .filter(|f| f.category == detection::FileCategory::TrackedConfig)
            .count();
        let gi_count = discovered
            .iter()
            .filter(|f| f.category == detection::FileCategory::Gitignored)
            .count();
        let ut_count = discovered
            .iter()
            .filter(|f| f.category == detection::FileCategory::Untracked)
            .count();
        println!(
            " found {} AI, {} tracked config, {} gitignored, {} untracked",
            selection::humanize_count(ai_count).green(),
            selection::humanize_count(tc_count).cyan(),
            selection::humanize_count(gi_count).yellow(),
            selection::humanize_count(ut_count).blue()
        );

        if discovered.is_empty() {
            bail!(
                "No files discovered and none specified.\n\n\
                 Use --include to specify files to include in the overlay.\n\
                 Example:\n  repoverlay create my-overlay --include .claude/ --include CLAUDE.md"
            );
        }

        // In dry-run mode without includes, show discovered files
        if dry_run {
            println!(
                "{} Discovered files in: {}",
                "Discovery:".cyan().bold(),
                source.display()
            );
            println!();

            let groups = detection::group_by_category(&discovered);
            for (category, files) in groups {
                let category_name = match category {
                    detection::FileCategory::AiConfig => "AI Configurations".green(),
                    detection::FileCategory::AiConfigDirectory => "AI Config Directories".magenta(),
                    detection::FileCategory::TrackedConfig => "Tracked Config".cyan(),
                    detection::FileCategory::Gitignored => "Gitignored".yellow(),
                    detection::FileCategory::Untracked => "Untracked".blue(),
                };
                let preselected_note = if files.iter().any(|f| f.preselected) {
                    " (pre-selected)"
                } else {
                    ""
                };
                println!("{}{}:", category_name.bold(), preselected_note.dimmed());
                for file in files {
                    let marker = if file.preselected { "[x]" } else { "[ ]" };
                    println!("  {} {}", marker, file.path.display());
                }
                println!();
            }

            println!(
                "{}",
                "Use --include to specify which files to include:".dimmed()
            );
            // Suggest command based on discovered AI configs
            let ai_configs: Vec<_> = discovered
                .iter()
                .filter(|f| f.category == detection::FileCategory::AiConfig)
                .collect();
            if !ai_configs.is_empty() {
                let includes: Vec<_> = ai_configs
                    .iter()
                    .map(|f| format!("--include {}", f.path.display()))
                    .collect();
                println!("  repoverlay create my-overlay {}", includes.join(" "));
            }
            return Ok(());
        }

        // Interactive mode: let user select files
        if !yes {
            use selection::{SelectionConfig, select_files};

            let config = SelectionConfig::default();
            let result = select_files(&discovered, &config)?;

            if result.cancelled {
                bail!("Selection cancelled.");
            }

            if result.selected_files.is_empty() {
                bail!("No files selected. Aborting.");
            }

            // Get output directory from user if not specified
            let final_output = if output.is_none() {
                use dialoguer::Input;

                println!(
                    "Where should the overlay be created?\n\
                     (This directory will contain the overlay files and config)"
                );

                let path_str: String = Input::new()
                    .with_prompt("Overlay directory")
                    .default(output_dir.display().to_string())
                    .interact_text()?;

                PathBuf::from(path_str)
            } else {
                output_dir
            };

            // Now create the overlay with selected files
            return create_overlay_with_files(source, &final_output, &result.selected_files, name);
        }

        // With --yes flag but no includes, auto-select files:
        // 1. Prefer AI configs (preselected)
        // 2. Fall back to tracked config files
        let preselected: Vec<PathBuf> = discovered
            .iter()
            .filter(|f| f.preselected)
            .map(|f| f.path.clone())
            .collect();

        if !preselected.is_empty() {
            println!(
                "{} Using {} pre-selected AI config file(s)",
                "Auto-select:".cyan().bold(),
                preselected.len()
            );
            return create_overlay_with_files(source, &output_dir, &preselected, name);
        }

        // No AI configs — fall back to tracked config files
        let tracked_configs: Vec<PathBuf> = discovered
            .iter()
            .filter(|f| f.category == detection::FileCategory::TrackedConfig)
            .map(|f| f.path.clone())
            .collect();

        if !tracked_configs.is_empty() {
            println!(
                "{} No AI configs found. Using {} tracked config file(s)",
                "Auto-select:".cyan().bold(),
                tracked_configs.len()
            );
            return create_overlay_with_files(source, &output_dir, &tracked_configs, name);
        }

        bail!(
            "No AI configs or tracked config files found to auto-select.\n\n\
             Use --include to specify files:\n  repoverlay create my-overlay --include .envrc"
        );
    }

    // Expand globs and validate include paths
    let expanded = expand_include_globs(source, include)?;

    if dry_run {
        println!(
            "{} Would create overlay at: {}",
            "Dry run:".yellow().bold(),
            output_dir.display()
        );
        println!();
        println!("Files to include:");
        for path in &expanded {
            let full_path = source.join(path);
            if full_path.is_dir() {
                for entry in walkdir::WalkDir::new(&full_path)
                    .into_iter()
                    .filter_map(std::result::Result::ok)
                    .filter(|e| e.file_type().is_file())
                {
                    let rel = entry
                        .path()
                        .strip_prefix(source)
                        .unwrap_or_else(|_| entry.path());
                    println!("  + {}", rel.display());
                }
            } else {
                println!("  + {}", path.display());
            }
        }
        return Ok(());
    }

    // Use shared helper to copy files and generate config
    create_overlay_with_files(source, &output_dir, &expanded, name)
}

/// Copy files from source to output directory.
pub(crate) fn copy_files_to_overlay(
    source: &Path,
    output_dir: &Path,
    include: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)?;

    let mut copied_files = Vec::new();
    for path in include {
        let src_path = source.join(path);
        if src_path.is_dir() {
            for entry in walkdir::WalkDir::new(&src_path)
                .into_iter()
                .filter_entry(|e| {
                    // Skip transient tool state directories that can contain
                    // thousands of files (matching detection.rs skip logic).
                    if e.file_type().is_dir() && e.depth() > 0 {
                        let name = e.file_name().to_string_lossy();
                        if detection::SKIP_CHILD_DIRS
                            .iter()
                            .any(|skip| name.as_ref() == *skip)
                        {
                            return false;
                        }
                    }
                    true
                })
                .filter_map(std::result::Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                let rel_path = entry.path().strip_prefix(source)?;
                let dest_path = output_dir.join(rel_path);
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(entry.path(), &dest_path)?;
                copied_files.push(rel_path.to_path_buf());
            }
        } else {
            let dest_path = output_dir.join(path);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src_path, &dest_path)?;
            copied_files.push(path.clone());
        }
    }

    Ok(copied_files)
}

/// Generate overlay config file content.
pub(crate) fn generate_overlay_config(name: &str) -> String {
    format!(
        r"/= Overlay configuration file.
/= This file describes an overlay and how it should be applied.

overlay =
  /= name: Display name for this overlay.
  /= Used in status output and when listing overlays.
  name = {name}

/= mappings (optional): Remap file paths when applying the overlay.
/= Keys are source paths (in the overlay), values are target paths (in the repo).
/= Use this to rename files or place them in different locations.
/= mappings =
/=   .envrc.template = .envrc
"
    )
}

/// Print overlay creation success message.
pub(crate) fn print_overlay_created(output_dir: &Path, copied_files: &[PathBuf]) {
    println!(
        "{} overlay at: {}",
        "Created".green().bold(),
        output_dir.display()
    );
    println!();
    println!("Files included:");
    for file in copied_files {
        println!("  + {}", file.display());
    }
    println!();
    println!(
        "Apply with: {} {} {}",
        "repoverlay apply".cyan(),
        output_dir.display(),
        "--target <repo>".dimmed()
    );
}

/// Helper to create overlay with specified files.
pub(crate) fn create_overlay_with_files(
    source: &Path,
    output_dir: &Path,
    include: &[PathBuf],
    name: Option<String>,
) -> Result<()> {
    let copied_files = copy_files_to_overlay(source, output_dir, include)?;

    let overlay_name = name.unwrap_or_else(|| {
        output_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("overlay")
            .to_string()
    });

    fs::write(
        output_dir.join("repoverlay.ccl"),
        generate_overlay_config(&overlay_name),
    )?;
    print_overlay_created(output_dir, &copied_files);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // Tests for copy_files_to_overlay
    mod copy_files_to_overlay_tests {
        use super::*;

        #[test]
        fn copies_single_file() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join("file.txt"), "content").unwrap();

            let copied =
                copy_files_to_overlay(source.path(), output.path(), &[PathBuf::from("file.txt")])
                    .unwrap();

            assert_eq!(copied.len(), 1);
            assert!(output.path().join("file.txt").exists());
        }

        #[test]
        fn copies_directory_recursively() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            fs::create_dir_all(source.path().join("dir/subdir")).unwrap();
            fs::write(source.path().join("dir/file1.txt"), "content1").unwrap();
            fs::write(source.path().join("dir/subdir/file2.txt"), "content2").unwrap();

            let copied =
                copy_files_to_overlay(source.path(), output.path(), &[PathBuf::from("dir")])
                    .unwrap();

            assert_eq!(copied.len(), 2);
            assert!(output.path().join("dir/file1.txt").exists());
            assert!(output.path().join("dir/subdir/file2.txt").exists());
        }

        #[test]
        fn creates_parent_directories() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            fs::create_dir_all(source.path().join("deep/nested")).unwrap();
            fs::write(source.path().join("deep/nested/file.txt"), "content").unwrap();

            copy_files_to_overlay(
                source.path(),
                output.path(),
                &[PathBuf::from("deep/nested/file.txt")],
            )
            .unwrap();

            assert!(output.path().join("deep/nested/file.txt").exists());
        }
    }

    // Tests for generate_overlay_config
    mod generate_overlay_config_tests {
        use super::*;

        #[test]
        fn includes_overlay_name() {
            let config = generate_overlay_config("my-overlay");
            assert!(config.contains("name = my-overlay"));
        }

        #[test]
        fn includes_commented_mappings() {
            let config = generate_overlay_config("test");
            assert!(config.contains("/= mappings"));
        }

        #[test]
        fn generates_valid_ccl() {
            let config = generate_overlay_config("test-name");
            // Basic structure check
            assert!(config.contains("overlay ="));
        }
    }

    // Tests for copy_files_to_overlay additional cases
    mod copy_files_to_overlay_additional_tests {
        use super::*;

        #[test]
        fn copies_multiple_files() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join("a.txt"), "a").unwrap();
            fs::write(source.path().join("b.txt"), "b").unwrap();
            fs::write(source.path().join("c.txt"), "c").unwrap();

            let copied = copy_files_to_overlay(
                source.path(),
                output.path(),
                &[
                    PathBuf::from("a.txt"),
                    PathBuf::from("b.txt"),
                    PathBuf::from("c.txt"),
                ],
            )
            .unwrap();

            assert_eq!(copied.len(), 3);
            assert_eq!(
                fs::read_to_string(output.path().join("a.txt")).unwrap(),
                "a"
            );
            assert_eq!(
                fs::read_to_string(output.path().join("b.txt")).unwrap(),
                "b"
            );
            assert_eq!(
                fs::read_to_string(output.path().join("c.txt")).unwrap(),
                "c"
            );
        }

        #[test]
        fn creates_output_dir_if_missing() {
            let source = TempDir::new().unwrap();
            let temp = TempDir::new().unwrap();
            let output = temp.path().join("nested/output/dir");

            fs::write(source.path().join("file.txt"), "content").unwrap();

            let copied =
                copy_files_to_overlay(source.path(), &output, &[PathBuf::from("file.txt")])
                    .unwrap();

            assert_eq!(copied.len(), 1);
            assert!(output.join("file.txt").exists());
        }

        #[test]
        fn preserves_file_content() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            let content = "line1\nline2\nline3\n特殊字符\n";
            fs::write(source.path().join("file.txt"), content).unwrap();

            copy_files_to_overlay(source.path(), output.path(), &[PathBuf::from("file.txt")])
                .unwrap();

            let read_content = fs::read_to_string(output.path().join("file.txt")).unwrap();
            assert_eq!(read_content, content);
        }
    }

    // Tests for generate_overlay_config additional cases
    mod generate_overlay_config_additional_tests {
        use super::*;

        #[test]
        fn handles_special_characters_in_name() {
            let config = generate_overlay_config("test-overlay_123");
            assert!(config.contains("name = test-overlay_123"));
        }

        #[test]
        fn includes_comment_header() {
            let config = generate_overlay_config("test");
            assert!(config.contains("/= Overlay configuration file"));
        }

        #[test]
        fn includes_mappings_example() {
            let config = generate_overlay_config("test");
            assert!(config.contains(".envrc.template = .envrc"));
        }

        #[test]
        fn handles_empty_name() {
            let config = generate_overlay_config("");
            assert!(config.contains("name = \n"));
            assert!(config.contains("overlay ="));
        }
    }

    mod expand_include_globs_tests {
        use super::*;

        #[test]
        fn literal_path_passes_through() {
            let dir = TempDir::new().unwrap();
            fs::write(dir.path().join("file.txt"), "content").unwrap();

            let result = expand_include_globs(dir.path(), &[PathBuf::from("file.txt")]).unwrap();
            assert_eq!(result, vec![PathBuf::from("file.txt")]);
        }

        #[test]
        fn literal_path_missing_errors() {
            let dir = TempDir::new().unwrap();
            let result = expand_include_globs(dir.path(), &[PathBuf::from("missing.txt")]);
            assert!(result.is_err());
        }

        #[test]
        fn glob_star_matches_files() {
            let dir = TempDir::new().unwrap();
            fs::write(dir.path().join("a.md"), "").unwrap();
            fs::write(dir.path().join("b.md"), "").unwrap();
            fs::write(dir.path().join("c.txt"), "").unwrap();

            let mut result = expand_include_globs(dir.path(), &[PathBuf::from("*.md")]).unwrap();
            result.sort();
            assert_eq!(result, vec![PathBuf::from("a.md"), PathBuf::from("b.md")]);
        }

        #[test]
        fn glob_no_match_errors() {
            let dir = TempDir::new().unwrap();
            let result = expand_include_globs(dir.path(), &[PathBuf::from("*.xyz")]);
            assert!(result.is_err());
        }

        #[test]
        fn glob_double_star_matches_nested() {
            let dir = TempDir::new().unwrap();
            let sub = dir.path().join("sub");
            fs::create_dir_all(&sub).unwrap();
            fs::write(dir.path().join("top.md"), "").unwrap();
            fs::write(sub.join("nested.md"), "").unwrap();

            let mut result = expand_include_globs(dir.path(), &[PathBuf::from("**/*.md")]).unwrap();
            result.sort();
            assert_eq!(
                result,
                vec![PathBuf::from("sub/nested.md"), PathBuf::from("top.md")]
            );
        }

        #[test]
        fn mixed_literal_and_glob() {
            let dir = TempDir::new().unwrap();
            fs::write(dir.path().join("keep.txt"), "").unwrap();
            fs::write(dir.path().join("a.md"), "").unwrap();

            let result = expand_include_globs(
                dir.path(),
                &[PathBuf::from("keep.txt"), PathBuf::from("*.md")],
            )
            .unwrap();
            assert_eq!(
                result,
                vec![PathBuf::from("keep.txt"), PathBuf::from("a.md")]
            );
        }
    }
}
