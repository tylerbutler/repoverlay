use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::load_config;
use crate::github::GitHubSource;
use crate::overlay_repo::{AvailableOverlay, BrowseOverlayItem};
use crate::reference::SourceReference;
use crate::selection::{FlatSelectionConfig, SelectableItem, ToSelectableItem, select_flat};
use crate::sources::SourceManager;
use crate::sources::list_overlays_in_dir;
use crate::state::{OverlaySource, ResolvedVia};
use crate::upstream::detect_repo_identity;
use crate::{
    CacheManager, ConflictStrategy, ResolvedSource, apply_multiple_overlays, canonicalize_path,
    config, get_cached_repo_commit, library, list_applied_overlays, list_overlays_from_cached_repo,
    selection::is_interactive, validate_git_repo,
};

/// Filter overlays to those matching the current repository.
///
/// When `skip_filter` is true, returns all overlays unfiltered. Otherwise, detects
/// the current repository from git remotes and returns only matching overlays.
/// Falls back to showing all overlays if detection fails or nothing matches.
///
/// Returns `(overlays, was_filtered)`.
fn auto_filter_overlays(
    overlays: Vec<AvailableOverlay>,
    skip_filter: bool,
) -> (Vec<AvailableOverlay>, bool) {
    if skip_filter {
        return (overlays, false);
    }

    let identity = PathBuf::from(".")
        .canonicalize()
        .ok()
        .and_then(|p| detect_repo_identity(&p).ok().flatten());

    let Some(identity) = identity else {
        return (overlays, false);
    };

    let matching: Vec<_> = overlays
        .iter()
        .filter(|o| {
            // Library overlays always pass through the auto-filter
            o.org == library::LIBRARY_SOURCE_NAME || identity.matches(&o.org, &o.repo)
        })
        .cloned()
        .collect();

    if matching.is_empty() {
        (overlays, false)
    } else {
        (matching, true)
    }
}

/// Print the overlay list as text (non-interactive output).
///
/// Caller must ensure `overlays` is non-empty.
fn print_overlay_list(overlays: &[AvailableOverlay], filtered: bool) {
    println!("{}\n", "Available overlays:".bold());

    // Group by org/repo
    let mut current_group: Option<(String, String)> = None;
    for overlay in overlays {
        let group = (overlay.org.clone(), overlay.repo.clone());
        if current_group.as_ref() != Some(&group) {
            if current_group.is_some() {
                println!();
            }
            if overlay.is_flat() {
                println!("{}:", "(flat)".dimmed());
            } else {
                println!("{}{}{}:", overlay.org.cyan(), "/".dimmed(), overlay.repo);
            }
            current_group = Some(group);
        }
        let config_marker = if overlay.has_config {
            ""
        } else {
            " (no config)"
        };
        println!("  - {}{}", overlay.name, config_marker.dimmed());
    }

    if filtered {
        println!(
            "\n{}",
            "Showing overlays for current repository. Use --show-all to see all.".dimmed()
        );
    }

    println!(
        "\n{}",
        "Run `repoverlay browse` in an interactive terminal to select and apply overlays.".dimmed()
    );
}

