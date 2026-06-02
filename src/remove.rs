use anyhow::{Context, Result, bail};
use colored::Colorize;
use log::{debug, trace};
use std::fs;
use std::path::Path;

use crate::OverlayName;
use crate::canonicalize_path;
use crate::path_safety::{check_no_symlink_ancestors, validate_repo_relative_path};
use crate::state::{
    EntryType, META_FILE, OVERLAYS_DIR, STATE_DIR, list_applied_overlays, load_overlay_state,
    normalize_overlay_name, remove_external_state,
};
use crate::update_git_exclude;

/// Remove applied overlay(s) from a target repository.
///
/// # Workflow
///
/// 1. Load overlay state from `.repoverlay/overlays/<name>.ccl`
/// 2. Remove each file/symlink managed by the overlay
/// 3. Clean up empty parent directories
/// 4. Remove overlay section from `.git/info/exclude`
/// 5. Delete state file
/// 6. Remove external backup
/// 7. If no overlays remain, remove `.repoverlay/` directory
pub(crate) fn remove_overlay(
    target: &Path,
    name: Option<String>,
    remove_all: bool,
    dry_run: bool,
) -> Result<()> {
    debug!(
        "remove_overlay: target={}, name={:?}, remove_all={}, dry_run={}",
        target.display(),
        name,
        remove_all,
        dry_run
    );

    if dry_run {
        let target = canonicalize_path(target, "Target directory")?;
        let applied_overlays = list_applied_overlays(&target)?;

        if remove_all {
            println!("{} Dry run - would remove all overlays:", "Note:".yellow());
            for overlay_name in &applied_overlays {
                println!("  - {overlay_name}");
            }
        } else if let Some(ref name) = name {
            println!(
                "{} Dry run - would remove overlay '{}'",
                "Note:".yellow(),
                name
            );
        }
        return Ok(());
    }
    let target = canonicalize_path(target, "Target directory")?;
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    if remove_all {
        // Remove all overlays
        let mut errors = Vec::new();
        for overlay_name in &applied_overlays {
            if let Err(e) = remove_single_overlay(&target, &overlays_dir, overlay_name.as_str()) {
                errors.push(format!("{overlay_name}: {e:#}"));
            }
        }

        if list_applied_overlays(&target)?.is_empty() {
            // Clean up state files but preserve library and config
            cleanup_state_dir(&target)?;
        }

        if !errors.is_empty() {
            bail!(
                "Failed to remove one or more overlays:\n{}",
                errors.join("\n")
            );
        }

        println!("\n{} Removed all overlays", "✓".green().bold());
    } else if let Some(name) = name {
        let normalized_name = normalize_overlay_name(&name)?;
        remove_single_overlay(&target, &overlays_dir, &normalized_name)?;

        // Check if any overlays remain
        let remaining = list_applied_overlays(&target)?;
        if remaining.is_empty() {
            // No overlays left, clean up state files but preserve library
            cleanup_state_dir(&target)?;
        }
    } else {
        // This path should not be reached from non-interactive contexts
        bail!("No overlay name specified. Use --all to remove all overlays, or specify a name.");
    }

    Ok(())
}

/// Remove overlay state files while preserving the library and config.
///
/// Removes `overlays/` dir and `meta.ccl` but leaves `library/`, `config.ccl`,
/// and any other non-state contents of `.repoverlay/` intact. If the state
/// directory is empty after cleanup, removes it too.
fn cleanup_state_dir(target: &Path) -> Result<()> {
    let state_dir = target.join(STATE_DIR);

    // Remove overlays/ subdirectory
    let overlays_dir = state_dir.join(OVERLAYS_DIR);
    if overlays_dir.exists() {
        fs::remove_dir_all(&overlays_dir)?;
    }

    // Remove meta.ccl
    let meta_file = state_dir.join(META_FILE);
    if meta_file.exists() {
        fs::remove_file(&meta_file)?;
    }

    // Remove state dir if now empty (fails silently if library/config remain)
    let _ = fs::remove_dir(&state_dir);

    Ok(())
}

