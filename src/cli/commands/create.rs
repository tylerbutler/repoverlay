use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::load_config;
use crate::overlay_repo::OverlayRepoManager;
use crate::{canonicalize_path, library, parse_github_owner_repo, selection::is_interactive};

/// Detect org/repo from git remote origin.
pub(crate) fn detect_target_repo(path: &Path) -> Result<(String, String)> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(path)
        .output()
        .context("Failed to get git remote")?;

    if !output.status.success() {
        bail!(
            "Could not detect target repository from git remote.\n\
             Please specify explicitly: repoverlay create org/repo/name"
        );
    }

    let url = String::from_utf8(output.stdout)?.trim().to_string();
    parse_github_owner_repo(&url)
}

/// Extract just the overlay name from a name argument.
///
/// Handles both short form (`my-overlay`) and full form (`org/repo/my-overlay`),
/// returning only the overlay name portion. Unlike [`parse_overlay_name_arg`],
/// this does not require a git remote to resolve org/repo.
pub(crate) fn extract_overlay_name(name_arg: &str) -> Result<String> {
    let slash_count = name_arg.chars().filter(|c| *c == '/').count();

    match slash_count {
        0 => Ok(name_arg.to_string()),
        2 => {
            let parts: Vec<&str> = name_arg.split('/').collect();
            if parts.iter().any(|p| p.is_empty()) {
                bail!(
                    "Invalid overlay path format: {name_arg}\n\n\
                     Use one of:\n  \
                     - my-overlay (overlay name)\n  \
                     - org/repo/my-overlay (explicit)"
                );
            }
            Ok(parts[2].to_string())
        }
        _ => {
            bail!(
                "Invalid overlay path format: {name_arg}\n\n\
                 Use one of:\n  \
                 - my-overlay (overlay name)\n  \
                 - org/repo/my-overlay (explicit)"
            );
        }
    }
}

/// Human-readable overlay label: `org/name` for globals (empty repo), else `org/repo/name`.
fn overlay_label(org: &str, repo: &str, name: &str) -> String {
    if repo.is_empty() {
        format!("{org}/{name}")
    } else {
        format!("{org}/{repo}/{name}")
    }
}

/// Parse an overlay name argument.
///
/// Returns (org, repo, name) tuple.
/// - If the argument contains 2 slashes, parses as org/repo/name
/// - If no slashes, detects org/repo from git remote
/// - If 1 slash, returns an error (invalid format)
pub(crate) fn parse_overlay_name_arg(
    name_arg: &str,
    source_path: &Path,
) -> Result<(String, String, String)> {
    let overlay_name = extract_overlay_name(name_arg)?;

    let slash_count = name_arg.chars().filter(|c| *c == '/').count();
    match slash_count {
        0 => {
            // Short form: detect org/repo from git remote
            let (org, repo) = detect_target_repo(source_path)?;
            Ok((org, repo, overlay_name))
        }
        2 => {
            // Full form: org/repo/name — extract org and repo
            let parts: Vec<&str> = name_arg.split('/').collect();
            Ok((parts[0].to_string(), parts[1].to_string(), overlay_name))
        }
        _ => unreachable!("extract_overlay_name already validates slash count"),
    }
}