/// Browse available overlays from the overlay repository.
///
/// In interactive mode (TTY), presents a multi-select picker and applies selected
/// overlays. In non-interactive mode, prints the overlay list as text.
/// Unless `show_all` is set, overlays are auto-filtered to the current repository.
///
/// When `source` is provided, overlays are fetched directly from the given source
/// (username, owner/repo, or GitHub URL) without requiring a configured source.
#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
pub(crate) fn browse_overlays(
    source: Option<&str>,
    target_filter: Option<&str>,
    update: bool,
    target: Option<PathBuf>,
    no_interactive: bool,
    dry_run: bool,
    show_all: bool,
) -> Result<()> {
    if let Some(source_str) = source {
        return browse_ephemeral_source(
            source_str,
            target_filter,
            update,
            target,
            no_interactive,
            dry_run,
            show_all,
        );
    }

    let target_path = target.as_deref().unwrap_or_else(|| Path::new("."));
    let target_canonical = fs::canonicalize(target_path)?;

    // Load merged config (repo-local + global sources)
    let config = load_config(Some(&target_canonical))?;

    // Collect library overlays (available regardless of configured sources)
    let library_overlays = library::get_library_path(&target_canonical)
        .and_then(|lp| library::list_library_overlays(&lp))
        .unwrap_or_default();
    let has_library = !library_overlays.is_empty();

    if config.sources.is_empty() && !has_library {
        eprintln!(
            "{} No overlay sources configured.\n\n\
             Add a source to get started:\n\
             \n  repoverlay source add <path-or-url>\n\n\
             Examples:\n\
             \n  repoverlay source add ./my-overlays          # local directory\
             \n  repoverlay source add owner/repo             # GitHub repo\
             \n  repoverlay source add https://github.com/owner/repo\n\n\
             Or browse an ephemeral source directly:\n\
             \n  repoverlay browse ./my-overlays\
             \n  repoverlay browse owner/repo\n",
            "hint:".yellow().bold(),
        );
        bail!("No overlay sources configured");
    }

    // Set up source manager (if configured sources exist)
    let manager = if config.sources.is_empty() {
        None
    } else {
        let mgr = SourceManager::new(config.sources, Some(&target_canonical))?;
        mgr.ensure_all_cloned()?;
        if update {
            println!("{} overlay sources...", "Updating".blue().bold());
            mgr.pull_all()?;
        }
        Some(mgr)
    };

    let all_with_sources = manager
        .as_ref()
        .map(SourceManager::list_all_overlays)
        .transpose()?
        .unwrap_or_default();

    // Build a lookup map: overlay key -> Source (for configured source overlays)
    let source_map: std::collections::HashMap<String, config::Source> = all_with_sources
        .iter()
        .map(|(src, overlay)| (overlay.to_string(), src.clone()))
        .collect();

    // Extract overlays for browse_and_apply
    let mut overlays: Vec<_> = if let Some(filter) = target_filter {
        let parts: Vec<&str> = filter.split('/').collect();
        if parts.len() != 2 {
            bail!("Invalid target filter format. Use: org/repo");
        }
        let (filter_org, filter_repo) = (parts[0], parts[1]);
        all_with_sources
            .into_iter()
            .filter(|(_, o)| {
                o.org.eq_ignore_ascii_case(filter_org) && o.repo.eq_ignore_ascii_case(filter_repo)
            })
            .map(|(_, o)| o)
            .collect()
    } else {
        all_with_sources.into_iter().map(|(_, o)| o).collect()
    };

    // Add library overlays to the browse list (#218)
    if has_library && target_filter.is_none() {
        for lib_overlay in &library_overlays {
            overlays.push(AvailableOverlay::synthetic_flat(
                library::LIBRARY_SOURCE_NAME.to_string(),
                lib_overlay.name.clone(),
                lib_overlay.name.clone().into(),
                true,
            ));
        }
    }

    let library_path = library::get_library_path(&target_canonical).ok();
    let build_source_info = |o: &AvailableOverlay| {
        // Library overlays resolve differently from source overlays
        if o.org == library::LIBRARY_SOURCE_NAME {
            let lp = library_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Library path not found"))?;
            let overlay_path = lp.join(&o.name).canonicalize()?;
            return Ok(ResolvedSource {
                path: overlay_path,
                source_info: OverlaySource::library(o.name.clone()),
            });
        }

        let overlay_key = o.to_string();
        let source = source_map.get(&overlay_key).ok_or_else(|| {
            anyhow::anyhow!("Could not determine source for overlay: {overlay_key}")
        })?;
        let mgr = manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No source manager available"))?;
        let base_path = mgr
            .get_source_base_path(&source.name)
            .ok_or_else(|| anyhow::anyhow!("Source base path not found: {}", source.name))?;
        let commit = mgr.get_source_commit(&source.name)?;

        resolve_configured_browse_source(
            base_path,
            o,
            commit,
            source.name.clone(),
            source.is_local(),
        )
    };

    browse_and_apply(
        overlays,
        target_filter,
        target,
        no_interactive,
        dry_run,
        show_all,
        build_source_info,
    )
}

