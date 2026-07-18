use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;

use super::create::{auto_commit_overlay, parse_overlay_name_arg};
use crate::config::load_config;
use crate::overlay_repo::OverlayRepoManager;
use crate::selection::{FlatSelectionConfig, SelectableItem, ToSelectableItem, select_flat};
use crate::state::OverlaySource;
use crate::{
    canonicalize_path, list_applied_overlays, load_overlay_state, normalize_overlay_name,
    selection::is_interactive,
};

/// Handle the sync command, dispatching to single or all-overlay sync.
pub(crate) fn handle_sync(
    target: &std::path::Path,
    name: Option<String>,
    sync_all: bool,
    dry_run: bool,
) -> Result<()> {
    // Validate target is a git repo
    let target = canonicalize_path(target, "Target directory")?;
    crate::validate_git_repo(&target)?;

    if sync_all {
        let applied_overlays = list_applied_overlays(&target)?;
        if applied_overlays.is_empty() {
            println!("{} No overlays are currently applied.", "Note:".yellow());
            return Ok(());
        }

        // Lazily initialized — only created when we encounter a syncable overlay.
        // This avoids failing when no overlay repo is configured but all applied
        // overlays are local/GitHub sources (#205).
        let mut manager: Option<OverlayRepoManager> = None;

        let mut synced = 0u32;
        let mut skipped = 0u32;

        for overlay_name in &applied_overlays {
            let mut state = load_overlay_state(&target, overlay_name.as_str())?;
            crate::try_upgrade_github_source(&target, &mut state)?;

            // Check syncability via the source (#146, #149)
            if !state.source.is_syncable() {
                let label = state.source.source_type_label();
                println!(
                    "{} Skipping '{}' ({label} source, not syncable)",
                    "Warning:".yellow(),
                    overlay_name
                );
                skipped += 1;
                continue;
            }

            // Initialize the manager on first syncable overlay
            let mgr = if let Some(m) = &manager {
                m
            } else {
                let config = load_config(None)?;
                let overlay_config = config.get_default_overlay_repo_config()?;
                let m = OverlayRepoManager::new(overlay_config)?;
                m.ensure_cloned()?;
                m.pull()?;
                manager = Some(m);
                manager.as_ref().unwrap()
            };

            // OverlayRepo source — sync directly
            match &state.source {
                OverlaySource::OverlayRepo {
                    org, repo, name, ..
                } => {
                    sync_single_overlay(&target, org, name, repo, &state, mgr, dry_run)?;
                    if !dry_run {
                        auto_commit_overlay(mgr, org, repo, name, false)?;
                    }
                    synced += 1;
                }
                // Other source types are already handled by the is_syncable check above
                _ => unreachable!("is_syncable() returned true for non-OverlayRepo source"),
            }
        }

        println!();
        let check = "✓".green().bold();
        println!("{check} Synced {synced} overlay(s), skipped {skipped}");
    } else if let Some(name_arg) = name {
        // Parse the name argument to get org/repo/name
        let (detected_org, detected_repo, overlay_name) =
            parse_overlay_name_arg(&name_arg, &target)?;

        // Verify the overlay is currently applied
        let normalized_name = normalize_overlay_name(&overlay_name)?;
        let applied_overlays = list_applied_overlays(&target)?;

        if !applied_overlays
            .iter()
            .any(|n| n == normalized_name.as_str())
        {
            bail!(
                "Overlay '{overlay_name}' is not currently applied.\n\n\
                 To apply it first: repoverlay apply {detected_org}/{detected_repo}/{overlay_name}"
            );
        }

        // Load overlay state to get file mappings
        let mut state = load_overlay_state(&target, &normalized_name)?;
        crate::try_upgrade_github_source(&target, &mut state)?;

        // Check source syncability upfront (#146, #149)
        if !state.source.is_syncable() {
            let label = state.source.source_type_label();
            bail!(
                "Cannot sync overlay '{overlay_name}' ({label} source).\n\n\
                 Only overlay repo sources can be synced."
            );
        }

        // Use org/repo from saved state rather than git remote detection.
        // When an overlay was applied via upstream fallback (e.g., fork
        // alexvy86/FluidFramework resolved to upstream microsoft/FluidFramework),
        // the state records the correct upstream org/repo. Using the git-remote-
        // detected org/repo would point to the fork path which doesn't exist
        // in the overlay repo.
        let (org, repo) = match &state.source {
            OverlaySource::OverlayRepo { org, repo, .. } => (org.clone(), repo.clone()),
            _ => (detected_org, detected_repo),
        };

        // Load overlay repo config (respects source_name for multi-source configs, #147)
        let config = load_config(None)?;
        let source_name = match &state.source {
            OverlaySource::OverlayRepo { source_name, .. } => source_name.as_deref(),
            _ => None,
        };
        let overlay_config = config.get_overlay_repo_config_by_name(source_name)?;

        // Create manager, ensure cloned, and pull latest
        let manager = OverlayRepoManager::new(overlay_config)?;
        manager.ensure_cloned()?;
        manager.pull()?;

        sync_single_overlay(
            &target,
            &org,
            &overlay_name,
            &repo,
            &state,
            &manager,
            dry_run,
        )?;

        // Auto-commit
        auto_commit_overlay(&manager, &org, &repo, &overlay_name, false)?;
    } else {
        bail!(
            "Must specify an overlay name or use --all.\n\n\
             Usage:\n  \
             repoverlay sync my-overlay\n  \
             repoverlay sync --all"
        );
    }

    Ok(())
}