/// Remove a single overlay by name.
pub(crate) fn remove_single_overlay(target: &Path, overlays_dir: &Path, name: &str) -> Result<()> {
    debug!("remove_single_overlay: {name}");
    let state_file = overlays_dir.join(format!("{name}.ccl"));

    if !state_file.exists() {
        // List available overlays for helpful error message
        let available = list_applied_overlays(target)?;

        if available.is_empty() {
            bail!("No overlays are currently applied");
        }
        let names: Vec<&str> = available.iter().map(OverlayName::as_str).collect();
        bail!(
            "Overlay '{}' not found. Available overlays: {}",
            name,
            names.join(", ")
        );
    }

    let state = load_overlay_state(target, name)?;

    println!("{} overlay: {}", "Removing".red().bold(), state.name);

    for entry in state.file_entries() {
        validate_removal_target(target, &entry.target)?;
    }

    // Remove files and directories
    for entry in state.file_entries() {
        let file_path = target.join(&entry.target);
        trace!("removing: {}", file_path.display());

        if file_path.exists() || file_path.is_symlink() {
            match entry.entry_type {
                EntryType::Directory => {
                    // For directory entries, check if it's a symlink or a real directory
                    if file_path.is_symlink() {
                        // Remove symlink (use remove_file on Unix, remove_dir on Windows for dir symlinks)
                        #[cfg(unix)]
                        fs::remove_file(&file_path).with_context(|| {
                            format!(
                                "Failed to remove directory symlink: {}",
                                file_path.display()
                            )
                        })?;
                        #[cfg(windows)]
                        fs::remove_dir(&file_path).with_context(|| {
                            format!(
                                "Failed to remove directory symlink: {}",
                                file_path.display()
                            )
                        })?;
                    } else {
                        // It's a copied directory, remove recursively
                        fs::remove_dir_all(&file_path).with_context(|| {
                            format!("Failed to remove directory: {}", file_path.display())
                        })?;
                    }
                    println!("  {} {}/", "-".red(), entry.target.display());
                }
                EntryType::File => {
                    fs::remove_file(&file_path)
                        .with_context(|| format!("Failed to remove: {}", file_path.display()))?;
                    println!("  {} {}", "-".red(), entry.target.display());
                }
            }

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
    }

    // Update git exclude (remove this overlay's section)
    let exclude_entries: Vec<String> = state
        .file_entries()
        .iter()
        .map(|e| {
            let path = e.target.to_string_lossy().replace('\\', "/");
            // Add trailing slash for directories in git exclude
            match e.entry_type {
                EntryType::Directory => format!("{path}/"),
                EntryType::File => path,
            }
        })
        .collect();
    let exclude_update_result = update_git_exclude(target, name, &exclude_entries, false);

    // Remove state file
    fs::remove_file(&state_file)?;

    // Remove external backup
    if let Err(e) = remove_external_state(target, name) {
        eprintln!(
            "  {} Could not remove external backup: {}",
            "Warning:".yellow(),
            e
        );
    }

    if let Err(e) = exclude_update_result {
        return Err(e).context("Managed files were removed, but failed to update git exclude");
    }

    println!(
        "\n{} Removed {} file(s) from '{}'",
        "✓".green().bold(),
        state.file_count(),
        state.name
    );

    Ok(())
}

fn validate_removal_target(repo_root: &Path, target_rel: &Path) -> Result<()> {
    validate_repo_relative_path(target_rel).with_context(|| {
        format!(
            "Unsafe state target '{}': managed paths must stay within the repository",
            target_rel.display()
        )
    })?;

    if let Some(parent) = target_rel.parent()
        && !parent.as_os_str().is_empty()
    {
        check_no_symlink_ancestors(repo_root, parent).with_context(|| {
            format!(
                "Unsafe state target '{}': managed path ancestors must not contain symlinks",
                target_rel.display()
            )
        })?;
    }

    Ok(())
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

    // Tests for remove_single_overlay
    mod remove_single_overlay_tests {
        use super::*;
        use crate::state::{FileEntry, LinkType, OverlayState};
        use crate::{ConflictStrategy, apply_overlay};

        fn create_test_repo_with_overlay() -> (TempDir, String) {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join(".envrc"), "export FOO=bar").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true, // copy mode so we don't need to keep overlay dir alive
                Some("test-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let overlays_dir = repo
                .path()
                .join(STATE_DIR)
                .join(OVERLAYS_DIR)
                .to_string_lossy()
                .to_string();
            (repo, overlays_dir)
        }

        #[test]
        fn removes_applied_overlay_files() {
            let (repo, _) = create_test_repo_with_overlay();
            let overlays_dir = repo.path().join(STATE_DIR).join(OVERLAYS_DIR);

            assert!(repo.path().join(".envrc").exists());

            remove_single_overlay(repo.path(), &overlays_dir, "test-overlay").unwrap();

            assert!(!repo.path().join(".envrc").exists());
        }

        #[test]
        fn removes_state_file() {
            let (repo, _) = create_test_repo_with_overlay();
            let overlays_dir = repo.path().join(STATE_DIR).join(OVERLAYS_DIR);

            let state_file = overlays_dir.join("test-overlay.ccl");
            assert!(state_file.exists());

            remove_single_overlay(repo.path(), &overlays_dir, "test-overlay").unwrap();

            assert!(!state_file.exists());
        }

        #[test]
        fn removes_git_exclude_section() {
            let (repo, _) = create_test_repo_with_overlay();
            let overlays_dir = repo.path().join(STATE_DIR).join(OVERLAYS_DIR);

            let exclude_path = repo.path().join(".git/info/exclude");
            let content_before = fs::read_to_string(&exclude_path).unwrap();
            assert!(content_before.contains("# repoverlay:test-overlay start"));

            remove_single_overlay(repo.path(), &overlays_dir, "test-overlay").unwrap();

            let content_after = fs::read_to_string(&exclude_path).unwrap();
            assert!(!content_after.contains("# repoverlay:test-overlay"));
        }

        #[test]
        fn nonexistent_overlay_returns_error() {
            let (repo, _) = create_test_repo_with_overlay();
            let overlays_dir = repo.path().join(STATE_DIR).join(OVERLAYS_DIR);

            // Try to remove non-existent overlay when there's a real one applied
            let result = remove_single_overlay(repo.path(), &overlays_dir, "nonexistent");
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("not found"),
                "Error should contain 'not found': {err}"
            );
            assert!(
                err.contains("test-overlay"),
                "Error should list available overlays: {err}"
            );
        }

        #[test]
        fn nonexistent_overlay_with_no_overlays_applied() {
            let repo = create_test_repo();
            let overlays_dir = repo.path().join(STATE_DIR).join(OVERLAYS_DIR);
            fs::create_dir_all(&overlays_dir).unwrap();

            let result = remove_single_overlay(repo.path(), &overlays_dir, "nonexistent");
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("No overlays are currently applied"));
        }

        #[test]
        fn removes_nested_file_and_cleans_parent_dir() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::create_dir_all(overlay.path().join(".vscode")).unwrap();
            fs::write(
                overlay.path().join(".vscode/settings.json"),
                r#"{"editor.tabSize": 2}"#,
            )
            .unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true, // copy mode
                Some("nested-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            assert!(repo.path().join(".vscode/settings.json").exists());
            assert!(repo.path().join(".vscode").exists());

            let overlays_dir = repo.path().join(STATE_DIR).join(OVERLAYS_DIR);
            remove_single_overlay(repo.path(), &overlays_dir, "nested-overlay").unwrap();

            assert!(!repo.path().join(".vscode/settings.json").exists());
            // Parent .vscode should be cleaned up since it's empty
            assert!(!repo.path().join(".vscode").exists());
        }

        #[cfg(unix)]
        #[test]
        fn rejects_tampered_state_target_through_symlink_ancestor() {
            use std::os::unix::fs::symlink;

            let (repo, _) = create_test_repo_with_overlay();
            let overlays_dir = repo.path().join(STATE_DIR).join(OVERLAYS_DIR);
            let state_file = overlays_dir.join("test-overlay.ccl");
            let outside = TempDir::new().unwrap();
            let external_file = outside.path().join("victim.txt");
            fs::write(&external_file, "keep me").unwrap();
            symlink(outside.path(), repo.path().join("linked")).unwrap();

            let content = fs::read_to_string(&state_file).unwrap();
            let mut state: OverlayState = sickle::from_str(&content).unwrap();
            state.files = vec![FileEntry {
                source: Path::new("victim.txt").to_path_buf(),
                target: Path::new("linked/victim.txt").to_path_buf(),
                link_type: LinkType::Copy,
                entry_type: EntryType::File,
            }];
            fs::write(&state_file, sickle::to_string(&state).unwrap()).unwrap();

            let result = remove_single_overlay(repo.path(), &overlays_dir, "test-overlay");

            assert!(result.is_err());
            assert_eq!(fs::read_to_string(external_file).unwrap(), "keep me");
            assert!(
                state_file.exists(),
                "state should remain for failed removal"
            );
        }
    }

    // Tests for cleanup_state_dir
    mod cleanup_state_dir_tests {
        use super::*;
        use crate::testutil::create_test_repo;

        #[test]
        fn preserves_library_directory() {
            let repo = create_test_repo();
            let state_dir = repo.path().join(STATE_DIR);
            let library_dir = state_dir.join("library").join("my-overlay");
            let overlays_dir = state_dir.join(OVERLAYS_DIR);

            // Set up: state files + library
            fs::create_dir_all(&library_dir).unwrap();
            fs::write(library_dir.join("file.txt"), "content").unwrap();
            fs::create_dir_all(&overlays_dir).unwrap();
            fs::write(overlays_dir.join("test.ccl"), "state").unwrap();
            fs::write(state_dir.join(META_FILE), "version = 1").unwrap();

            cleanup_state_dir(repo.path()).unwrap();

            // Library should survive
            assert!(library_dir.join("file.txt").exists());
            // State files should be gone
            assert!(!overlays_dir.exists());
            assert!(!state_dir.join(META_FILE).exists());
            // State dir itself should still exist (library is inside)
            assert!(state_dir.exists());
        }

        #[test]
        fn removes_empty_state_dir() {
            let repo = create_test_repo();
            let state_dir = repo.path().join(STATE_DIR);
            let overlays_dir = state_dir.join(OVERLAYS_DIR);

            // Set up: only state files, no library
            fs::create_dir_all(&overlays_dir).unwrap();
            fs::write(overlays_dir.join("test.ccl"), "state").unwrap();
            fs::write(state_dir.join(META_FILE), "version = 1").unwrap();

            cleanup_state_dir(repo.path()).unwrap();

            // Everything should be gone
            assert!(!state_dir.exists());
        }

        #[test]
        fn idempotent_when_already_clean() {
            let repo = create_test_repo();

            // No state dir at all — should not error
            cleanup_state_dir(repo.path()).unwrap();
            cleanup_state_dir(repo.path()).unwrap();
        }

        #[test]
        fn preserves_config_file() {
            let repo = create_test_repo();
            let state_dir = repo.path().join(STATE_DIR);
            let overlays_dir = state_dir.join(OVERLAYS_DIR);

            fs::create_dir_all(&overlays_dir).unwrap();
            fs::write(overlays_dir.join("test.ccl"), "state").unwrap();
            fs::write(state_dir.join("config.ccl"), "library_path = custom").unwrap();

            cleanup_state_dir(repo.path()).unwrap();

            // Config should survive
            assert!(state_dir.join("config.ccl").exists());
            assert!(!overlays_dir.exists());
        }
    }

    // Tests for remove_overlay
    mod remove_overlay_tests {
        use super::*;
        use crate::{ConflictStrategy, apply_overlay};

        #[test]
        fn remove_all_removes_all_overlays() {
            let repo = create_test_repo();
            let overlay1 = TempDir::new().unwrap();
            fs::write(overlay1.path().join("file1.txt"), "content1").unwrap();
            let overlay2 = TempDir::new().unwrap();
            fs::write(overlay2.path().join("file2.txt"), "content2").unwrap();

            // Apply two overlays
            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("overlay-1".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("overlay-2".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            assert!(canonical.join("file1.txt").exists());
            assert!(canonical.join("file2.txt").exists());

            // Remove all
            remove_overlay(&canonical, None, true, false).unwrap();

            assert!(!canonical.join("file1.txt").exists());
            assert!(!canonical.join("file2.txt").exists());
            assert!(!canonical.join(STATE_DIR).exists());
        }

        #[test]
        fn remove_by_name_removes_single_overlay() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("test-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            remove_overlay(&canonical, Some("test-overlay".to_string()), false, false).unwrap();

            assert!(!canonical.join("file.txt").exists());
        }

        #[test]
        fn remove_no_overlays_applied_fails() {
            let repo = create_test_repo();
            let canonical = repo.path().canonicalize().unwrap();

            let result = remove_overlay(&canonical, None, true, false);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No overlays"));
        }

        #[test]
        fn remove_no_name_no_all_fails() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let result = remove_overlay(&canonical, None, false, false);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("No overlay name specified")
            );
        }

        #[test]
        fn dry_run_does_not_remove() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            // Dry run with --all
            remove_overlay(&canonical, None, true, true).unwrap();
            // Files should still exist
            assert!(canonical.join("file.txt").exists());

            // Dry run with specific name
            remove_overlay(&canonical, Some("test".to_string()), false, true).unwrap();
            assert!(canonical.join("file.txt").exists());
        }

        #[test]
        fn remove_last_overlay_cleans_state_dir() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("only-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            assert!(canonical.join(STATE_DIR).exists());

            remove_overlay(&canonical, Some("only-overlay".to_string()), false, false).unwrap();

            // State dir should be cleaned up when last overlay is removed
            assert!(!canonical.join(STATE_DIR).exists());
        }

        #[test]
        fn remove_reports_error_when_git_exclude_cleanup_fails_after_removing_files() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("exclude-cleanup-fails".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let exclude_path = canonical.join(".git/info/exclude");
            fs::remove_file(&exclude_path).unwrap();
            fs::create_dir(&exclude_path).unwrap();

            let result = remove_overlay(
                &canonical,
                Some("exclude-cleanup-fails".to_string()),
                false,
                false,
            );

            assert!(result.is_err());
            let error = result.unwrap_err().to_string();
            assert!(error.contains("Managed files were removed"));
            assert!(error.contains("failed to update git exclude"));
            assert!(!canonical.join("file.txt").exists());
            assert!(
                !canonical
                    .join(STATE_DIR)
                    .join(OVERLAYS_DIR)
                    .join("exclude-cleanup-fails.ccl")
                    .exists()
            );
        }

        #[test]
        fn remove_all_continues_after_git_exclude_cleanup_failure() {
            let repo = create_test_repo();
            let overlay_one = TempDir::new().unwrap();
            fs::write(overlay_one.path().join("one.txt"), "one").unwrap();
            let overlay_two = TempDir::new().unwrap();
            fs::write(overlay_two.path().join("two.txt"), "two").unwrap();

            apply_overlay(
                overlay_one.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("one".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();
            apply_overlay(
                overlay_two.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("two".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let exclude_path = canonical.join(".git/info/exclude");
            fs::remove_file(&exclude_path).unwrap();
            fs::create_dir(&exclude_path).unwrap();

            let result = remove_overlay(&canonical, None, true, false);

            assert!(result.is_err());
            let error = result.unwrap_err().to_string();
            assert!(error.contains("Failed to remove one or more overlays"));
            assert!(!canonical.join("one.txt").exists());
            assert!(!canonical.join("two.txt").exists());
            assert!(!canonical.join(STATE_DIR).exists());
        }
    }
}