/// Browse overlays from an ephemeral source (not saved to config).
///
/// Fetches the source repository to cache, lists available overlays, and
/// presents them for selection and apply — without modifying the source config.
#[allow(clippy::fn_params_excessive_bools)]
fn browse_ephemeral_source(
    source_str: &str,
    target_filter: Option<&str>,
    update: bool,
    target: Option<PathBuf>,
    no_interactive: bool,
    dry_run: bool,
    show_all: bool,
) -> Result<()> {
    let reference = SourceReference::parse(source_str);

    let (owner, repo) = match reference {
        SourceReference::LocalPath { path, .. } => {
            let canonical = path
                .canonicalize()
                .with_context(|| format!("Path does not exist: {}", path.display()))?;
            return browse_local_source(
                &canonical,
                target_filter,
                target,
                no_interactive,
                dry_run,
                show_all,
            );
        }
        SourceReference::OnePart { username } => {
            let default_repo = config::default_overlay_repo_name();
            (username, default_repo)
        }
        SourceReference::TwoPart { owner, repo } => (owner, repo),
        SourceReference::GitHubUrl(url) => {
            let github_source = GitHubSource::parse(&url)?;
            (github_source.owner, github_source.repo)
        }
        SourceReference::ThreePart { .. } => {
            bail!(
                "Invalid source for browse: '{source_str}'\n\n\
                 Use a GitHub username, owner/repo, GitHub URL, or local path."
            );
        }
    };

    let github_url = format!("https://github.com/{owner}/{repo}");
    let github_source = GitHubSource::parse(&github_url)?;
    let cache = CacheManager::new()?;
    println!(
        "{} repository: {}/{}",
        if update { "Updating" } else { "Fetching" }.blue().bold(),
        owner,
        repo
    );
    let cached = cache.ensure_cached(&github_source, update)?;

    let overlays = list_overlays_from_cached_repo(&owner, &repo)?;

    let git_ref_str = github_source.git_ref.as_str().to_string();
    let commit = get_cached_repo_commit(&cached.path).unwrap_or_else(|| "unknown".to_string());
    let cached_path = cached.path;

    let build_source_info = |o: &AvailableOverlay| {
        let overlay_path = cached_path.join(&o.org).join(&o.repo).join(&o.name);
        if !overlay_path.exists() {
            bail!("Overlay directory not found: {}", overlay_path.display());
        }
        Ok(ResolvedSource {
            path: overlay_path,
            source_info: OverlaySource::github(
                github_url.clone(),
                owner.clone(),
                repo.clone(),
                git_ref_str.clone(),
                commit.clone(),
                Some(o.to_string()),
            ),
        })
    };

    browse_and_apply(
        overlays,
        target_filter,
        target,
        no_interactive,
        dry_run,
        show_all,
        build_source_info,
    )
}

/// Browse overlays from an ephemeral local directory source.
///
/// Scans the local directory for overlays, auto-detecting whether the directory
/// uses structured (org/repo/name) or flat layout.
#[allow(clippy::fn_params_excessive_bools)]
fn browse_local_source(
    local_path: &Path,
    target_filter: Option<&str>,
    target: Option<PathBuf>,
    no_interactive: bool,
    dry_run: bool,
    show_all: bool,
) -> Result<()> {
    println!(
        "{} local source: {}",
        "Scanning".blue().bold(),
        local_path.display()
    );

    let all_overlays = list_overlays_in_dir(local_path)?;

    let overlays = if let Some(filter) = target_filter {
        let parts: Vec<&str> = filter.split('/').collect();
        if parts.len() != 2 {
            bail!("Invalid target filter format. Use: org/repo");
        }
        let (filter_org, filter_repo) = (parts[0], parts[1]);
        all_overlays
            .into_iter()
            .filter(|o| {
                o.org.eq_ignore_ascii_case(filter_org) && o.repo.eq_ignore_ascii_case(filter_repo)
            })
            .collect()
    } else {
        all_overlays
    };

    let local_base = local_path.to_path_buf();
    let build_source_info = move |o: &AvailableOverlay| resolve_local_browse_source(&local_base, o);

    browse_and_apply(
        overlays,
        target_filter,
        target,
        no_interactive,
        dry_run,
        show_all,
        build_source_info,
    )
}

