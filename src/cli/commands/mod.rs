pub(crate) mod browse;
pub(crate) mod cache;
pub(crate) mod copilot;
pub(crate) mod create;
pub(crate) mod edit;
pub(crate) mod library;
pub(crate) mod marketplace;
pub(crate) mod r#move;
pub(crate) mod profile;
pub(crate) mod source;
pub(crate) mod sync;

use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::selection::{FlatSelectionConfig, SelectableItem, ToSelectableItem, select_flat};
use crate::state::{OverlaySource, SourceResolver};
use crate::{
    OVERLAYS_DIR, STATE_DIR, canonicalize_path, list_applied_overlays, remove_overlay,
    remove_single_overlay, selection::is_interactive, state, validate_git_repo,
};

/// Canonicalize and validate an optional target path (defaults to current directory).
pub(crate) fn resolve_target(target: Option<PathBuf>) -> Result<PathBuf> {
    let target_dir = target.unwrap_or_else(|| PathBuf::from("."));
    let target = canonicalize_path(&target_dir, "Target")?;
    validate_git_repo(&target)?;
    Ok(target)
}

/// Resolve an applied overlay name to its source path on disk.
///
/// Looks up the overlay in the applied state and resolves its source to a local path.
/// For library sources, resolves via the library path. For other sources, uses the
/// source resolver.
pub(crate) fn resolve_applied_overlay_source(target: &Path, name: &str) -> Result<PathBuf> {
    let applied = state::list_applied_overlays(target)?;
    if !applied.iter().any(|n| n.as_str() == name) {
        bail!("Source path not found and '{name}' is not an applied overlay");
    }

    let overlay_state = state::load_overlay_state(target, name)?;
    match &overlay_state.source {
        OverlaySource::Library { name: lib_name } => {
            let library_path = crate::library::get_library_path(target)?;
            Ok(library_path.join(lib_name))
        }
        source => source.resolve_local_path(),
    }
}

pub(crate) fn find_repo_root() -> Result<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("Failed to find git repository root")?;
    if !output.status.success() {
        bail!("Not inside a git repository");
    }
    let root = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(PathBuf::from(root))
}

/// Handle remove command with interactive selection support.
pub(crate) fn handle_remove(
    target: &std::path::Path,
    name: Option<String>,
    remove_all: bool,
    dry_run: bool,
    interactive: bool,
) -> Result<()> {
    // If name or --all is specified, use direct removal
    if remove_all || name.is_some() {
        return remove_overlay(target, name, remove_all, dry_run);
    }

    // If not interactive and no name specified, require explicit action.
    // In an interactive terminal, default to interactive mode automatically.
    if !interactive && !is_interactive() {
        bail!(
            "No overlay name specified.\n\n\
             Usage:\n  \
             repoverlay remove <name>        # Remove specific overlay\n  \
             repoverlay remove --all         # Remove all overlays\n  \
             repoverlay remove --interactive # Interactive selection"
        );
    }

    // Interactive selection
    let target = canonicalize_path(target, "Target directory")?;
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let items: Vec<SelectableItem> = applied_overlays
        .iter()
        .map(|name| name.to_selectable_item(&target))
        .collect();

    let result = select_flat(
        &items,
        &FlatSelectionConfig {
            prompt: "Select overlay(s) to remove:".into(),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlays selected for removal");
    }

    let remove_all = result.selected_ids.len() == applied_overlays.len();

    for overlay_name in &result.selected_ids {
        if dry_run {
            println!(
                "{} Dry run - would remove overlay '{overlay_name}'",
                "Note:".yellow()
            );
        } else {
            remove_single_overlay(&target, &overlays_dir, overlay_name)?;
        }
    }

    if !dry_run {
        if remove_all {
            fs::remove_dir_all(target.join(STATE_DIR))?;
            println!("\n{} Removed all overlays", "✓".green().bold());
        } else {
            let remaining = list_applied_overlays(&target)?;
            if remaining.is_empty() {
                fs::remove_dir_all(target.join(STATE_DIR))?;
            }
        }
    }

    Ok(())
}