/// Create an overlay directly into the in-repo library.
///
/// After creation, prompts to apply the overlay (unless `--yes` or `--no-apply`).
#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
pub(crate) fn create_into_library(
    source: &Path,
    target: &Path,
    name: Option<String>,
    include: &[PathBuf],
    dry_run: bool,
    yes: bool,
    no_apply: bool,
    force: bool,
) -> Result<()> {
    // Validate source is a git repo
    if !source.join(".git").exists() {
        bail!(
            "Source directory is not a git repository: {}",
            source.display()
        );
    }

    crate::validate_git_repo(target)?;

    let source = canonicalize_path(source, "Source")?;
    let target = canonicalize_path(target, "Target")?;
    let library_path = library::get_library_path(&target)?;

    // Determine overlay name
    let overlay_name = name.unwrap_or_else(|| "overlay".to_string());
    let output_path = library_path.join(&overlay_name);

    // Check if overlay already exists
    if output_path.exists() && !force {
        bail!("Overlay '{overlay_name}' already exists in the library. Use --force to overwrite.");
    }

    // Auto-fix gitignore if library path is ignored
    if library::ensure_library_not_gitignored(&target, &library_path)? {
        eprintln!(
            "{} Updated .gitignore to track library path {}",
            "Note:".cyan().bold(),
            library_path
                .strip_prefix(&target)
                .unwrap_or(&library_path)
                .display()
        );
    }

    if dry_run {
        println!(
            "{} Would create overlay '{}' in library at {}",
            "Dry run:".yellow().bold(),
            overlay_name,
            output_path.display()
        );
        return Ok(());
    }

    // If force and exists, remove existing first
    if output_path.exists() && force {
        fs::remove_dir_all(&output_path)?;
    }

    // Create the overlay into the library path
    crate::create_overlay(
        &source,
        Some(output_path.clone()),
        include,
        Some(overlay_name.clone()),
        dry_run,
        yes,
    )?;

    // Prompt to apply (unless --no-apply)
    if !no_apply {
        let should_apply = if yes {
            true
        } else if is_interactive() {
            print!("Apply it now? [Y/n] ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let trimmed = input.trim();
            trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y")
        } else {
            // Non-interactive, default to applying
            true
        };

        if should_apply {
            let overlay_source = output_path.to_string_lossy().to_string();
            crate::apply_overlay(
                &overlay_source,
                &target,
                &crate::ApplyOptions {
                    name_override: Some(overlay_name),
                    conflict_strategy: crate::ConflictStrategy::Force,
                    ..crate::ApplyOptions::default()
                },
            )?;
        }
    }

    Ok(())
}

/// Handle the create command with the new argument structure.
///
/// This function handles:
/// - `create <name>` - create in overlay repo, auto-detect org/repo
/// - `create org/repo/name` - create in overlay repo at explicit path
/// - `create --local ./output` - create in local directory only
///
/// After creating the overlay, it is automatically applied to the source
/// repository (symlinks replace originals, state saved, git exclude updated).
#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
pub(crate) fn create_overlay_command(
    source: &Path,
    name_arg: Option<String>,
    local: Option<PathBuf>,
    include: &[PathBuf],
    global: bool,
    dry_run: bool,
    yes: bool,
    force: bool,
) -> Result<()> {
    // Validate source is a git repo
    if !source.join(".git").exists() {
        bail!(
            "Source directory is not a git repository: {}",
            source.display()
        );
    }

    // Handle --local mode (write to local directory)
    if let Some(local_path) = local {
        // When a name is provided, create a subdirectory: output/<name>/
        let (output_path, overlay_name) = if let Some(ref name) = name_arg {
            (local_path.join(name), Some(name.clone()))
        } else {
            (local_path, None)
        };

        // Use existing create_overlay function for local mode
        crate::create_overlay(
            source,
            Some(output_path.clone()),
            include,
            overlay_name,
            dry_run,
            yes,
        )?;

        // Auto-apply the newly created overlay back to the source repo
        if !dry_run {
            let overlay_source = output_path.to_string_lossy().to_string();
            crate::apply_overlay(
                &overlay_source,
                source,
                &crate::ApplyOptions {
                    conflict_strategy: crate::ConflictStrategy::Force,
                    ..crate::ApplyOptions::default()
                },
            )?;
        }

        return Ok(());
    }

    // For overlay repo mode, we need the name argument
    let name_arg = name_arg.ok_or_else(|| {
        anyhow::anyhow!(
            "Missing overlay name.\n\n\
             Usage:\n  \
             repoverlay create my-overlay          # Detects org/repo from git remote\n  \
             repoverlay create org/repo/my-overlay # Explicit target\n  \
             repoverlay create --output ./output   # Write to local directory"
        )
    })?;

    // Parse the name argument. Global overlays live in the @global namespace and
    // do not need git-remote org/repo detection.
    let (org, repo, overlay_name) = if global {
        if name_arg.contains('/') {
            bail!(
                "A global overlay takes a bare name, not an org/repo path: \
                 repoverlay create <name> --global"
            );
        }
        let overlay_name = extract_overlay_name(&name_arg)?;
        (
            crate::library::GLOBAL_NAMESPACE.to_string(),
            String::new(),
            overlay_name,
        )
    } else {
        parse_overlay_name_arg(&name_arg, source)?
    };

    // Human-readable target label (`@global/name` for globals, else `org/repo/name`).
    let target_label = overlay_label(&org, &repo, &overlay_name);

    // Load overlay repo config
    let config = load_config(None)?;
    let overlay_config = config.get_default_overlay_repo_config()?;

    // Create manager, ensure cloned, and pull latest
    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;
    manager.pull()?;

    // Determine output path in overlay repo (empty repo segment is a no-op for globals)
    let output_path = manager.path().join(&org).join(&repo).join(&overlay_name);

    // Check if overlay already exists
    if output_path.exists() && !force {
        bail!(
            "Overlay '{target_label}' already exists.\n\n\
             To update an applied overlay, use: repoverlay sync {overlay_name}\n\
             To overwrite, use: repoverlay create {name_arg} --force"
        );
    }

    println!(
        "{} Creating overlay: {}",
        "Create".blue().bold(),
        target_label
    );

    if dry_run {
        println!("  Source:  {}", source.display());
        println!("  Target:  {}", output_path.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        return Ok(());
    }

    // If includes not specified, use discovery/interactive mode
    if include.is_empty() {
        // Use the existing discovery logic from create_overlay
        crate::create_overlay(
            source,
            Some(output_path.clone()),
            include,
            Some(overlay_name.clone()),
            dry_run,
            yes,
        )?;

        // Auto-commit after creating
        auto_commit_overlay(&manager, &org, &repo, &overlay_name, true)?;

        // Auto-apply the overlay back to the source repo
        let overlay_source = output_path.to_string_lossy().to_string();
        crate::apply_overlay(
            &overlay_source,
            source,
            &crate::ApplyOptions {
                name_override: Some(overlay_name),
                conflict_strategy: crate::ConflictStrategy::Force,
                ..crate::ApplyOptions::default()
            },
        )?;

        return Ok(());
    }

    // Expand globs and validate include paths
    let expanded = crate::expand_include_globs(source, include)?;

    // If force and exists, remove existing first
    if output_path.exists() && force {
        fs::remove_dir_all(&output_path)?;
    }

    // Copy files and create overlay
    let copied_files = crate::copy_files_to_overlay(source, &output_path, &expanded)?;

    // Generate config
    fs::write(
        output_path.join("repoverlay.ccl"),
        crate::generate_overlay_config(&overlay_name),
    )?;

    crate::print_overlay_created(&output_path, &copied_files);

    // Auto-commit
    auto_commit_overlay(&manager, &org, &repo, &overlay_name, true)?;

    // Auto-apply the overlay back to the source repo
    let overlay_source = output_path.to_string_lossy().to_string();
    crate::apply_overlay(
        &overlay_source,
        source,
        &crate::ApplyOptions {
            name_override: Some(overlay_name),
            conflict_strategy: crate::ConflictStrategy::Force,
            ..crate::ApplyOptions::default()
        },
    )?;

    Ok(())
}

/// Auto-commit changes to an overlay in the overlay repo.
pub(crate) fn auto_commit_overlay(
    manager: &OverlayRepoManager,
    org: &str,
    repo: &str,
    name: &str,
    is_new: bool,
) -> Result<()> {
    // Fetch latest from remote before committing to avoid divergence
    let fetch_result = crate::git::run_git_with_spinner(
        &["fetch", "origin"],
        Some(manager.path()),
        "Fetching from remote...",
        false,
    );

    match fetch_result {
        Ok((status, _)) if status.success() => {
            // Try to pull/rebase to incorporate remote changes
            let pull_result = crate::git::run_git_with_spinner(
                &["pull", "--rebase", "--autostash"],
                Some(manager.path()),
                "Pulling latest changes...",
                false,
            );

            match pull_result {
                Ok((status, _)) if !status.success() => {
                    eprintln!(
                        "{} Could not pull latest changes, continuing...",
                        "Warning:".yellow(),
                    );
                }
                Err(e) => {
                    eprintln!("{} Could not pull latest changes: {e}", "Warning:".yellow(),);
                }
                _ => {}
            }
        }
        _ => {
            // Fetch failed, but continue - might be offline
            eprintln!(
                "{} Could not fetch from remote (offline?), continuing...",
                "Warning:".yellow()
            );
        }
    }

    // Check if there are changes to commit
    if !manager.has_staged_changes()? {
        // Stage all changes
        let output = Command::new("git")
            .args(["add", "."])
            .current_dir(manager.path())
            .output()
            .context("Failed to stage changes")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr.trim();
            bail!("Failed to stage changes: {msg}");
        }
    }

    // Check again if there are staged changes
    if !manager.has_staged_changes()? {
        println!("{} No changes to commit.", "Note:".yellow());
        return Ok(());
    }

    let action = if is_new { "Add" } else { "Update" };
    let label = overlay_label(org, repo, name);
    let commit_msg = format!("{action} overlay: {label}");

    println!("{} changes...", "Committing".blue().bold());
    manager.commit(&commit_msg)?;

    // Auto-push to remote
    let push_result = crate::git::run_git_with_spinner(
        &["push"],
        Some(manager.path()),
        "Pushing to remote...",
        false,
    );

    match push_result {
        Ok((status, _)) if status.success() => {
            let check = "✓".green().bold();
            let action_word = if is_new { "created" } else { "updated" };
            println!("\n{check} Overlay {action_word}: {label}");
        }
        Ok(_) | Err(_) => {
            let warn = "Warning:".yellow();
            eprintln!("\n{warn} Committed locally but failed to push.");
            eprintln!("Run 'repoverlay push' to push manually when online.");
        }
    }

    println!("To apply: repoverlay apply {org}/{repo}/{name}");

    Ok(())
}