/// Sync a single overlay's files from the target repo back to the overlay repo.
pub(crate) fn sync_single_overlay(
    target: &std::path::Path,
    org: &str,
    overlay_name: &str,
    repo: &str,
    state: &crate::state::OverlayState,
    manager: &OverlayRepoManager,
    dry_run: bool,
) -> Result<()> {
    let overlay_repo_path = manager.path().join(org).join(repo).join(overlay_name);

    if !overlay_repo_path.exists() {
        bail!(
            "Overlay '{org}/{repo}/{overlay_name}' does not exist in overlay repo.\n\n\
             Did you mean to use 'repoverlay create {org}/{repo}/{overlay_name}' instead?"
        );
    }

    let syncing = "Syncing".blue().bold();
    println!("{syncing} overlay: {org}/{repo}/{overlay_name}");

    if dry_run {
        println!("  Target: {}", target.display());
        println!("  Repo:   {}", overlay_repo_path.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());

        // Show what would be synced
        println!("\nFiles that would be synced:");
        for entry in state.file_entries() {
            let target_file = target.join(&entry.target);

            if target_file.exists() {
                println!(
                    "  {} {} -> {}",
                    "→".cyan(),
                    entry.target.display(),
                    entry.source.display()
                );
            }
        }

        return Ok(());
    }

    // Copy files from target back to overlay repo
    let mut synced_count = 0;
    for entry in state.file_entries() {
        let target_file = target.join(&entry.target);
        let overlay_file = overlay_repo_path.join(&entry.source);

        if target_file.exists() {
            // Ensure parent directory exists
            if let Some(parent) = overlay_file.parent() {
                fs::create_dir_all(parent)?;
            }

            // Copy file
            fs::copy(&target_file, &overlay_file).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    target_file.display(),
                    overlay_file.display()
                )
            })?;

            println!("  {} {}", "→".green(), entry.source.display());
            synced_count += 1;
        }
    }

    if synced_count == 0 {
        println!("{} No files to sync.", "Note:".yellow());
    }

    Ok(())
}

/// Interactively select an applied overlay by name.
///
/// Lists all applied overlays and lets the user pick one. Bails in non-TTY
/// environments since interactive selection requires a terminal.
pub(crate) fn select_overlay_interactive(target: &std::path::Path) -> Result<String> {
    let target = canonicalize_path(target, "Target directory")?;
    let applied = list_applied_overlays(&target)?;

    if applied.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    if applied.len() == 1 {
        return Ok(applied[0].to_string());
    }

    if !is_interactive() {
        bail!(
            "Multiple overlays applied — specify which one to edit.\n\n\
             Usage:\n  \
             repoverlay edit <name>\n  \
             repoverlay edit add <name> <files>...\n  \
             repoverlay edit remove <name> <files>..."
        );
    }

    let items: Vec<SelectableItem> = applied
        .iter()
        .map(|name| name.to_selectable_item(&target))
        .collect();

    let result = select_flat(
        &items,
        &FlatSelectionConfig {
            prompt: "Select overlay to edit:".into(),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlay selected");
    }

    Ok(result.selected_ids[0].clone())
}
