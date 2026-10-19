pub(crate) mod apm;
pub(crate) mod browse;
pub(crate) mod cache;
pub(crate) mod claude;
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
use std::path::{Path, PathBuf};

use crate::selection::{FlatSelectionConfig, SelectableItem, ToSelectableItem, select_flat};
use crate::state::{OverlaySource, SourceResolver};
use crate::{
    canonicalize_path, list_applied_overlays, remove_overlay, selection::is_interactive, state,
    validate_git_repo,
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
    if remove_all {
        return crate::remove::remove_all(target, dry_run);
    }
    if name.is_some() {
        return remove_overlay(target, name, false, dry_run);
    }

    // If not interactive and no name specified, require explicit action.
    // In an interactive terminal, default to interactive mode automatically.
    if !interactive && !is_interactive() {
        bail!(
            "No overlay name specified.\n\n\
             Usage:\n  \
             repoverlay remove <name>        # Remove specific overlay\n  \
             repoverlay remove --all         # Remove all overlays and profiles\n  \
             repoverlay remove --interactive # Interactive selection"
        );
    }

    // Interactive selection
    let target = canonicalize_path(target, "Target directory")?;
    let applied_overlays = list_applied_overlays(&target)?;
    let profiles = crate::profile_plan::list_profile_states(&target)?;
    if applied_overlays.is_empty() && profiles.is_empty() {
        bail!(
            "No overlays or profiles are currently applied in: {}",
            target.display()
        );
    }

    let mut items: Vec<SelectableItem> = applied_overlays
        .iter()
        .map(|name| {
            let mut item = name.to_selectable_item(&target);
            item.id = format!("overlay:{name}");
            if let Some(profile) = profiles
                .iter()
                .find(|profile| profile.overlays.contains(&name.to_string()))
            {
                item.disabled = true;
                item.description = Some(format!(
                    "Managed by profile '{}' ({}); select that profile",
                    profile.name, profile.harness
                ));
            }
            item
        })
        .collect();
    items.extend(profiles.iter().map(|profile| SelectableItem {
        id: format!("profile:{}:{}", profile.harness, profile.name),
        label: format!("{} (profile, {})", profile.name, profile.harness),
        description: Some(format!("Applied {}", profile.applied_at.to_rfc3339())),
        preselected: false,
        disabled: false,
    }));

    let result = select_flat(
        &items,
        &FlatSelectionConfig {
            prompt: "Select overlay(s) or profile(s) to remove:".into(),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlays or profiles selected for removal");
    }

    let overlays = applied_overlays
        .into_iter()
        .filter(|name| result.selected_ids.contains(&format!("overlay:{name}")))
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let profiles = profiles
        .into_iter()
        .filter(|profile| {
            result
                .selected_ids
                .contains(&format!("profile:{}:{}", profile.harness, profile.name))
        })
        .collect();
    crate::remove::remove_selected(&target, &overlays, profiles, dry_run)
}