fn resolve_local_browse_source(local_base: &Path, o: &AvailableOverlay) -> Result<ResolvedSource> {
    let overlay_path = resolve_browse_overlay_path(local_base, o)?;
    Ok(ResolvedSource {
        path: overlay_path.clone(),
        source_info: OverlaySource::local(overlay_path),
    })
}

fn resolve_browse_overlay_path(base: &Path, o: &AvailableOverlay) -> Result<PathBuf> {
    let overlay_path = base.join(o.source_relative_path());
    let canonical_base = base
        .canonicalize()
        .with_context(|| format!("Source base not found: {}", base.display()))?;
    let canonical_overlay = overlay_path
        .canonicalize()
        .with_context(|| format!("Overlay directory not found: {}", overlay_path.display()))?;

    if !canonical_overlay.starts_with(&canonical_base) {
        bail!(
            "Overlay directory escapes source base: {}",
            overlay_path.display()
        );
    }

    if !canonical_overlay.is_dir() {
        bail!(
            "Overlay path is not a directory: {}",
            overlay_path.display()
        );
    }

    Ok(canonical_overlay)
}

fn resolve_configured_browse_source(
    base_path: &Path,
    o: &AvailableOverlay,
    commit: impl Into<String>,
    source_name: impl Into<String>,
    source_is_local: bool,
) -> Result<ResolvedSource> {
    let overlay_path = resolve_browse_overlay_path(base_path, o)?;
    let commit = commit.into();
    let source_name = source_name.into();
    let source_info = if source_is_local {
        OverlaySource::configured_local(overlay_path.clone(), source_name)
    } else {
        OverlaySource::overlay_repo_full(
            o.org.clone(),
            o.repo.clone(),
            o.name.clone(),
            commit,
            ResolvedVia::Direct,
            source_name,
        )
    };

    Ok(ResolvedSource {
        path: overlay_path,
        source_info,
    })
}

