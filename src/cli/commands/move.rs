use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::load_config;
use crate::library;
use crate::overlay_repo::copy_dir_recursive;
use crate::sources::SourceManager;
use crate::state::{
    EntryType, LinkType, OverlaySource, load_overlay_state, normalize_overlay_name,
    save_external_state, save_overlay_state,
};
use crate::upstream::detect_repo_identity;

use super::resolve_target;

/// Handle the `move` command.
pub(crate) fn handle_move_command(
    overlay: &str,
    to: &str,
    target: &Path,
    force: bool,
    name_override: Option<&str>,
    target_repo: Option<&str>,
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
    let destination = parse_destination(to, &target, dest_name, target_repo)?;

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

    if matches!(destination, Destination::SourceRepo { .. }) {
        println!(
            "\n{} Changes are not committed. Commit and push in the source repo to make them available.",
            "Note:".yellow().bold()
        );
    }

    Ok(())
}

/// Parsed destination for a move operation.
enum Destination {
    Library {
        path: PathBuf,
    },
    Path {
        path: PathBuf,
    },
    SourceRepo {
        /// Full path where the overlay will be placed: `<source-base>/org/repo/name`
        path: PathBuf,
        source_name: String,
        org: String,
        repo: String,
        overlay_name: String,
    },
}

impl Destination {
    fn path(&self) -> &Path {
        match self {
            Self::Library { path } | Self::Path { path } | Self::SourceRepo { path, .. } => path,
        }
    }

    fn display_label(&self) -> String {
        match self {
            Self::Library { path } => format!("library ({})", path.display()),
            Self::Path { path } => path.display().to_string(),
            Self::SourceRepo {
                source_name,
                org,
                repo,
                overlay_name,
                ..
            } => format!("source:{source_name} ({org}/{repo}/{overlay_name})"),
        }
    }

    fn to_overlay_source(&self, name: &str) -> OverlaySource {
        match self {
            Self::Library { .. } => OverlaySource::library(name.to_string()),
            Self::Path { path } => OverlaySource::local(path.clone()),
            Self::SourceRepo {
                source_name,
                org,
                repo,
                overlay_name,
                ..
            } => OverlaySource::OverlayRepo {
                org: org.clone(),
                repo: repo.clone(),
                name: overlay_name.clone(),
                commit: "local".to_string(),
                resolved_via: None,
                source_name: Some(source_name.clone()),
            },
        }
    }
}

/// Parse the `--to` argument into a resolved destination.
fn parse_destination(
    to: &str,
    target: &Path,
    dest_name: &str,
    target_repo: Option<&str>,
) -> Result<Destination> {
    if to == "library" {
        let library_path = library::get_library_path(target)?;
        let dest = library_path.join(dest_name);
        Ok(Destination::Library { path: dest })
    } else if let Some(source_name) = to.strip_prefix("source:") {
        resolve_source_destination(source_name, target, dest_name, target_repo)
    } else {
        let base = PathBuf::from(to);
        let dest = base.join(dest_name);
        Ok(Destination::Path { path: dest })
    }
}

/// Resolve a `source:<name>` destination to a concrete filesystem path.
fn resolve_source_destination(
    source_name: &str,
    target: &Path,
    dest_name: &str,
    target_repo: Option<&str>,
) -> Result<Destination> {
    let config = load_config(Some(target))?;
    let source_mgr = SourceManager::new(config.sources, Some(target))?;

    let source_base = source_mgr
        .get_source_base_path(source_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No source named '{source_name}' is configured. \
                 Use `repoverlay source list` to see available sources."
            )
        })?
        .to_path_buf();

    let (org, repo) = resolve_org_repo(target, target_repo)?;

    let dest = source_base.join(&org).join(&repo).join(dest_name);

    Ok(Destination::SourceRepo {
        path: dest,
        source_name: source_name.to_string(),
        org,
        repo,
        overlay_name: dest_name.to_string(),
    })
}

/// Resolve the org/repo pair from `--target-repo` flag or by detecting the git origin remote.
fn resolve_org_repo(target: &Path, target_repo: Option<&str>) -> Result<(String, String)> {
    if let Some(tr) = target_repo {
        return parse_target_repo(tr);
    }

    let identity = detect_repo_identity(target)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Could not detect the target repository org/repo from git remotes.\n\
             Specify it explicitly with --target-repo org/repo."
        )
    })?;

    identity.origin.or(identity.upstream).ok_or_else(|| {
        anyhow::anyhow!(
            "Could not parse a GitHub org/repo from the git remotes.\n\
             Specify it explicitly with --target-repo org/repo."
        )
    })
}

/// Parse a `org/repo` string into its two components.
fn parse_target_repo(target_repo: &str) -> Result<(String, String)> {
    let mut parts = target_repo.splitn(2, '/');
    let org = parts.next().unwrap_or("").trim();
    let repo = parts.next().unwrap_or("").trim();
    if org.is_empty() || repo.is_empty() {
        bail!(
            "--target-repo must be in 'org/repo' format (e.g. acme/my-app), got: '{target_repo}'"
        );
    }
    Ok((org.to_string(), repo.to_string()))
}

/// Resolve the filesystem path for the current overlay source.
fn resolve_source_path(target: &Path, source: &OverlaySource) -> Result<PathBuf> {
    match source {
        OverlaySource::Library { name } => {
            let library_path = library::get_library_path(target)?;
            Ok(library_path.join(name))
        }
        OverlaySource::Local { path, .. } => Ok(path.clone()),
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
