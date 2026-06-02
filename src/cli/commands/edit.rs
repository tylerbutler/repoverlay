use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

use super::create::{auto_commit_overlay, extract_overlay_name};
use crate::config::load_config;
use crate::detection::{DetectedFile, FileCategory};
use crate::overlay_repo::OverlayRepoManager;
use crate::selection::{SelectionConfig, select_files};
use crate::state::{EntryType, FileEntry, LinkType, OverlaySource, SourceResolver};
use crate::{
    OverlayName, canonicalize_path, list_applied_overlays, load_all_overlay_targets,
    load_overlay_state, normalize_overlay_name, save_external_state, save_overlay_state,
    update_git_exclude,
};

/// Tracks completed operations for rollback on failure.
enum RollbackEntry {
    File {
        target: PathBuf,
        overlay: PathBuf,
        original_content: Vec<u8>,
    },
    Directory {
        target: PathBuf,
        overlay: PathBuf,
    },
}

/// Edit an existing applied overlay by re-selecting files interactively.
pub(crate) fn edit_overlay(name_arg: &str, target: &std::path::Path, dry_run: bool) -> Result<()> {
    interactive_edit_overlay(name_arg, target, dry_run)
}

/// Resolve an overlay's source to a local filesystem path.
///
/// Uses the `SourceResolver` trait to handle all source types uniformly:
/// - Local: returns the stored path directly
/// - `OverlayRepo`: reconstructs path from the overlay repo (respects `source_name`)
/// - GitHub: returns the cached download path
pub(crate) fn resolve_overlay_source_path(state: &crate::state::OverlayState) -> Result<PathBuf> {
    state.source.resolve_local_path()
}

