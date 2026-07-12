//! Update applied overlays from their remote sources.

use anyhow::{Result, bail};
use colored::Colorize;
use log::debug;
use std::path::{Path, PathBuf};

use crate::ApplyOptions;
use crate::ConflictStrategy;
use crate::OverlayName;
use crate::apply_overlay;
use crate::cache::CacheManager;
use crate::canonicalize_path;
use crate::github::GitHubSource;
use crate::remove_single_overlay;
use crate::state::{
    OVERLAYS_DIR, OverlaySource, STATE_DIR, list_applied_overlays, load_overlay_state,
    normalize_overlay_name,
};

/// Update applied overlays from remote sources.
///
/// Only GitHub-sourced overlays can be updated. Local overlays are skipped.
///
/// # Workflow
///
/// 1. List applied overlays (optionally filtered by name)
/// 2. For each GitHub overlay, check remote for new commits
/// 3. Report available updates
/// 4. If not dry-run, remove and re-apply each overlay with updated cache
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn update_overlays(
    target: &Path,
    name: Option<String>,
    dry_run: bool,
    conflict_strategy: ConflictStrategy,
    merge: bool,
) -> Result<()> {
    debug!(
        "update_overlays: target={}, name={:?}, dry_run={}, conflict_strategy={:?}",
        target.display(),
        name,
        dry_run,
        conflict_strategy
    );
    let target = canonicalize_path(target, "Target directory")?;
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    // Filter to just the specified overlay if name provided
    let overlays_to_check: Vec<OverlayName> = if let Some(ref name) = name {
        let normalized = normalize_overlay_name(name)?;
        if !applied_overlays.iter().any(|n| n == normalized.as_str()) {
            let names: Vec<&str> = applied_overlays.iter().map(OverlayName::as_str).collect();
            bail!(
                "Overlay '{}' is not applied. Available: {}",
                name,
                names.join(", ")
            );
        }
        vec![OverlayName::new(normalized)]
    } else {
        applied_overlays
    };

    let cache = CacheManager::new()?;
    let mut updates_available = Vec::new();

    // Check for updates
    for overlay_name in &overlays_to_check {
        let state = load_overlay_state(&target, overlay_name.as_str())?;

        if let OverlaySource::GitHub {
            owner,
            repo,
            git_ref,
            commit,
            subpath,
            url,
            ..
        } = &state.source
        {
            let source = GitHubSource {
                owner: owner.clone(),
                repo: repo.clone(),
                git_ref: git_ref.parse().map_err(|e: String| anyhow::anyhow!(e))?,
                subpath: subpath.as_ref().map(PathBuf::from),
            };

            match cache.check_for_updates(&source) {
                Ok(Some(new_commit)) => {
                    updates_available.push((
                        overlay_name.clone(),
                        state.name.clone(),
                        url.clone(),
                        commit.clone(),
                        new_commit,
                    ));
                }
                Ok(None) => {
                    println!("  {} {} is up to date", "✓".green(), state.name);
                }
                Err(e) => {
                    println!(
                        "  {} Could not check {} for updates: {}",
                        "?".yellow(),
                        state.name,
                        e
                    );
                }
            }
        } else if state.source.is_library() {
            // Library overlays are managed in-repo — update via git
            println!(
                "  {} {} (library overlay — update via git)",
                "-".dimmed(),
                state.name,
            );
        } else if state.source.is_updatable() {
            // OverlayRepo sources: update by re-applying from the overlay repo
            println!(
                "  {} {} ({} source, update via 'repoverlay restore')",
                "-".dimmed(),
                state.name,
                state.source.source_type_label()
            );
        } else {
            println!(
                "  {} {} is a {} overlay (not updatable)",
                "-".dimmed(),
                state.name,
                state.source.source_type_label()
            );
        }
    }

    if updates_available.is_empty() {
        println!("\n{} All overlays are up to date.", "Status:".bold());
        return Ok(());
    }

    println!(
        "\n{} {} update(s) available:",
        "Found".blue().bold(),
        updates_available.len()
    );

    for (_, name, url, old_commit, new_commit) in &updates_available {
        println!("  {} {}", "↑".cyan(), name);
        println!(
            "    {}  →  {}",
            &old_commit[..7.min(old_commit.len())],
            &new_commit[..7.min(new_commit.len())]
        );
        println!("    {}", url.dimmed());
    }

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        return Ok(());
    }

    println!();

    // Apply updates
    for (normalized_name, _, _, _, _) in &updates_available {
        let state = load_overlay_state(&target, normalized_name.as_str())?;

        if let OverlaySource::GitHub { url, git_ref, .. } = &state.source {
            // Remove old overlay
            let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
            remove_single_overlay(&target, &overlays_dir, normalized_name.as_str())?;

            // Re-apply with update
            apply_overlay(
                url,
                &target,
                &ApplyOptions {
                    name_override: Some(state.name.clone()),
                    ref_override: Some(git_ref.clone()),
                    update_cache: true,
                    conflict_strategy,
                    merge,
                    ..ApplyOptions::default()
                },
            )?;
        }
    }

    Ok(())
}