/// Shared logic for browse: filter, display, select, and apply overlays.
#[allow(clippy::fn_params_excessive_bools)]
fn browse_and_apply<F>(
    overlays: Vec<AvailableOverlay>,
    target_filter: Option<&str>,
    target: Option<PathBuf>,
    no_interactive: bool,
    dry_run: bool,
    show_all: bool,
    build_source: F,
) -> Result<()>
where
    F: Fn(&AvailableOverlay) -> Result<ResolvedSource>,
{
    // Auto-filter by current repo when no explicit filter and not --show-all
    let (display_overlays, filtered) =
        auto_filter_overlays(overlays, show_all || target_filter.is_some());

    if display_overlays.is_empty() {
        if let Some(filter) = target_filter {
            println!("{} No overlays found for {}.", "Status:".bold(), filter);
        } else {
            println!("{} No overlays found in repository.", "Status:".bold());
        }
        return Ok(());
    }

    // Non-interactive: just print the list
    if no_interactive || !is_interactive() {
        print_overlay_list(&display_overlays, filtered);
        return Ok(());
    }

    // Interactive mode: select and apply
    let target = canonicalize_path(
        &target.unwrap_or_else(|| PathBuf::from(".")),
        "Target directory",
    )?;
    validate_git_repo(&target)?;

    // Get already-applied overlays to disable them in the selector
    let applied_overlays = list_applied_overlays(&target).unwrap_or_default();

    let items: Vec<SelectableItem> = display_overlays
        .iter()
        .map(|o| {
            BrowseOverlayItem {
                overlay: o,
                applied_overlays: &applied_overlays,
            }
            .to_selectable_item(&target)
        })
        .collect();

    let result = select_flat(
        &items,
        &FlatSelectionConfig {
            prompt: "Select overlay(s) to apply:".into(),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        println!("No overlays selected.");
        return Ok(());
    }

    // Map selected IDs back to AvailableOverlay values
    let selected: Vec<_> = result
        .selected_ids
        .iter()
        .filter_map(|id| display_overlays.iter().find(|o| o.to_string() == *id))
        .collect();

    // Build ResolvedSources for apply
    let sources: Vec<ResolvedSource> = selected
        .iter()
        .map(|o| build_source(o))
        .collect::<Result<Vec<_>>>()?;

    apply_multiple_overlays(
        &sources,
        &target,
        false,
        dry_run,
        ConflictStrategy::default(),
        false,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_overlay(name: &str, relative_path: &str) -> AvailableOverlay {
        AvailableOverlay::flat(name.to_string(), PathBuf::from(relative_path), false)
    }

    fn structured_overlay() -> AvailableOverlay {
        AvailableOverlay::structured(
            "owner".to_string(),
            "repo".to_string(),
            "config".to_string(),
            false,
        )
    }

    fn assert_same_canonical_path(actual: &Path, expected: &Path) {
        assert_eq!(
            actual.canonicalize().unwrap(),
            expected.canonicalize().unwrap()
        );
    }

    #[test]
    fn local_browse_flat_subdirectory_records_overlay_path() {
        let source = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(source.path().join("config-a")).unwrap();
        let overlay = flat_overlay("config-a", "config-a");

        let resolved = resolve_local_browse_source(source.path(), &overlay).unwrap();

        assert_same_canonical_path(&resolved.path, &source.path().join("config-a"));
        match resolved.source_info {
            OverlaySource::Local { path, .. } => {
                assert_same_canonical_path(&path, &source.path().join("config-a"));
            }
            other => panic!("expected local source, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn local_browse_rejects_symlink_overlay_that_escapes_source_base() {
        use std::os::unix::fs::symlink;

        let source = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        fs::write(outside.path().join(".envrc"), "export SECRET=1").unwrap();
        symlink(outside.path(), source.path().join("escape")).unwrap();
        let overlay = flat_overlay("escape", "escape");

        let Err(err) = resolve_local_browse_source(source.path(), &overlay) else {
            panic!("expected symlink escape to be rejected");
        };

        assert!(err.to_string().contains("escapes source base"));
    }

    #[test]
    fn configured_browse_flat_subdirectory_records_local_overlay_path() {
        let source = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(source.path().join("config-a")).unwrap();
        let overlay = flat_overlay("config-a", "config-a");

        let resolved =
            resolve_configured_browse_source(source.path(), &overlay, "local", "local-flat", true)
                .unwrap();

        assert_same_canonical_path(&resolved.path, &source.path().join("config-a"));
        match resolved.source_info {
            OverlaySource::Local { path, source_name } => {
                assert_same_canonical_path(&path, &source.path().join("config-a"));
                assert_eq!(source_name.as_deref(), Some("local-flat"));
            }
            other => panic!("expected local source, got {other:?}"),
        }
    }

    #[test]
    fn configured_browse_structured_preserves_overlay_repo_source() {
        let source = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(source.path().join("owner/repo/config")).unwrap();
        let overlay = structured_overlay();

        let resolved =
            resolve_configured_browse_source(source.path(), &overlay, "abc123", "shared", false)
                .unwrap();

        assert_same_canonical_path(&resolved.path, &source.path().join("owner/repo/config"));
        match resolved.source_info {
            OverlaySource::OverlayRepo {
                org,
                repo,
                name,
                commit,
                resolved_via,
                source_name,
            } => {
                assert_eq!(org, "owner");
                assert_eq!(repo, "repo");
                assert_eq!(name, "config");
                assert_eq!(commit, "abc123");
                assert_eq!(resolved_via, Some(ResolvedVia::Direct));
                assert_eq!(source_name.as_deref(), Some("shared"));
            }
            other => panic!("expected overlay repo source, got {other:?}"),
        }
    }
}