/// Interactively re-select which files from an overlay source should be applied.
///
/// Shows the selection UI with all files from the overlay source directory,
/// pre-selecting the currently applied files. Computes the diff between the
/// old and new selections and applies adds/removes accordingly.
fn interactive_edit_overlay(name_arg: &str, target: &std::path::Path, dry_run: bool) -> Result<()> {
    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        bail!(
            "Target directory is not a git repository: {}",
            target.display()
        );
    }

    // Parse overlay name and verify it's applied
    let overlay_name = extract_overlay_name(name_arg)?;

    let normalized_name = normalize_overlay_name(&overlay_name)?;
    let applied_overlays = list_applied_overlays(&target)?;
    if !applied_overlays
        .iter()
        .any(|n| n == normalized_name.as_str())
    {
        bail!("Overlay '{overlay_name}' is not currently applied.");
    }

    let mut state = load_overlay_state(&target, &normalized_name)?;
    crate::try_upgrade_github_source(&target, &mut state)?;

    // Check mutability upfront before any changes (#142, #148, #149)
    if state.source.is_library() {
        bail!(
            "Interactive edit is not supported for library overlays.\n\n\
             Library overlays are managed in the repository's overlay library.\n\
             Edit the overlay files directly in the library directory,\n\
             then re-apply with: repoverlay apply {overlay_name}"
        );
    }
    if !state.source.is_mutable() {
        let label = state.source.source_type_label();
        bail!(
            "Interactive edit is not supported for {label} overlays.\n\n\
             {label} overlays are read-only. Use --add and --remove flags instead."
        );
    }

    // Resolve overlay source to a local directory
    let source_path = resolve_overlay_source_path(&state)?;

    if !source_path.exists() {
        bail!(
            "Overlay source directory not found: {}\n\n\
             Interactive edit requires access to the overlay source files.",
            source_path.display()
        );
    }

    // Collect current overlay file paths for pre-selection
    let current_files: HashSet<PathBuf> = state
        .file_entries()
        .iter()
        .map(|e| e.target.clone())
        .collect();

    // Walk the overlay source directory to find all available files.
    // Only skip .git directory - hidden directories like .claude/, .vscode/, etc.
    // are valid overlay content and must be included.
    let mut detected_files: Vec<DetectedFile> = Vec::new();
    let mut overlay_source_files: HashSet<PathBuf> = HashSet::new();
    for entry in WalkDir::new(&source_path)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            // Only skip .git directory
            !(e.file_type().is_dir() && e.file_name() == ".git")
        })
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            let relative = entry
                .path()
                .strip_prefix(&source_path)
                .unwrap_or_else(|_| entry.path())
                .to_path_buf();

            let rel_str = relative.to_string_lossy();

            // Skip overlay config file and cache metadata (not user-facing overlay content)
            if relative == std::path::Path::new(crate::state::CONFIG_FILE)
                || rel_str == ".repoverlay-cache-meta.ccl"
            {
                continue;
            }

            let is_currently_applied = current_files.contains(&relative);
            overlay_source_files.insert(relative.clone());

            detected_files.push(DetectedFile {
                path: relative,
                category: FileCategory::Untracked, // Generic category for overlay files
                preselected: is_currently_applied,
                parent_dir: None,
            });
        }
    }

    // Discover files in the target repo that could be added to the overlay (#190).
    // This includes AI configs, gitignored files, and untracked files that aren't
    // already in the overlay source.
    let target_repo_candidates = crate::detection::discover_files(&target);
    for candidate in target_repo_candidates {
        // Skip files already in the overlay source
        if overlay_source_files.contains(&candidate.path) {
            continue;
        }
        // Skip files that are symlinks (already managed by an overlay)
        let full_path = target.join(&candidate.path);
        if full_path.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
            continue;
        }
        detected_files.push(DetectedFile {
            preselected: false, // Target repo files start unselected
            ..candidate
        });
    }

    if detected_files.is_empty() {
        bail!(
            "No files found in overlay source or target repository: {}",
            source_path.display()
        );
    }

    // Sort by path for consistent display
    detected_files.sort_by(|a, b| a.path.cmp(&b.path));

    // Configure selection UI - don't hide any categories
    let config = SelectionConfig {
        prompt: format!("Edit overlay '{normalized_name}' \u{2014} select files to include"),
        default_hidden_categories: HashSet::new(),
    };

    let result = select_files(&detected_files, &config)?;

    if result.cancelled {
        println!("{} Selection cancelled, no changes made.", "Note:".yellow());
        return Ok(());
    }

    // Compute diff: what to add and what to remove
    let new_selection: HashSet<PathBuf> = result.selected_files.into_iter().collect();

    let to_add: Vec<PathBuf> = new_selection.difference(&current_files).cloned().collect();
    let to_remove: Vec<PathBuf> = current_files.difference(&new_selection).cloned().collect();

    if to_add.is_empty() && to_remove.is_empty() {
        println!("{} No changes to apply.", "Note:".yellow());
        return Ok(());
    }

    // Print summary
    println!("{} overlay: {}", "Editing".blue().bold(), normalized_name);
    if !to_add.is_empty() {
        println!("\nFiles to add:");
        for f in &to_add {
            println!("  {} {}", "+".green(), f.display());
        }
    }
    if !to_remove.is_empty() {
        println!("\nFiles to remove:");
        for f in &to_remove {
            println!("  {} {}", "-".red(), f.display());
        }
    }

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        return Ok(());
    }

    // Apply removals first, then additions
    if !to_remove.is_empty() {
        remove_files_from_overlay(name_arg, &target, &to_remove, false)?;
    }
    if !to_add.is_empty() {
        // Files from the overlay source need to be copied to the target first,
        // since add_files_to_overlay expects them to exist in the target.
        // Files from the target repo already exist there.
        for file in &to_add {
            if overlay_source_files.contains(file) {
                let source_file = source_path.join(file);
                let target_file = target.join(file);
                if let Some(parent) = target_file.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&source_file, &target_file).with_context(|| {
                    format!("Failed to copy {} from overlay source", file.display())
                })?;
            }
        }
        add_files_to_overlay(name_arg, &target, &to_add, false)?;
    }

    Ok(())
}

