use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::library;
use crate::overlay_repo::copy_dir_recursive;
use crate::state::{
    EntryType, LinkType, OverlaySource, load_overlay_state, normalize_overlay_name,
    save_external_state, save_overlay_state,
};

use super::resolve_target;

/// Handle the `move` command.
pub(crate) fn handle_move_command(
    overlay: &str,
    to: &str,
    target: &Path,
    force: bool,
    name_override: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    let target = resolve_target(Some(target.to_path_buf()))?;
    let normalized_name = normalize_overlay_name(overlay)?;

    // Load current overlay state
    let mut state = load_overlay_state(&target, &normalized_name)?;

    // Resolve current source path
    let current_source_path = resolve_source_path(&target, &state.source)?;

    // Determine destination name
    let dest_name = name_override.unwrap_or(overlay);

    // Parse and resolve destination
    let destination = parse_destination(to, &target, dest_name)?;

    // Check for circular move (same location)
    if let Destination::Library { ref path } = destination {
        if let OverlaySource::Library { .. } = &state.source
            && name_override.is_none()
        {
            eprintln!(
                "{} Overlay '{}' is already in the library — nothing to do.",
                "Warning:".yellow(),
                overlay
            );
            return Ok(());
        }
        // Also check if the source path IS the destination
        if current_source_path.exists()
            && path.exists()
            && paths_equivalent(&current_source_path, path)
        {
            eprintln!(
                "{} Source and destination are the same — nothing to do.",
                "Warning:".yellow(),
            );
            return Ok(());
        }
    }

    let dest_path = destination.path();

    if dry_run {
        println!("{} Dry run — would move overlay:", "Note:".yellow());
        println!("  From: {}", current_source_path.display());
        println!("  To:   {}", dest_path.display());
        return Ok(());
    }

    // Check for conflicts at destination
    if dest_path.exists() {
        if force {
            fs::remove_dir_all(dest_path).with_context(|| {
                format!(
                    "Failed to remove existing destination: {}",
                    dest_path.display()
                )
            })?;
        } else {
            bail!(
                "Destination already exists: {}. Use --force to overwrite or --name to rename.",
                dest_path.display()
            );
        }
    }

    println!(
        "{} overlay '{}' to {}",
        "Moving".green().bold(),
        overlay,
        destination.display_label()
    );

    // Step 1: Copy to destination
    fs::create_dir_all(dest_path)
        .with_context(|| format!("Failed to create destination: {}", dest_path.display()))?;
    copy_dir_recursive(&current_source_path, dest_path)
        .with_context(|| format!("Failed to copy overlay to: {}", dest_path.display()))?;

    // Step 2: Update state source reference
    let new_source = destination.to_overlay_source(dest_name);
    state.source = new_source;

    save_overlay_state(&target, &state)?;
    // Best-effort external state update
    if let Err(e) = save_external_state(&target, &normalized_name, &state) {
        eprintln!(
            "  {} Could not update external backup: {}",
            "Warning:".yellow(),
            e
        );
    }

    // Step 3: Re-create symlinks pointing to new location
    let mut relinked = 0;
    for entry in state.file_entries() {
        if entry.link_type != LinkType::Symlink {
            continue;
        }

        let symlink_path = target.join(&entry.target);
        if !symlink_path.is_symlink() {
            continue;
        }

        let new_link_target = dest_path.join(&entry.source);

        // Remove old symlink
        #[cfg(unix)]
        {
            fs::remove_file(&symlink_path).with_context(|| {
                format!("Failed to remove old symlink: {}", symlink_path.display())
            })?;
        }
        #[cfg(windows)]
        {
            if entry.entry_type == EntryType::Directory {
                fs::remove_dir(&symlink_path).with_context(|| {
                    format!("Failed to remove old symlink: {}", symlink_path.display())
                })?;
            } else {
                fs::remove_file(&symlink_path).with_context(|| {
                    format!("Failed to remove old symlink: {}", symlink_path.display())
                })?;
            }
        }

        // Create new symlink
        match entry.entry_type {
            EntryType::File => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&new_link_target, &symlink_path).with_context(|| {
                    format!("Failed to create symlink: {}", symlink_path.display())
                })?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(&new_link_target, &symlink_path).with_context(
                    || format!("Failed to create symlink: {}", symlink_path.display()),
                )?;
            }
            EntryType::Directory => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&new_link_target, &symlink_path).with_context(|| {
                    format!("Failed to create symlink: {}", symlink_path.display())
                })?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(&new_link_target, &symlink_path).with_context(
                    || format!("Failed to create symlink: {}", symlink_path.display()),
                )?;
            }
        }

        relinked += 1;
    }

    // Step 4: Delete from source location
    if current_source_path.exists() {
        fs::remove_dir_all(&current_source_path).with_context(|| {
            format!(
                "Failed to remove old source: {}",
                current_source_path.display()
            )
        })?;
    }

    println!(
        "\n{} Moved '{}' to {}",
        "✓".green().bold(),
        overlay,
        dest_path.display()
    );

    if relinked > 0 {
        println!("  Re-linked {relinked} symlink(s)");
    }

    Ok(())
}

/// Parsed destination for a move operation.
enum Destination {
    Library { path: PathBuf },
    Path { path: PathBuf },
}

impl Destination {
    fn path(&self) -> &Path {
        match self {
            Self::Library { path } | Self::Path { path } => path,
        }
    }

    fn display_label(&self) -> String {
        match self {
            Self::Library { path } => format!("library ({})", path.display()),
            Self::Path { path } => path.display().to_string(),
        }
    }

    fn to_overlay_source(&self, name: &str) -> OverlaySource {
        match self {
            Self::Library { .. } => OverlaySource::library(name.to_string()),
            Self::Path { path } => OverlaySource::local(path.clone()),
        }
    }
}

/// Parse the `--to` argument into a resolved destination.
fn parse_destination(to: &str, target: &Path, dest_name: &str) -> Result<Destination> {
    if to == "library" {
        let library_path = library::get_library_path(target)?;
        let dest = library_path.join(dest_name);
        Ok(Destination::Library { path: dest })
    } else if let Some(source_name) = to.strip_prefix("source:") {
        bail!(
            "Moving to named source '{source_name}' is not yet implemented. \
             Use a filesystem path instead."
        );
    } else {
        let base = PathBuf::from(to);
        let dest = base.join(dest_name);
        Ok(Destination::Path { path: dest })
    }
}

/// Resolve the filesystem path for the current overlay source.
fn resolve_source_path(target: &Path, source: &OverlaySource) -> Result<PathBuf> {
    match source {
        OverlaySource::Library { name } => {
            let library_path = library::get_library_path(target)?;
            Ok(library_path.join(name))
        }
        OverlaySource::Local { path } => Ok(path.clone()),
        OverlaySource::GitHub { .. } => {
            bail!("Cannot move a GitHub-sourced overlay. Import it to the library first.")
        }
        OverlaySource::OverlayRepo { .. } => {
            bail!(
                "Cannot move an overlay-repo-sourced overlay directly. \
                 Import it to the library first."
            )
        }
    }
}

/// Check if two paths point to the same location.
fn paths_equivalent(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
