use anyhow::{Result, bail};
use colored::Colorize;
use std::path::PathBuf;

use crate::{library, state};
use super::super::LibraryCommand;
use super::{resolve_applied_overlay_source, resolve_target};

/// Handle library subcommands.
pub(crate) fn handle_library_command(command: LibraryCommand) -> Result<()> {
    match command {
        LibraryCommand::List { target } => {
            let target = resolve_target(target)?;
            let library_path = library::get_library_path(&target)?;
            let overlays = library::list_library_overlays(&library_path)?;
            if overlays.is_empty() {
                println!("No overlays in library.");
            } else {
                for overlay in &overlays {
                    println!("  {}", overlay.name);
                }
            }
        }
        LibraryCommand::Import {
            source,
            name,
            force,
            target,
        } => {
            let target = resolve_target(target)?;

            // Resolve source: try as a filesystem path first, then as an applied overlay name
            let (source_path, resolved_from_name) = {
                let candidate = PathBuf::from(&source);
                if let Ok(canonical) = candidate.canonicalize() {
                    (canonical, false)
                } else {
                    // Not a valid path — try resolving as an applied overlay name
                    let path = resolve_applied_overlay_source(&target, &source)?;
                    (path, true)
                }
            };

            if !source_path.is_dir() {
                bail!("Source is not a directory: {}", source_path.display());
            }

            let overlay_name = name.unwrap_or_else(|| {
                if resolved_from_name {
                    // Use the applied overlay name when resolved by name
                    source.clone()
                } else {
                    source_path.file_name().map_or_else(
                        || "overlay".to_string(),
                        |n| n.to_string_lossy().to_string(),
                    )
                }
            });

            let library_path = library::get_library_path(&target)?;

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

            let dest =
                library::import_to_library(&source_path, &library_path, &overlay_name, force)?;
            println!(
                "{} overlay '{}' to library at {}",
                "Imported".green().bold(),
                overlay_name,
                dest.display()
            );
        }
        LibraryCommand::Export {
            overlay,
            dest,
            target,
        } => {
            let target = resolve_target(target)?;
            let library_path = library::get_library_path(&target)?;

            let dest_path = PathBuf::from(&dest);
            let exported = library::export_from_library(&library_path, &overlay, &dest_path)?;
            println!(
                "{} overlay '{}' to {}",
                "Exported".green().bold(),
                overlay,
                exported.display()
            );
        }
        LibraryCommand::Remove {
            overlay,
            force,
            target,
        } => {
            let target = resolve_target(target)?;

            // Check if overlay is currently applied
            if !force {
                let applied = state::list_applied_overlays(&target)?;
                if applied.iter().any(|n| n.as_str() == overlay)
                    && let Ok(state) = state::load_overlay_state(&target, &overlay)
                    && state.source.is_library()
                {
                    bail!(
                        "Overlay '{overlay}' is currently applied from the library. Use --force to remove anyway, or remove the overlay first with 'repoverlay remove {overlay}'.",
                    );
                }
            }

            let library_path = library::get_library_path(&target)?;

            library::remove_from_library(&library_path, &overlay)?;
            println!(
                "{} overlay '{}' from library",
                "Removed".green().bold(),
                overlay
            );
        }
    }

    Ok(())
}