pub(crate) fn remove_files_from_overlay(
    name_arg: &str,
    target: &std::path::Path,
    files: &[PathBuf],
    dry_run: bool,
) -> Result<()> {
    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        bail!(
            "Target directory is not a git repository: {}",
            target.display()
        );
    }

    // Extract overlay name from the argument (handles both short and full forms)
    let overlay_name = extract_overlay_name(name_arg)?;

    let normalized_name = normalize_overlay_name(&overlay_name)?;
    let applied_overlays = list_applied_overlays(&target)?;

    if !applied_overlays
        .iter()
        .any(|n| n == normalized_name.as_str())
    {
        let names: Vec<&str> = applied_overlays.iter().map(OverlayName::as_str).collect();
        bail!(
            "Overlay '{}' is not currently applied.\n\n\
             Applied overlays: {}",
            overlay_name,
            if applied_overlays.is_empty() {
                "(none)".to_string()
            } else {
                names.join(", ")
            }
        );
    }

    let mut state = load_overlay_state(&target, &normalized_name)?;

    // Normalize trailing slashes and validate all files are managed by this overlay
    let files: Vec<PathBuf> = files
        .iter()
        .map(|f| {
            let s = f.to_string_lossy();
            let trimmed = s.trim_end_matches('/');
            PathBuf::from(trimmed)
        })
        .collect();

    for file in &files {
        let file_normalized = file.to_string_lossy().replace('\\', "/");
        if !state
            .file_entries()
            .iter()
            .any(|e| e.target.to_string_lossy().replace('\\', "/") == file_normalized)
        {
            let managed: Vec<String> = state
                .file_entries()
                .iter()
                .map(|e| e.target.to_string_lossy().into_owned())
                .collect();
            bail!(
                "File '{}' is not managed by overlay '{}'.\n\n\
                 Files in this overlay: {}",
                file.display(),
                normalized_name,
                managed.join(", ")
            );
        }
    }

    println!(
        "{} files from overlay: {}",
        "Removing".red().bold(),
        normalized_name
    );

    if dry_run {
        println!("  Target: {}", target.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        println!("\nFiles that would be removed:");
        for file in &files {
            println!("  {} {}", "-".red(), file.display());
        }
        return Ok(());
    }

    let mut removed_count = 0;

    for file in &files {
        let file_path = target.join(file);

        // Capture entry type before removing from state — validation above
        // guarantees the entry exists, so expect() is safe here.
        let entry_type = state
            .file_entries()
            .iter()
            .find(|e| e.target == *file)
            .expect("validated file must exist in state")
            .entry_type;

        if file_path.exists() || file_path.is_symlink() {
            if entry_type == EntryType::Directory {
                if file_path.is_symlink() {
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
                    fs::remove_dir_all(&file_path).with_context(|| {
                        format!("Failed to remove directory: {}", file_path.display())
                    })?;
                }
                println!("  {} {}/", "-".red(), file.display());
            } else {
                fs::remove_file(&file_path)
                    .with_context(|| format!("Failed to remove: {}", file_path.display()))?;
                println!("  {} {}", "-".red(), file.display());
            }

            // Clean up empty parent directories
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

        // Remove from state and add to exclusions
        state.remove_file(file);
        state.add_exclusion(file.clone(), entry_type);
        removed_count += 1;
    }

    // Rebuild exclude entries from remaining files
    let remaining_entries: Vec<String> = state
        .file_entries()
        .iter()
        .map(|e| {
            let path = e.target.to_string_lossy().replace('\\', "/");
            match e.entry_type {
                EntryType::Directory => format!("{path}/"),
                EntryType::File => path,
            }
        })
        .collect();

    // Rewrite git exclude with remaining entries
    update_git_exclude(&target, &normalized_name, &remaining_entries, true)?;

    // Save updated state
    save_overlay_state(&target, &state)?;

    // Save external backup — must succeed so exclusions persist across remove/reapply
    save_external_state(&target, &normalized_name, &state)
        .context("Failed to save external state backup")?;

    println!(
        "\n{} Removed {} file(s) from overlay '{}'",
        "done".green().bold(),
        removed_count,
        normalized_name
    );

    Ok(())
}

/// Add files to an existing applied overlay.
///
/// This adds new files to an overlay that is already applied to the target repository.
/// The files are linked to the overlay source and the overlay state is updated.
///
/// Source-type-aware behavior (#148):
/// - **`OverlayRepo`**: copies files to overlay repo, creates symlinks, auto-commits
/// - **Local**: copies files to local overlay directory, creates symlinks
/// - **GitHub**: rejected (read-only source)
///
/// File operations are performed atomically: if any step fails, all changes
/// are rolled back to prevent leaving the target in a half-modified state.
pub(crate) fn add_files_to_overlay(
    name_arg: &str,
    target: &std::path::Path,
    files: &[PathBuf],
    dry_run: bool,
) -> Result<()> {
    // Validate target is a git repo
    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        let target_display = target.display();
        bail!("Target directory is not a git repository: {target_display}");
    }

    // Check that files were provided
    if files.is_empty() {
        bail!(
            "No files specified.\n\n\
             Usage: repoverlay edit add <overlay-name> <file> [file...]"
        );
    }

    // Extract overlay name from the argument (handles both short and full forms)
    let overlay_name = extract_overlay_name(name_arg)?;

    // Verify the overlay is currently applied
    let normalized_name = normalize_overlay_name(&overlay_name)?;
    let applied_overlays = list_applied_overlays(&target)?;

    if !applied_overlays
        .iter()
        .any(|n| n == normalized_name.as_str())
    {
        let names: Vec<_> = applied_overlays
            .iter()
            .map(|n| format!("  - {n}"))
            .collect();
        bail!(
            "Overlay '{overlay_name}' is not currently applied.\n\n\
             Applied overlays:\n{}",
            if applied_overlays.is_empty() {
                "  (none)".to_string()
            } else {
                names.join("\n")
            }
        );
    }

    // Load existing overlay state
    let mut state = load_overlay_state(&target, &normalized_name)?;
    crate::try_upgrade_github_source(&target, &mut state)?;

    // Check source mutability upfront before any filesystem changes (#148)
    if state.source.is_library() {
        bail!(
            "Cannot add files to a library overlay.\n\n\
             Library overlays are managed in the repository's overlay library.\n\
             Add files to the overlay directory in the library,\n\
             then re-apply with: repoverlay apply {normalized_name}"
        );
    }
    if !state.source.is_mutable() {
        let label = state.source.source_type_label();
        bail!(
            "Cannot add files to a {label} overlay (read-only source).\n\n\
             {label} overlays are cached read-only. Use a local or overlay repo source instead."
        );
    }

    // Validate all files exist before any mutations
    for file in files {
        let full_path = target.join(file);
        if !full_path.exists() {
            bail!(
                "File does not exist: {}\n\n\
                 Create the file first, then add it to the overlay.",
                file.display()
            );
        }
    }

    // Load all existing overlay targets to check for conflicts
    let existing_targets = load_all_overlay_targets(&target)?;

    // Check that files aren't already managed by an overlay
    for file in files {
        let file_str = file.to_string_lossy().replace('\\', "/");
        if let Some(other_overlay) = existing_targets.get(&file_str) {
            bail!(
                "File '{}' is already managed by overlay '{}'.\n\
                 Remove it from that overlay first.",
                file.display(),
                other_overlay
            );
        }
    }

    println!(
        "{} files to overlay: {}",
        "Adding".blue().bold(),
        overlay_name
    );

    if dry_run {
        println!("  Target: {}", target.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        println!("\nFiles that would be added:");
        for file in files {
            println!("  {} {}", "+".green(), file.display());
        }
        return Ok(());
    }

    // Resolve the overlay source to a local path using the SourceResolver trait (#149).
    // This correctly handles Local, OverlayRepo (with source_name), and GitHub sources.
    let overlay_repo_path = state
        .source
        .resolve_local_path()
        .with_context(|| format!("Failed to resolve source path for overlay '{overlay_name}'"))?;

    if !overlay_repo_path.exists() {
        bail!(
            "Overlay source directory not found: {}\n\n\
             Did you mean to use 'repoverlay create {name_arg}' instead?",
            overlay_repo_path.display()
        );
    }

    // Determine link type (symlink unless on Windows)
    let link_type = if cfg!(windows) {
        LinkType::Copy
    } else {
        LinkType::Symlink
    };

    // Validate all file copy destinations are writable before mutating anything.
    // This catches permission errors early to avoid partial mutations.
    for file in files {
        let overlay_file = overlay_repo_path.join(file);
        if let Some(parent) = overlay_file.parent() {
            // Verify we can create the parent directory
            if !parent.exists() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Cannot create directory in overlay source: {}",
                        parent.display()
                    )
                })?;
            }
        }
    }

    let mut completed: Vec<RollbackEntry> = Vec::new();
    let mut added_count = 0;

    let result: Result<()> = (|| {
        for file in files {
            let target_file = target.join(file);
            let overlay_file = overlay_repo_path.join(file);
            let is_dir = target_file.is_dir();

            // Ensure overlay parent directory exists
            if is_dir {
                if let Some(parent) = overlay_file.parent() {
                    fs::create_dir_all(parent)?;
                }
            } else if let Some(parent) = overlay_file.parent() {
                fs::create_dir_all(parent)?;
            }

            if is_dir {
                // Copy directory tree to overlay source
                fs::create_dir_all(&overlay_file)?;
                crate::overlay_repo::copy_dir_recursive(&target_file, &overlay_file).with_context(
                    || {
                        format!(
                            "Failed to copy directory {} to overlay source",
                            target_file.display()
                        )
                    },
                )?;

                // Remove original directory
                fs::remove_dir_all(&target_file).with_context(|| {
                    format!(
                        "Failed to remove directory {} for linking",
                        target_file.display()
                    )
                })?;

                // Create symlink/copy from overlay source to target
                match link_type {
                    LinkType::Symlink => {
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(&overlay_file, &target_file).with_context(
                            || {
                                format!(
                                    "Failed to create directory symlink: {}",
                                    target_file.display()
                                )
                            },
                        )?;
                        #[cfg(windows)]
                        std::os::windows::fs::symlink_dir(&overlay_file, &target_file)
                            .with_context(|| {
                                format!(
                                    "Failed to create directory symlink: {}",
                                    target_file.display()
                                )
                            })?;
                    }
                    LinkType::Copy | LinkType::Merged => {
                        fs::create_dir_all(&target_file).with_context(|| {
                            format!("Failed to create directory: {}", target_file.display())
                        })?;
                        crate::overlay_repo::copy_dir_recursive(&overlay_file, &target_file)
                            .with_context(|| {
                                format!("Failed to copy directory: {}", target_file.display())
                            })?;
                    }
                }

                completed.push(RollbackEntry::Directory {
                    target: target_file.clone(),
                    overlay: overlay_file,
                });

                state.add_file(FileEntry {
                    source: file.clone(),
                    target: file.clone(),
                    link_type,
                    entry_type: EntryType::Directory,
                });

                println!("  {} {}/", "+".green(), file.display());
            } else {
                // Read original content before any mutations (for rollback)
                let original_content = fs::read(&target_file).with_context(|| {
                    format!("Failed to read {} for backup", target_file.display())
                })?;

                // Copy file to overlay source directory
                fs::copy(&target_file, &overlay_file).with_context(|| {
                    format!("Failed to copy {} to overlay source", target_file.display())
                })?;

                // Remove original file (we'll replace it with symlink)
                fs::remove_file(&target_file).with_context(|| {
                    format!("Failed to remove {} for linking", target_file.display())
                })?;

                // Create symlink/copy from overlay source to target
                match link_type {
                    LinkType::Symlink => {
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(&overlay_file, &target_file).with_context(
                            || format!("Failed to create symlink: {}", target_file.display()),
                        )?;
                        #[cfg(windows)]
                        std::os::windows::fs::symlink_file(&overlay_file, &target_file)
                            .with_context(|| {
                                format!("Failed to create symlink: {}", target_file.display())
                            })?;
                    }
                    LinkType::Copy | LinkType::Merged => {
                        fs::copy(&overlay_file, &target_file).with_context(|| {
                            format!("Failed to copy file: {}", target_file.display())
                        })?;
                    }
                }

                completed.push(RollbackEntry::File {
                    target: target_file.clone(),
                    overlay: overlay_file,
                    original_content,
                });

                state.add_file(FileEntry {
                    source: file.clone(),
                    target: file.clone(),
                    link_type,
                    entry_type: EntryType::File,
                });

                println!("  {} {}", "+".green(), file.display());
            }

            added_count += 1;
        }

        Ok(())
    })();

    // On failure, roll back all completed operations
    if let Err(ref e) = result {
        eprintln!(
            "\n{} Rolling back {} file(s) due to error: {}",
            "Error:".red().bold(),
            completed.len(),
            e
        );
        for entry in completed.iter().rev() {
            match entry {
                RollbackEntry::File {
                    target: target_file,
                    overlay: overlay_file,
                    original_content,
                } => {
                    // Remove the symlink/copy we created
                    if target_file.exists() || target_file.is_symlink() {
                        let _ = fs::remove_file(target_file);
                    }
                    // Restore original file content
                    let _ = fs::write(target_file, original_content);
                    // Remove the copy in overlay source
                    let _ = fs::remove_file(overlay_file);
                }
                RollbackEntry::Directory {
                    target: target_file,
                    overlay: overlay_file,
                } => {
                    // Remove the symlink/copy we created
                    if target_file.is_symlink() {
                        let _ = fs::remove_file(target_file);
                    } else if target_file.exists() {
                        let _ = fs::remove_dir_all(target_file);
                    }
                    // Restore directory from overlay copy
                    let _ = fs::create_dir_all(target_file);
                    let _ = crate::overlay_repo::copy_dir_recursive(overlay_file, target_file);
                    // Remove the copy in overlay source
                    let _ = fs::remove_dir_all(overlay_file);
                }
            }
        }
        return result;
    }

    // Rebuild full exclude list from state (which now includes both old and new files)
    // Also clear any exclusions for files that were just added back
    for file in files {
        state.remove_exclusion(file);
    }
    let all_exclude_entries: Vec<String> = state
        .file_entries()
        .iter()
        .map(|e| {
            let path = e.target.to_string_lossy().replace('\\', "/");
            match e.entry_type {
                EntryType::Directory => format!("{path}/"),
                EntryType::File => path,
            }
        })
        .collect();
    update_git_exclude(&target, &normalized_name, &all_exclude_entries, true)?;

    // Save updated overlay state
    save_overlay_state(&target, &state)?;

    // Save external backup
    if let Err(e) = save_external_state(&target, &normalized_name, &state) {
        eprintln!(
            "  {} Could not save external backup: {}",
            "Warning:".yellow(),
            e
        );
    }

    println!(
        "\n{} Added {} file(s) to overlay '{}'",
        "✓".green().bold(),
        added_count,
        overlay_name
    );

    // Auto-commit to overlay repo (only for OverlayRepo sources)
    if let OverlaySource::OverlayRepo {
        org,
        repo,
        source_name,
        ..
    } = &state.source
    {
        let config = load_config(None)?;
        let overlay_config = config.get_overlay_repo_config_by_name(source_name.as_deref())?;
        let manager = OverlayRepoManager::new(overlay_config)?;
        auto_commit_overlay(&manager, org, repo, &overlay_name, false)?;
    }

    Ok(())
}
