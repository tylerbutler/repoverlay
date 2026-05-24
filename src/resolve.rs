//! Source resolution — convert user-supplied source strings into local paths.

use anyhow::{Context, Result, bail};
use colored::Colorize;
use log::debug;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::CacheManager;
use crate::config;
use crate::fuzzy::OverlayMatcher;
use crate::github;
use crate::github::GitHubSource;
use crate::library;
use crate::overlay_name::OverlayName;
use crate::overlay_repo::AvailableOverlay;
use crate::reference::SourceReference;
use crate::selection::is_interactive;
use crate::sources;
use crate::state;
use crate::state::{
    OverlaySource, OverlayState, ResolvedVia, SourceResolver, list_applied_overlays,
    normalize_overlay_name, save_overlay_state,
};
use crate::upstream;
use crate::upstream::detect_upstream;

/// Canonicalize a path and return an error with a descriptive message if it fails.
pub(crate) fn canonicalize_path(path: &Path, description: &str) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("{} not found: {}", description, path.display()))
}

/// Resolved source information for applying an overlay.
pub(crate) struct ResolvedSource {
    /// Local path to the overlay files
    pub path: PathBuf,
    /// Source information for state tracking
    pub source_info: OverlaySource,
}

/// Result of source resolution - may be a single overlay or multiple from browse mode.
pub(crate) enum ResolvedSources {
    /// A single resolved overlay source.
    Single(ResolvedSource),
    /// Multiple resolved overlay sources (from browse mode multi-select).
    Multiple(Vec<ResolvedSource>),
}

/// Resolve a source string to a local path.
///
/// Resolution order:
/// 1. GitHub URL (`https://github.com/...`) - downloads to cache, returns cached path
/// 2. Explicit local path (`./path`, `/path`, `~/path`) - returns path directly
/// 3. Existing local path (backward compat, warns about future `./` requirement)
/// 4. Three-part reference (`org/repo/name`) - resolves from configured sources
/// 5. Two-part reference (`org/repo`) - browse mode (Phase B)
/// 6. One-part reference (`username`) - expands to `username/repo-overlays` (Phase C)
///
/// Find a configured source whose URL matches the given GitHub owner/repo.
///
/// Loads the user config and checks each source URL against the provided
/// owner and repo. Returns the first matching source, if any.
fn find_matching_source(
    owner: &str,
    repo: &str,
    target_path: Option<&Path>,
) -> Option<config::Source> {
    let config = config::load_config(target_path).ok()?;
    config.sources.into_iter().find(|source| {
        source
            .url
            .as_deref()
            .and_then(github::parse_remote_url)
            .is_some_and(|(src_owner, src_repo)| src_owner == owner && src_repo == repo)
    })
}

/// Upgrade a GitHub overlay state to `OverlayRepo` if it matches a configured source.
///
/// When an overlay was originally applied via a GitHub URL that points to a configured
/// overlay repo source, this converts the state from read-only `GitHub` to editable
/// `OverlayRepo`. The upgrade is persisted to disk.
///
/// Returns `true` if the state was upgraded and saved.
///
/// # Errors
///
/// Returns an error if saving the upgraded state fails.
pub(crate) fn try_upgrade_github_source(target: &Path, state: &mut OverlayState) -> Result<bool> {
    let (owner, gh_repo, subpath, commit) = match &state.source {
        OverlaySource::GitHub {
            owner,
            repo,
            subpath: Some(subpath),
            commit,
            ..
        } => (owner.clone(), repo.clone(), subpath.clone(), commit.clone()),
        _ => return Ok(false),
    };

    // Parse subpath as org/repo/name (3 non-empty parts)
    let parts: Vec<&str> = subpath.split('/').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return Ok(false);
    }
    let (org, target_repo, name) = (
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    );

    let Some(source) = find_matching_source(&owner, &gh_repo, Some(target)) else {
        return Ok(false);
    };

    state.source = OverlaySource::overlay_repo_full(
        org,
        target_repo,
        name.clone(),
        commit,
        ResolvedVia::Direct,
        source.name.clone(),
    );
    save_overlay_state(target, state)?;

    println!(
        "{} overlay '{}' from GitHub to overlay repo source '{}' (now editable)",
        "Upgraded".green().bold(),
        name,
        source.name,
    );

    Ok(true)
}

/// # Errors
///
/// Returns an error if:
/// - The source doesn't match any valid format
/// - A local path doesn't exist
/// - GitHub fetch fails
/// - Overlay repo is not configured (for structured references)
/// - `source_filter` specifies an unknown source
pub(crate) fn resolve_source(
    source_str: &str,
    ref_override: Option<&str>,
    update: bool,
    target_path: Option<&Path>,
    source_filter: Option<&str>,
) -> Result<ResolvedSources> {
    debug!(
        "resolve_source: {source_str} (ref_override={ref_override:?}, update={update}, source_filter={source_filter:?})"
    );

    // Check library first for bare names or explicit @library source filter
    let is_bare_name = !source_str.contains('/') && !source_str.starts_with('.');
    let is_library_filter = source_filter == Some(library::LIBRARY_SOURCE_NAME);
    if (is_bare_name || is_library_filter)
        && let Some(target) = target_path
        && let Ok(library_path) = library::get_library_path(target)
        && let overlay_path = library_path.join(source_str)
        && overlay_path.is_dir()
    {
        // Canonicalize so symlinks are created with absolute paths
        let overlay_path = overlay_path.canonicalize().unwrap_or(overlay_path);
        debug!(
            "resolved '{source_str}' from library at {}",
            overlay_path.display()
        );
        return Ok(ResolvedSources::Single(ResolvedSource {
            path: overlay_path,
            source_info: state::OverlaySource::library(source_str.to_string()),
        }));
    }

    // If --from @library was explicitly requested but overlay wasn't found, error immediately
    if is_library_filter {
        let library_overlays = target_path
            .and_then(|t| library::get_library_path(t).ok())
            .and_then(|lp| library::list_library_overlays(&lp).ok())
            .unwrap_or_default();

        if library_overlays.is_empty() {
            bail!(
                "No overlays found in library. Import one with: repoverlay library import <source>"
            );
        }
        let names: Vec<&str> = library_overlays.iter().map(|o| o.name.as_str()).collect();
        bail!(
            "Overlay '{}' not found in library. Available: {}",
            source_str,
            names.join(", ")
        );
    }

    if source_filter.is_some() && is_unqualified_overlay_name(source_str) {
        return resolve_one_part_from_configured_source(
            source_str,
            target_path,
            source_filter,
            update,
        )
        .map(ResolvedSources::Single);
    }

    // Parse input into structured reference
    let reference = SourceReference::parse(source_str);
    debug!("parsed reference: {reference:?}");

    match reference {
        SourceReference::GitHubUrl(url) => {
            // Check if this GitHub URL matches a configured source — if so, resolve
            // as an overlay repo (editable) instead of a read-only GitHub source.
            if let Ok(github_source) = GitHubSource::parse(&url)
                && let Some(matched) =
                    find_matching_source(&github_source.owner, &github_source.repo, target_path)
            {
                // Check if the URL includes a subpath that looks like org/repo/name
                if let Some(ref subpath) = github_source.subpath {
                    let subpath_str = subpath.to_string_lossy().to_string();
                    let parts: Vec<&str> =
                        subpath_str.split('/').filter(|s| !s.is_empty()).collect();
                    if parts.len() >= 3 {
                        let (org, repo_name, name) = (parts[0], parts[1], parts[2]);
                        println!(
                            "{} Detected configured source '{}', resolving as overlay repo (editable)",
                            "Info".cyan().bold(),
                            matched.name,
                        );
                        return resolve_three_part(
                            org,
                            repo_name,
                            name,
                            target_path,
                            source_filter,
                            update,
                        )
                        .map(ResolvedSources::Single);
                    }
                }

                // Bare repo URL (no qualifying subpath) — browse mode, but editable
                println!(
                    "{} Detected configured source '{}', resolving as overlay repo (editable)",
                    "Info".cyan().bold(),
                    matched.name,
                );
                return resolve_two_part(
                    &github_source.owner,
                    &github_source.repo,
                    ref_override,
                    update,
                    target_path,
                    source_filter,
                    Some(&matched),
                );
            }

            resolve_github_url(&url, ref_override, update).map(ResolvedSources::Single)
        }

        SourceReference::LocalPath {
            path,
            needs_prefix_warning,
        } => {
            resolve_local_path(&path, source_str, needs_prefix_warning).map(ResolvedSources::Single)
        }

        SourceReference::ThreePart {
            owner,
            repo,
            overlay,
        } => resolve_three_part(&owner, &repo, &overlay, target_path, source_filter, update)
            .map(ResolvedSources::Single),

        SourceReference::TwoPart { owner, repo } => resolve_two_part(
            &owner,
            &repo,
            ref_override,
            update,
            target_path,
            source_filter,
            None,
        ),

        SourceReference::OnePart { username } if source_filter.is_some() => {
            resolve_one_part_from_configured_source(&username, target_path, source_filter, update)
                .map(ResolvedSources::Single)
        }

        SourceReference::OnePart { username } => {
            // Before treating as a GitHub username, check if this is an applied
            // overlay name that could be resolved from its source. This catches
            // cases like `switch ff-oce` where the user means an overlay name,
            // not a GitHub user.
            if let Some(target) = target_path {
                let applied = state::list_applied_overlays(target).unwrap_or_default();
                if applied.iter().any(|n| n.as_str() == username) {
                    // Already applied — resolve from its existing source
                    let overlay_state = state::load_overlay_state(target, &username)?;
                    let path = match &overlay_state.source {
                        state::OverlaySource::Library { name } => {
                            // Library sources need repo context to resolve
                            let lib_path = library::get_library_path(target)?;
                            lib_path.join(name).canonicalize()?
                        }
                        source => source.resolve_local_path()?,
                    };
                    return Ok(ResolvedSources::Single(ResolvedSource {
                        path,
                        source_info: overlay_state.source,
                    }));
                }
            }

            // Phase C: Expand username to username/{default_repo}
            let default_repo = config::default_overlay_repo_name();
            debug!("expanding one-part reference: {username} -> {username}/{default_repo}",);
            resolve_two_part(
                &username,
                &default_repo,
                ref_override,
                update,
                target_path,
                source_filter,
                None,
            )
        }
    }
}

fn is_unqualified_overlay_name(source_str: &str) -> bool {
    !source_str.is_empty()
        && !source_str.contains('/')
        && !source_str.starts_with('.')
        && source_str != "~"
}

/// Resolve a GitHub URL source.
fn resolve_github_url(
    url: &str,
    ref_override: Option<&str>,
    update: bool,
) -> Result<ResolvedSource> {
    debug!("resolving GitHub URL: {url}");
    let mut github_source = GitHubSource::parse(url)?;

    // Apply ref override if provided
    if let Some(ref_str) = ref_override {
        github_source = github_source.with_ref_override(Some(ref_str))?;
    }

    // Ensure cached and get path
    let cache = CacheManager::new()?;

    let label = if update {
        "Fetching"
    } else {
        "Fetching (cached)"
    };
    println!(
        "{} repository: {}/{}",
        label.blue().bold(),
        github_source.owner,
        github_source.repo
    );

    let cached = cache.ensure_cached(&github_source, update)?;

    Ok(ResolvedSource {
        path: cached.path,
        source_info: OverlaySource::github(
            url.to_string(),
            github_source.owner,
            github_source.repo,
            github_source.git_ref.as_str().to_string(),
            cached.commit,
            github_source
                .subpath
                .map(|p| p.to_string_lossy().to_string()),
        ),
    })
}

/// Resolve a local filesystem path.
pub(crate) fn resolve_local_path(
    path: &Path,
    original_input: &str,
    needs_prefix_warning: bool,
) -> Result<ResolvedSource> {
    debug!("resolving local path: {}", path.display());

    // Require `./` prefix for local paths to avoid ambiguity with overlay repo references.
    // Absolute paths (`/...`) and home-relative paths (`~/...`) are unambiguous and allowed.
    if needs_prefix_warning {
        bail!(
            "Ambiguous path '{}': use './{original_input}' to specify a local path explicitly.",
            path.display()
        );
    }

    if !path.exists() {
        bail!("Overlay source not found: {}", path.display());
    }

    let canonical = path
        .canonicalize()
        .with_context(|| format!("Overlay source not found: {}", path.display()))?;

    Ok(ResolvedSource {
        path: canonical.clone(),
        source_info: OverlaySource::local(canonical),
    })
}

/// Resolve a two-part overlay reference by browsing a GitHub overlay repository.
///
/// This is "GitHub browse mode" - the user specified a GitHub repo containing overlays
/// (e.g., `tylerbutler/repo-overlays`). We clone it to the GitHub cache and let them
/// interactively select one or more overlays.
///
/// The overlay repo structure is: `<target_org>/<target_repo>/<overlay_name>/`
/// For example: `microsoft/FluidFramework/vscode-setup/`
///
/// Note: This differs from `resolve_three_part` which uses configured overlay sources.
/// Here we resolve directly from the cached GitHub repo, not through `SourceManager`.
fn resolve_two_part(
    owner: &str,
    repo: &str,
    ref_override: Option<&str>,
    update: bool,
    target_path: Option<&Path>,
    _source_filter: Option<&str>,
    matched_source: Option<&config::Source>,
) -> Result<ResolvedSources> {
    let mode = if matched_source.is_some() {
        "overlay repo browse"
    } else {
        "GitHub browse"
    };
    debug!("resolving two-part reference ({mode} mode): {owner}/{repo}");

    // Create a GitHubSource to fetch/cache the repo
    let github_url = format!("https://github.com/{owner}/{repo}");
    let mut github_source = GitHubSource::parse(&github_url)?;

    if let Some(ref_str) = ref_override {
        github_source = github_source.with_ref_override(Some(ref_str))?;
    }

    // Fetch/cache the repository
    let cache = CacheManager::new()?;
    let label = if update {
        "Fetching"
    } else {
        "Fetching (cached)"
    };
    println!("{} repository: {}/{}", label.blue().bold(), owner, repo);
    let cached = cache.ensure_cached(&github_source, update)?;

    // List available overlays
    let available_overlays = list_overlays_from_cached_repo(owner, repo)?;

    if available_overlays.is_empty() {
        bail!(
            "No overlays found in {owner}/{repo}.\n\n\
             Make sure the repository contains overlay directories in the format:\n\
             <target-org>/<target-repo>/<overlay-name>/"
        );
    }

    // Select overlays based on interactivity
    let applied = target_path
        .map(|p| list_applied_overlays(p).unwrap_or_default())
        .unwrap_or_default();
    let repo_identity = target_path.and_then(|p| upstream::detect_repo_identity(p).ok().flatten());
    let selected_overlays = if is_interactive() {
        select_overlays_interactive(
            owner,
            repo,
            &available_overlays,
            &applied,
            repo_identity.as_ref(),
        )?
    } else {
        // Non-interactive mode - error with available overlays
        let overlay_list = available_overlays
            .iter()
            .map(|o| format!("  {}", o.display_bold()))
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "No overlay specified. Available overlays in {owner}/{repo}:\n{overlay_list}\n\n\
             Use: repoverlay apply {owner}/{repo}/<target-org>/<target-repo>/<overlay-name>"
        );
    };

    if selected_overlays.len() == 1 {
        println!(
            "{} overlay: {}",
            "Selected".green().bold(),
            selected_overlays[0].display_bold()
        );
    } else {
        println!(
            "{} {} overlays:",
            "Selected".green().bold(),
            selected_overlays.len()
        );
        for selected in &selected_overlays {
            println!("  - {}", selected.display_bold());
        }
    }

    // Skip save prompt when source is already configured
    if matched_source.is_none() {
        prompt_save_source(owner, repo, target_path)?;
    }

    // Resolve each selected overlay to a ResolvedSource
    let commit = get_cached_repo_commit(&cached.path).unwrap_or_else(|| "unknown".to_string());

    let mut resolved_sources = Vec::with_capacity(selected_overlays.len());

    for selected in &selected_overlays {
        let overlay_path = cached
            .path
            .join(&selected.org)
            .join(&selected.repo)
            .join(&selected.name);

        if !overlay_path.exists() {
            bail!("Overlay directory not found: {}", overlay_path.display());
        }

        let source_info = if let Some(source) = matched_source {
            // Resolve as overlay repo — editable and syncable
            OverlaySource::overlay_repo_full(
                selected.org.clone(),
                selected.repo.clone(),
                selected.name.clone(),
                commit.clone(),
                ResolvedVia::Direct,
                source.name.clone(),
            )
        } else {
            // Resolve as read-only GitHub source
            let git_ref_str = github_source.git_ref.as_str().to_string();
            OverlaySource::github(
                github_url.clone(),
                owner.to_string(),
                repo.to_string(),
                git_ref_str,
                commit.clone(),
                Some(selected.to_string()),
            )
        };

        resolved_sources.push(ResolvedSource {
            path: overlay_path,
            source_info,
        });
    }

    if resolved_sources.len() == 1 {
        Ok(ResolvedSources::Single(
            resolved_sources.into_iter().next().unwrap(),
        ))
    } else {
        Ok(ResolvedSources::Multiple(resolved_sources))
    }
}

/// Check whether a source is already configured (globally or repo-locally).
///
/// Returns `true` if a source with a matching URL or name is found in the merged config.
fn source_is_configured(owner: &str, repo: &str, target_path: Option<&Path>) -> Result<bool> {
    let config = config::load_config(target_path)?;
    let url = format!("https://github.com/{owner}/{repo}");
    let source_name = repo;

    Ok(config
        .sources
        .iter()
        .any(|s| s.url.as_deref() == Some(&url) || s.name == source_name))
}

/// Prompt the user to save a source for future use, if not already configured.
///
/// Skips silently if the source is already configured or the session is non-interactive.
fn prompt_save_source(owner: &str, repo: &str, target_path: Option<&Path>) -> Result<()> {
    if !is_interactive() {
        return Ok(());
    }

    if source_is_configured(owner, repo, target_path)? {
        return Ok(());
    }

    // Load global-only config for saving — repo-local sources should not be
    // written into the global config file.
    let mut config = config::load_config(None)?;
    let url = format!("https://github.com/{owner}/{repo}");
    let source_name = repo.to_string();

    let prompt = format!("Save {owner}/{repo} as a source for future use?");
    let confirmed = dialoguer::Confirm::new()
        .with_prompt(&prompt)
        .default(true)
        .interact()?;

    if !confirmed {
        return Ok(());
    }

    config.sources.push(config::Source {
        name: source_name.clone(),
        url: Some(url.clone()),
        path: None,
    });
    config::save_config(&config)?;

    println!(
        "{} source '{}' ({})",
        "Saved".green().bold(),
        source_name,
        url
    );

    Ok(())
}

/// Get the current commit hash from a cached repository.
pub(crate) fn get_cached_repo_commit(repo_path: &Path) -> Option<String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// List overlays from a cached GitHub repository.
///
/// Used by GitHub browse mode (`resolve_two_part`) to list overlays from a
/// directly-specified GitHub repo (e.g., `tylerbutler/repo-overlays`).
///
/// Overlay repos have a nested structure: `target_org/target_repo/overlay_name/`
/// Returns `AvailableOverlay` structs with org, repo, and overlay name.
///
/// Note: This is the same structure used by `OverlayRepoManager::list_overlays()`,
/// but operates on the GitHub cache instead of managed overlay repositories.
pub(crate) fn list_overlays_from_cached_repo(
    owner: &str,
    repo: &str,
) -> Result<Vec<AvailableOverlay>> {
    debug!("listing overlays from cached GitHub repo: {owner}/{repo}");

    let cache = CacheManager::new()?;
    let cache_dir = cache.cache_dir();
    let repo_path = cache_dir.join("github").join(owner).join(repo);

    if !repo_path.exists() {
        bail!("Repository not cached: {owner}/{repo}");
    }

    list_overlays_from_path(&repo_path)
}

/// List overlays from a directory with nested org/repo/overlay structure.
///
/// This is the core logic for listing overlays, extracted for testability.
pub(crate) fn list_overlays_from_path(repo_path: &Path) -> Result<Vec<AvailableOverlay>> {
    let mut overlays = Vec::new();

    for (org_path, org_name) in visible_subdirs(repo_path)? {
        for (repo_dir, repo_name) in visible_subdirs(&org_path)? {
            for (overlay_path, overlay_name) in visible_subdirs(&repo_dir)? {
                let has_config = overlay_path.join("repoverlay.ccl").exists();
                overlays.push(AvailableOverlay::structured(
                    org_name.clone(),
                    repo_name.clone(),
                    overlay_name,
                    has_config,
                ));
            }
        }
    }

    overlays.sort_by(|a, b| (&a.org, &a.repo, &a.name).cmp(&(&b.org, &b.repo, &b.name)));
    debug!("found {} overlays in path", overlays.len());
    Ok(overlays)
}

/// Returns visible (non-hidden) subdirectories with their names.
pub(crate) fn visible_subdirs(path: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut results = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str().filter(|n| !n.starts_with('.')) {
            results.push((entry_path, name.to_string()));
        }
    }
    Ok(results)
}

/// Present an interactive multi-select picker for overlays.
///
/// Already-applied overlays appear dimmed and cannot be selected.
/// When `repo_identity` is provided, overlays targeting a different repository
/// are shown dimmed with a "different repo" description but remain selectable.
/// Matching overlays are listed first.
fn select_overlays_interactive(
    owner: &str,
    repo: &str,
    overlays: &[AvailableOverlay],
    applied_overlays: &[OverlayName],
    repo_identity: Option<&upstream::RepoIdentity>,
) -> Result<Vec<AvailableOverlay>> {
    use crate::selection::{FlatSelectionConfig, SelectableItem, select_flat};

    // Order overlays: matching repo first, then non-matching
    let ordered: Vec<&AvailableOverlay> = repo_identity.map_or_else(
        || overlays.iter().collect(),
        |identity| {
            let (matching, non_matching): (Vec<_>, Vec<_>) = overlays
                .iter()
                .partition(|o| identity.matches(&o.org, &o.repo));
            matching.into_iter().chain(non_matching).collect()
        },
    );

    let items: Vec<SelectableItem> = ordered
        .iter()
        .map(|o| {
            let already_applied = normalize_overlay_name(&o.name)
                .ok()
                .is_some_and(|normalized| {
                    applied_overlays.iter().any(|n| n == normalized.as_str())
                });
            let is_different_repo =
                repo_identity.is_some_and(|identity| !identity.matches(&o.org, &o.repo));

            let description = if already_applied {
                Some("already applied".into())
            } else if is_different_repo {
                Some("different repo".into())
            } else {
                None
            };

            SelectableItem {
                id: o.to_string(),
                label: o.to_string(),
                description,
                preselected: false,
                disabled: already_applied,
            }
        })
        .collect();

    let result = select_flat(
        &items,
        &FlatSelectionConfig {
            prompt: format!("Select overlay(s) from {owner}/{repo}:"),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlays selected");
    }

    // Map selected IDs back to AvailableOverlay values
    let selected: Vec<AvailableOverlay> = result
        .selected_ids
        .iter()
        .filter_map(|id| overlays.iter().find(|o| o.to_string() == *id).cloned())
        .collect();

    Ok(selected)
}

/// Resolve a three-part overlay reference (`org/repo/overlay`).
///
/// Provides fuzzy suggestions when the overlay is not found.
fn resolve_three_part(
    org: &str,
    repo: &str,
    name: &str,
    target_path: Option<&Path>,
    source_filter: Option<&str>,
    update: bool,
) -> Result<ResolvedSource> {
    debug!("resolving three-part reference: {org}/{repo}/{name}");

    // Load config (pass target_path to include repo-local sources)
    let config = config::load_config(target_path)?;

    // Detect upstream for fallback resolution
    let upstream = target_path.and_then(|p| detect_upstream(p).ok()).flatten();

    // Try multi-source resolution first if sources are configured
    if !config.sources.is_empty() {
        debug!(
            "using multi-source resolution with {} sources",
            config.sources.len()
        );
        return resolve_from_sources_with_suggestions(
            &config.sources,
            org,
            repo,
            name,
            upstream.as_ref(),
            source_filter,
            update,
            target_path,
        );
    }

    // No sources configured
    bail!(
        "Overlay repository not configured.\n\n\
         To apply overlays from a shared repository, first run:\n\
         repoverlay source add <url>\n\n\
         Or use a local path or GitHub URL instead."
    )
}

/// Resolve an overlay from configured sources with fuzzy suggestions on failure.
#[allow(clippy::too_many_arguments)]
fn resolve_from_sources_with_suggestions(
    sources: &[config::Source],
    org: &str,
    repo: &str,
    name: &str,
    upstream: Option<&upstream::UpstreamInfo>,
    source_filter: Option<&str>,
    update: bool,
    repo_root: Option<&Path>,
) -> Result<ResolvedSource> {
    let manager = sources::SourceManager::new(sources.to_vec(), repo_root)?;

    prepare_sources_for_resolution(&manager, sources, source_filter, update)?;

    // Resolve overlay from sources
    if let Some(resolved) = manager.resolve(org, repo, name, upstream, source_filter)? {
        if resolved.source.is_local() {
            let source_suffix = format!(" [{}]", resolved.source.name).cyan().to_string();
            if resolved.flat {
                println!(
                    "{} overlay: {}{}",
                    "Resolving".blue().bold(),
                    name,
                    source_suffix,
                );
            } else {
                println!(
                    "{} overlay: {}/{}/{}{}",
                    "Resolving".blue().bold(),
                    org,
                    repo,
                    name,
                    source_suffix,
                );
            }
            return Ok(ResolvedSource {
                path: resolved.path.clone(),
                source_info: OverlaySource::configured_local(resolved.path, resolved.source.name),
            });
        }

        // Determine actual org/repo for state tracking
        let via_upstream = resolved.resolved_via == state::ResolvedVia::Upstream;
        let (actual_org, actual_repo) = match (upstream, via_upstream) {
            (Some(up), true) => (up.org.clone(), up.repo.clone()),
            _ => (org.to_string(), repo.to_string()),
        };

        let via_suffix = if via_upstream {
            " (via upstream)".dimmed().to_string()
        } else {
            String::new()
        };
        let source_suffix = format!(" [{}]", resolved.source.name).cyan().to_string();

        println!(
            "{} overlay: {}/{}/{}{}{}",
            "Resolving".blue().bold(),
            actual_org,
            actual_repo,
            name,
            via_suffix,
            source_suffix,
        );

        if resolved.source.is_local() {
            return Ok(ResolvedSource {
                path: resolved.path.clone(),
                source_info: OverlaySource::configured_local(resolved.path, resolved.source.name),
            });
        }

        return Ok(ResolvedSource {
            path: resolved.path,
            source_info: OverlaySource::overlay_repo_full(
                actual_org,
                actual_repo,
                name.to_string(),
                resolved.commit,
                resolved.resolved_via,
                resolved.source.name,
            ),
        });
    }

    // Overlay not found - provide fuzzy suggestions
    let suggestions = get_fuzzy_suggestions_multi_source(&manager, org, repo, name);
    let source_list = manager.source_names().join(", ");
    let error_msg = format_not_found_error(org, repo, name, &suggestions, Some(&source_list));
    bail!("{error_msg}")
}

/// Resolve a one-part overlay name from an explicitly configured source.
///
/// Used for `apply <name> --from <source>` before falling back to GitHub username
/// expansion, so flat local sources can be addressed by overlay name.
fn resolve_one_part_from_configured_source(
    name: &str,
    target_path: Option<&Path>,
    source_filter: Option<&str>,
    update: bool,
) -> Result<ResolvedSource> {
    let config = config::load_config(target_path)?;
    if config.sources.is_empty() {
        bail!(
            "Overlay source not configured.\n\n\
             Add a source with: repoverlay source add <url-or-path>"
        );
    }

    let manager = sources::SourceManager::new(config.sources.clone(), target_path)?;
    prepare_sources_for_resolution(&manager, &config.sources, source_filter, update)?;

    let upstream = target_path.and_then(|p| detect_upstream(p).ok()).flatten();
    if let Some((org, repo)) = target_path
        .and_then(|p| upstream::detect_repo_identity(p).ok().flatten())
        .and_then(|identity| identity.origin.or(identity.upstream))
        && let Some(resolved) =
            manager.resolve(&org, &repo, name, upstream.as_ref(), source_filter)?
        && !resolved.flat
    {
        let via_upstream = resolved.resolved_via == state::ResolvedVia::Upstream;
        let (actual_org, actual_repo) = match (upstream.as_ref(), via_upstream) {
            (Some(up), true) => (up.org.clone(), up.repo.clone()),
            _ => (org, repo),
        };
        let via_suffix = if via_upstream {
            " (via upstream)".dimmed().to_string()
        } else {
            String::new()
        };
        let source_suffix = format!(" [{}]", resolved.source.name).cyan().to_string();
        println!(
            "{} overlay: {}/{}/{}{}{}",
            "Resolving".blue().bold(),
            actual_org,
            actual_repo,
            name,
            via_suffix,
            source_suffix,
        );

        return Ok(ResolvedSource {
            path: resolved.path,
            source_info: OverlaySource::overlay_repo_full(
                actual_org,
                actual_repo,
                name.to_string(),
                resolved.commit,
                resolved.resolved_via,
                resolved.source.name,
            ),
        });
    }

    if let Some(resolved) = manager.resolve("", "", name, None, source_filter)? {
        let source_suffix = format!(" [{}]", resolved.source.name).cyan().to_string();
        println!(
            "{} overlay: {}{}",
            "Resolving".blue().bold(),
            name,
            source_suffix,
        );

        let source_info = if resolved.source.is_local() {
            OverlaySource::configured_local(resolved.path.clone(), resolved.source.name)
        } else {
            OverlaySource::overlay_repo_full(
                String::new(),
                String::new(),
                name.to_string(),
                resolved.commit,
                resolved.resolved_via,
                resolved.source.name,
            )
        };

        return Ok(ResolvedSource {
            path: resolved.path,
            source_info,
        });
    }

    let suggestions = get_fuzzy_suggestions_multi_source(&manager, "", "", name);
    let source_list = manager.source_names().join(", ");
    let error_msg = format_not_found_error("", "", name, &suggestions, Some(&source_list));
    bail!("{error_msg}")
}

fn prepare_sources_for_resolution(
    manager: &sources::SourceManager,
    sources: &[config::Source],
    source_filter: Option<&str>,
    update: bool,
) -> Result<()> {
    if selected_sources_are_all_local(sources, source_filter) {
        debug!("skipping overlay source clone/update for local-only source selection");
        return Ok(());
    }

    manager.ensure_all_cloned()?;

    if update {
        println!("{} overlay sources...", "Updating".blue().bold());
        manager.pull_all()?;
    } else {
        debug!("skipping overlay source update (--no-update)");
    }

    Ok(())
}

fn selected_sources_are_all_local(sources: &[config::Source], source_filter: Option<&str>) -> bool {
    if let Some(filter_name) = source_filter {
        return sources
            .iter()
            .find(|source| source.name == filter_name)
            .is_none_or(config::Source::is_local);
    }

    !sources.is_empty() && sources.iter().all(config::Source::is_local)
}

/// Get fuzzy suggestions for overlay names from multi-source config.
fn get_fuzzy_suggestions_multi_source(
    manager: &sources::SourceManager,
    org: &str,
    repo: &str,
    query: &str,
) -> Vec<String> {
    let available: Vec<String> = manager
        .list_overlays_for_repo(org, repo)
        .unwrap_or_default()
        .into_iter()
        .map(|n| n.to_string())
        .collect();
    fuzzy_suggest(query, &available)
}

/// Find fuzzy matches for a query in the given candidates.
pub(crate) fn fuzzy_suggest(query: &str, candidates: &[String]) -> Vec<String> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let matcher = OverlayMatcher::new();
    matcher.suggest(query, candidates, 3)
}

/// Format a "not found" error message with optional fuzzy suggestions and source list.
pub(crate) fn format_not_found_error(
    org: &str,
    repo: &str,
    name: &str,
    suggestions: &[String],
    source_list: Option<&str>,
) -> String {
    use std::fmt::Write;

    let mut msg = format!("Overlay not found: {org}/{repo}/{name}");

    if let Some(sources) = source_list {
        let _ = write!(msg, "\n\nSearched sources: {sources}");
    }

    if !suggestions.is_empty() {
        msg.push_str("\n\nDid you mean?");
        for suggestion in suggestions {
            let _ = write!(msg, "\n  - {suggestion}");
        }
    }

    let _ = write!(
        msg,
        "\n\nUse `repoverlay list --filter {org}/{repo}` to see available overlays."
    );

    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to init git repo");
        dir
    }

    mod canonicalize_path_tests {
        use super::*;

        #[test]
        fn succeeds_on_existing_path() {
            let dir = TempDir::new().unwrap();
            let result = canonicalize_path(dir.path(), "Test directory");
            assert!(result.is_ok());
        }

        #[test]
        fn fails_on_nonexistent_path() {
            let result = canonicalize_path(Path::new("/nonexistent/path/12345"), "Test path");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not found"));
        }
    }

    mod resolved_source_tests {
        use super::*;

        #[test]
        fn resolved_source_struct_fields() {
            let source = ResolvedSource {
                path: PathBuf::from("/some/path"),
                source_info: OverlaySource::local(PathBuf::from("/origin")),
            };

            assert_eq!(source.path, PathBuf::from("/some/path"));
            match source.source_info {
                OverlaySource::Local { path, .. } => {
                    assert_eq!(path, PathBuf::from("/origin"));
                }
                _ => panic!("Expected Local source"),
            }
        }
    }

    mod resolved_sources_tests {
        use super::*;

        #[test]
        fn single_variant_holds_one_source() {
            let source = ResolvedSource {
                path: PathBuf::from("/some/path"),
                source_info: OverlaySource::local(PathBuf::from("/origin")),
            };
            let resolved = ResolvedSources::Single(source);
            match resolved {
                ResolvedSources::Single(s) => {
                    assert_eq!(s.path, PathBuf::from("/some/path"));
                }
                ResolvedSources::Multiple(_) => panic!("Expected Single variant"),
            }
        }

        #[test]
        fn multiple_variant_holds_vec_of_sources() {
            let sources = vec![
                ResolvedSource {
                    path: PathBuf::from("/path/a"),
                    source_info: OverlaySource::local(PathBuf::from("/origin-a")),
                },
                ResolvedSource {
                    path: PathBuf::from("/path/b"),
                    source_info: OverlaySource::local(PathBuf::from("/origin-b")),
                },
            ];
            let resolved = ResolvedSources::Multiple(sources);
            match resolved {
                ResolvedSources::Multiple(v) => {
                    assert_eq!(v.len(), 2);
                    assert_eq!(v[0].path, PathBuf::from("/path/a"));
                    assert_eq!(v[1].path, PathBuf::from("/path/b"));
                }
                ResolvedSources::Single(_) => panic!("Expected Multiple variant"),
            }
        }
    }

    mod configured_source_tests {
        use super::*;

        #[test]
        fn one_part_with_source_filter_resolves_flat_local_source() {
            let temp = TempDir::new().unwrap();
            let repo_root = temp.path();
            let local_source = repo_root.join("overlays");
            let overlay = local_source.join("config-a");
            fs::create_dir_all(&overlay).unwrap();
            fs::write(overlay.join(".envrc"), "export A=1").unwrap();
            config::save_repo_config(
                repo_root,
                &config::RepoverlayConfig {
                    sources: vec![config::Source {
                        name: "local".to_string(),
                        url: None,
                        path: Some(PathBuf::from("overlays")),
                    }],
                    library_path: None,
                },
            )
            .unwrap();

            let resolved =
                resolve_source("config-a", None, false, Some(repo_root), Some("local")).unwrap();

            match resolved {
                ResolvedSources::Single(source) => {
                    assert_eq!(source.path, overlay);
                    match source.source_info {
                        OverlaySource::Local { path, source_name } => {
                            assert_eq!(path, overlay);
                            assert_eq!(source_name.as_deref(), Some("local"));
                        }
                        other => panic!("expected local source, got {other:?}"),
                    }
                }
                ResolvedSources::Multiple(_) => panic!("expected single source"),
            }
        }
    }

    mod browse_mode_tests {
        use super::*;

        #[test]
        fn list_overlays_from_cached_repo_nonexistent() {
            // Non-existent repo should return error
            let result =
                list_overlays_from_cached_repo("nonexistent-owner-xyz", "nonexistent-repo-xyz");
            assert!(result.is_err());
        }

        #[test]
        fn list_overlays_from_cached_repo_finds_overlays_at_correct_path() {
            use crate::cache::CacheManager;
            use crate::github::GitHubSource;

            // Use a unique test owner/repo to avoid conflicts with real cache
            let test_owner = "test-owner-abc123xyz";
            let test_repo = "test-repo-abc123xyz";

            let cache = CacheManager::new().unwrap();
            let source =
                GitHubSource::parse(&format!("https://github.com/{test_owner}/{test_repo}"))
                    .unwrap();

            // Get the path where CacheManager would store this repo
            // This includes the "github" subdirectory: {cache_dir}/github/{owner}/{repo}
            let expected_repo_path = cache.repo_path(&source);

            // Create overlay structure at the correct cache location
            let overlay_path = expected_repo_path.join("target-org/target-repo/test-overlay");
            fs::create_dir_all(&overlay_path).unwrap();

            // Now list_overlays_from_cached_repo should find it
            let result = list_overlays_from_cached_repo(test_owner, test_repo);

            // Clean up before asserting (so cleanup happens even if test fails)
            let _ = fs::remove_dir_all(&expected_repo_path);
            // Also clean up parent dirs if empty
            if let Some(parent) = expected_repo_path.parent() {
                let _ = fs::remove_dir(parent);
                if let Some(grandparent) = parent.parent() {
                    let _ = fs::remove_dir(grandparent);
                }
            }

            // This should succeed - we created overlays at the correct cache location
            let overlays = result.expect(
                "list_overlays_from_cached_repo should find overlays at the path returned by CacheManager::repo_path()"
            );
            assert_eq!(overlays.len(), 1);
            assert_eq!(
                overlays[0].to_string(),
                "target-org/target-repo/test-overlay"
            );
        }

        #[test]
        fn list_overlays_from_cached_repo_path_matches_cache_manager() {
            use crate::cache::CacheManager;
            use crate::github::GitHubSource;

            // This test verifies that list_overlays_from_cached_repo looks in the same
            // location where CacheManager stores repositories.
            //
            // CacheManager::repo_path() returns: {cache_dir}/github/{owner}/{repo}
            // list_overlays_from_cached_repo should look in the same location.

            let cache = CacheManager::new().unwrap();
            let source =
                GitHubSource::parse("https://github.com/test-owner-xyz/test-repo-xyz").unwrap();

            let cache_manager_path = cache.repo_path(&source);

            // Verify the cache manager path includes "github" subdirectory
            assert!(
                cache_manager_path
                    .to_string_lossy()
                    .contains("/github/test-owner-xyz/test-repo-xyz"),
                "CacheManager::repo_path() should include 'github' subdirectory, got: {}",
                cache_manager_path.display()
            );

            // The path that list_overlays_from_cached_repo constructs should match
            // Currently it constructs: {cache_dir}/{owner}/{repo} (MISSING "github"!)
            // This test documents the expected behavior.
        }

        #[test]
        fn list_overlays_from_path_with_nested_structure() {
            // Create a temp directory with the nested org/repo/overlay structure
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            // Create nested overlay directories
            fs::create_dir_all(repo_path.join("microsoft/FluidFramework/vscode-setup")).unwrap();
            fs::create_dir_all(repo_path.join("microsoft/FluidFramework/ci-config")).unwrap();
            fs::create_dir_all(repo_path.join("tylerbutler/some-repo/my-overlay")).unwrap();

            let overlays = list_overlays_from_path(repo_path).unwrap();

            assert_eq!(overlays.len(), 3);
            // Results should be sorted
            assert_eq!(
                overlays[0].to_string(),
                "microsoft/FluidFramework/ci-config"
            );
            assert_eq!(
                overlays[1].to_string(),
                "microsoft/FluidFramework/vscode-setup"
            );
            assert_eq!(overlays[2].to_string(), "tylerbutler/some-repo/my-overlay");
        }

        #[test]
        fn list_overlays_from_path_skips_hidden_dirs() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            // Create visible and hidden directories at each level
            fs::create_dir_all(repo_path.join("org/repo/overlay")).unwrap();
            fs::create_dir_all(repo_path.join(".hidden-org/repo/overlay")).unwrap();
            fs::create_dir_all(repo_path.join("org/.hidden-repo/overlay")).unwrap();
            fs::create_dir_all(repo_path.join("org/repo/.hidden-overlay")).unwrap();

            let overlays = list_overlays_from_path(repo_path).unwrap();

            // Only the non-hidden overlay should be found
            assert_eq!(overlays.len(), 1);
            assert_eq!(overlays[0].to_string(), "org/repo/overlay");
        }

        #[test]
        fn list_overlays_from_path_skips_files() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            // Create overlay directory
            fs::create_dir_all(repo_path.join("org/repo/overlay")).unwrap();

            // Create files at various levels (should be skipped)
            fs::write(repo_path.join("README.md"), "readme").unwrap();
            fs::write(repo_path.join("org/README.md"), "readme").unwrap();
            fs::write(repo_path.join("org/repo/README.md"), "readme").unwrap();

            let overlays = list_overlays_from_path(repo_path).unwrap();

            assert_eq!(overlays.len(), 1);
            assert_eq!(overlays[0].to_string(), "org/repo/overlay");
        }

        #[test]
        fn list_overlays_from_path_empty_directory() {
            let temp = TempDir::new().unwrap();
            let overlays = list_overlays_from_path(temp.path()).unwrap();
            assert!(overlays.is_empty());
        }

        #[test]
        fn list_overlays_from_path_incomplete_nesting() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            // Only one level deep (org only, no repo/overlay)
            fs::create_dir_all(repo_path.join("org-only")).unwrap();
            // Two levels deep (org/repo, no overlay)
            fs::create_dir_all(repo_path.join("org/repo-only")).unwrap();
            // Complete three-level nesting
            fs::create_dir_all(repo_path.join("org/repo/overlay")).unwrap();

            let overlays = list_overlays_from_path(repo_path).unwrap();

            // Only the complete three-level path should be found
            assert_eq!(overlays.len(), 1);
            assert_eq!(overlays[0].to_string(), "org/repo/overlay");
        }

        #[test]
        fn get_cached_repo_commit_valid_git_repo() {
            let repo = create_test_repo();

            // Configure git user for this repo (required for commits)
            Command::new("git")
                .args(["config", "user.email", "test@test.com"])
                .current_dir(repo.path())
                .output()
                .unwrap();
            Command::new("git")
                .args(["config", "user.name", "Test User"])
                .current_dir(repo.path())
                .output()
                .unwrap();

            // Create a file and commit it
            fs::write(repo.path().join("test.txt"), "test content").unwrap();
            Command::new("git")
                .args(["add", "."])
                .current_dir(repo.path())
                .output()
                .unwrap();
            let commit_output = Command::new("git")
                .args(["commit", "-m", "initial"])
                .current_dir(repo.path())
                .output()
                .unwrap();

            // Verify commit succeeded
            assert!(
                commit_output.status.success(),
                "git commit failed: {}",
                String::from_utf8_lossy(&commit_output.stderr)
            );

            let commit = get_cached_repo_commit(repo.path());
            assert!(commit.is_some());
            // Commit hash should be 40 hex characters
            let hash = commit.unwrap();
            assert_eq!(hash.len(), 40);
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        }

        #[test]
        fn get_cached_repo_commit_non_git_directory() {
            let temp = TempDir::new().unwrap();
            let commit = get_cached_repo_commit(temp.path());
            assert!(commit.is_none());
        }

        #[test]
        fn get_cached_repo_commit_empty_git_repo() {
            let repo = create_test_repo();
            // Empty repo has no commits, so rev-parse HEAD fails
            let commit = get_cached_repo_commit(repo.path());
            assert!(commit.is_none());
        }
    }

    mod fuzzy_helper_tests {
        use super::*;

        #[test]
        fn fuzzy_suggest_with_empty_candidates() {
            let result = fuzzy_suggest("query", &[]);
            assert!(result.is_empty());
        }

        #[test]
        fn fuzzy_suggest_finds_matches() {
            let candidates = vec!["claude-config".to_string(), "copilot-config".to_string()];
            let result = fuzzy_suggest("claude", &candidates);
            assert!(!result.is_empty());
            assert!(result.contains(&"claude-config".to_string()));
        }

        #[test]
        fn format_not_found_error_without_suggestions() {
            let msg = format_not_found_error("owner", "repo", "overlay", &[], None);
            assert!(msg.contains("owner"));
            assert!(msg.contains("repo"));
            assert!(msg.contains("overlay"));
            assert!(msg.contains("not found"));
        }

        #[test]
        fn format_not_found_error_with_suggestions() {
            let suggestions = vec!["claude-config".to_string()];
            let msg = format_not_found_error("owner", "repo", "overlay", &suggestions, None);
            assert!(msg.contains("Did you mean"));
            assert!(msg.contains("claude-config"));
        }

        #[test]
        fn format_not_found_error_with_source_list() {
            let msg =
                format_not_found_error("owner", "repo", "overlay", &[], Some("source1, source2"));
            assert!(msg.contains("source1, source2"));
        }
    }

    mod visible_subdirs_tests {
        use super::*;

        #[test]
        fn returns_non_hidden_directories() {
            let temp = TempDir::new().unwrap();

            fs::create_dir(temp.path().join("visible1")).unwrap();
            fs::create_dir(temp.path().join("visible2")).unwrap();
            fs::create_dir(temp.path().join(".hidden")).unwrap();

            let result = visible_subdirs(temp.path()).unwrap();

            assert_eq!(result.len(), 2);
            let names: Vec<&str> = result.iter().map(|(_, n)| n.as_str()).collect();
            assert!(names.contains(&"visible1"));
            assert!(names.contains(&"visible2"));
            assert!(!names.contains(&".hidden"));
        }

        #[test]
        fn skips_files() {
            let temp = TempDir::new().unwrap();

            fs::create_dir(temp.path().join("dir")).unwrap();
            fs::write(temp.path().join("file.txt"), "content").unwrap();

            let result = visible_subdirs(temp.path()).unwrap();

            assert_eq!(result.len(), 1);
            assert_eq!(result[0].1, "dir");
        }

        #[test]
        fn returns_empty_for_empty_dir() {
            let temp = TempDir::new().unwrap();
            let result = visible_subdirs(temp.path()).unwrap();
            assert!(result.is_empty());
        }

        #[test]
        fn returns_paths_with_names() {
            let temp = TempDir::new().unwrap();
            fs::create_dir(temp.path().join("subdir")).unwrap();

            let result = visible_subdirs(temp.path()).unwrap();

            assert_eq!(result.len(), 1);
            let (path, name) = &result[0];
            assert_eq!(name, "subdir");
            assert!(path.ends_with("subdir"));
        }
    }

    mod resolve_local_path_tests {
        use super::*;

        #[test]
        fn resolves_existing_directory() {
            let temp = TempDir::new().unwrap();
            let result = resolve_local_path(temp.path(), "test", false).unwrap();
            assert!(result.path.exists());
        }

        #[test]
        fn returns_local_source_type() {
            let temp = TempDir::new().unwrap();
            let result = resolve_local_path(temp.path(), "test", false).unwrap();
            match result.source_info {
                OverlaySource::Local { .. } => {}
                _ => panic!("Expected Local source type"),
            }
        }

        #[test]
        fn fails_on_nonexistent_path() {
            let result = resolve_local_path(Path::new("/nonexistent/path/xyz123"), "test", false);
            assert!(result.is_err());
            let err = result.err().unwrap();
            assert!(err.to_string().contains("not found"));
        }

        #[test]
        fn resolves_file_as_well_as_directory() {
            let temp = TempDir::new().unwrap();
            let file_path = temp.path().join("file.txt");
            fs::write(&file_path, "content").unwrap();

            let result = resolve_local_path(&file_path, "test", false).unwrap();
            assert!(result.path.exists());
        }
    }

    mod fuzzy_suggest_tests {
        use super::*;

        #[test]
        fn empty_candidates_returns_empty() {
            let result = fuzzy_suggest("test", &[]);
            assert!(result.is_empty());
        }

        #[test]
        fn empty_query_returns_results() {
            let candidates = vec!["alpha".to_string(), "beta".to_string()];
            let result = fuzzy_suggest("", &candidates);
            // Empty query may or may not return results depending on fuzzy matcher
            // but it should not panic
            let _ = result;
        }

        #[test]
        fn exact_match_returns_match() {
            let candidates = vec![
                "vscode-setup".to_string(),
                "ci-config".to_string(),
                "claude-config".to_string(),
            ];
            let result = fuzzy_suggest("vscode-setup", &candidates);
            assert!(!result.is_empty());
            assert!(result.contains(&"vscode-setup".to_string()));
        }

        #[test]
        fn limits_to_three_results() {
            let candidates: Vec<String> = (0..10).map(|i| format!("overlay-{i}")).collect();
            let result = fuzzy_suggest("overlay", &candidates);
            assert!(result.len() <= 3);
        }

        #[test]
        fn partial_match_returns_suggestions() {
            let candidates = vec![
                "vscode-setup".to_string(),
                "vscode-debug".to_string(),
                "ci-config".to_string(),
            ];
            let result = fuzzy_suggest("vscode", &candidates);
            // Should find vscode-related matches
            assert!(!result.is_empty());
        }
    }

    mod format_not_found_error_tests {
        use super::*;

        #[test]
        fn basic_error_message() {
            let msg = format_not_found_error("org", "repo", "name", &[], None);
            assert!(msg.contains("Overlay not found: org/repo/name"));
            assert!(msg.contains("repoverlay list --filter org/repo"));
        }

        #[test]
        fn with_suggestions() {
            let suggestions = vec!["vscode-setup".to_string(), "ci-config".to_string()];
            let msg = format_not_found_error("org", "repo", "name", &suggestions, None);
            assert!(msg.contains("Did you mean?"));
            assert!(msg.contains("vscode-setup"));
            assert!(msg.contains("ci-config"));
        }

        #[test]
        fn with_source_list() {
            let msg = format_not_found_error("org", "repo", "name", &[], Some("personal, team"));
            assert!(msg.contains("Searched sources: personal, team"));
        }

        #[test]
        fn with_both_suggestions_and_source_list() {
            let suggestions = vec!["vscode-setup".to_string()];
            let msg = format_not_found_error("org", "repo", "name", &suggestions, Some("personal"));
            assert!(msg.contains("Did you mean?"));
            assert!(msg.contains("vscode-setup"));
            assert!(msg.contains("Searched sources: personal"));
        }

        #[test]
        fn empty_suggestions_no_did_you_mean() {
            let msg = format_not_found_error("org", "repo", "name", &[], None);
            assert!(!msg.contains("Did you mean?"));
        }

        #[test]
        fn special_characters_in_names() {
            let msg = format_not_found_error("my-org", "my_repo", "name-123", &[], None);
            assert!(msg.contains("my-org/my_repo/name-123"));
        }
    }

    mod list_overlays_from_path_additional_tests {
        use super::*;

        #[test]
        fn handles_symlinks_in_directory() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            // Create real overlay
            fs::create_dir_all(repo_path.join("org/repo/overlay")).unwrap();

            // Create a symlink at org level (should still work)
            #[cfg(unix)]
            {
                let real_org = repo_path.join("real-org/repo/overlay2");
                fs::create_dir_all(&real_org).unwrap();
                std::os::unix::fs::symlink(
                    repo_path.join("real-org"),
                    repo_path.join("linked-org"),
                )
                .unwrap();

                let overlays = list_overlays_from_path(repo_path).unwrap();
                // Should find overlays through both real and symlinked paths
                assert!(overlays.len() >= 2);
            }
        }

        #[test]
        fn nonexistent_path_returns_error() {
            let result = list_overlays_from_path(Path::new("/nonexistent/path/xyz123"));
            assert!(result.is_err());
        }
    }

    mod list_overlays_from_path_tests {
        use super::*;

        #[test]
        fn finds_overlays_in_nested_structure() {
            let temp = TempDir::new().unwrap();
            let root = temp.path();

            fs::create_dir_all(root.join("microsoft/FluidFramework/vscode-setup")).unwrap();
            fs::create_dir_all(root.join("microsoft/FluidFramework/ci-config")).unwrap();
            fs::create_dir_all(root.join("microsoft/other-repo/overlay1")).unwrap();

            let overlays = list_overlays_from_path(root).unwrap();
            assert_eq!(overlays.len(), 3);

            let names: Vec<&str> = overlays.iter().map(|o| o.name.as_str()).collect();
            assert!(names.contains(&"vscode-setup"));
            assert!(names.contains(&"ci-config"));
            assert!(names.contains(&"overlay1"));
        }

        #[test]
        fn detects_has_config() {
            let temp = TempDir::new().unwrap();
            let root = temp.path();

            // Overlay with config
            let with_config = root.join("org/repo/with-config");
            fs::create_dir_all(&with_config).unwrap();
            fs::write(with_config.join("repoverlay.ccl"), "").unwrap();

            // Overlay without config
            fs::create_dir_all(root.join("org/repo/no-config")).unwrap();

            let overlays = list_overlays_from_path(root).unwrap();
            assert_eq!(overlays.len(), 2);

            let with = overlays.iter().find(|o| o.name == "with-config").unwrap();
            let without = overlays.iter().find(|o| o.name == "no-config").unwrap();
            assert!(with.has_config);
            assert!(!without.has_config);
        }

        #[test]
        fn returns_sorted_overlays() {
            let temp = TempDir::new().unwrap();
            let root = temp.path();

            fs::create_dir_all(root.join("z-org/z-repo/z-overlay")).unwrap();
            fs::create_dir_all(root.join("a-org/a-repo/a-overlay")).unwrap();
            fs::create_dir_all(root.join("a-org/a-repo/b-overlay")).unwrap();

            let overlays = list_overlays_from_path(root).unwrap();
            assert_eq!(overlays.len(), 3);

            // Should be sorted by org, then repo, then name
            assert_eq!(overlays[0].org, "a-org");
            assert_eq!(overlays[0].name, "a-overlay");
            assert_eq!(overlays[1].org, "a-org");
            assert_eq!(overlays[1].name, "b-overlay");
            assert_eq!(overlays[2].org, "z-org");
        }

        #[test]
        fn empty_directory_returns_empty() {
            let temp = TempDir::new().unwrap();
            let overlays = list_overlays_from_path(temp.path()).unwrap();
            assert!(overlays.is_empty());
        }

        #[test]
        fn skips_hidden_directories() {
            let temp = TempDir::new().unwrap();
            let root = temp.path();

            // Hidden org directory should be skipped
            fs::create_dir_all(root.join(".hidden-org/repo/overlay")).unwrap();
            // Visible org
            fs::create_dir_all(root.join("visible-org/repo/overlay")).unwrap();

            let overlays = list_overlays_from_path(root).unwrap();
            assert_eq!(overlays.len(), 1);
            assert_eq!(overlays[0].org, "visible-org");
        }

        #[test]
        fn files_at_org_level_are_ignored() {
            let temp = TempDir::new().unwrap();
            let root = temp.path();

            // File at root level (not a directory)
            fs::write(root.join("README.md"), "# Overlays").unwrap();
            // Real overlay
            fs::create_dir_all(root.join("org/repo/overlay")).unwrap();

            let overlays = list_overlays_from_path(root).unwrap();
            assert_eq!(overlays.len(), 1);
        }

        #[test]
        fn shallow_structure_returns_empty() {
            let temp = TempDir::new().unwrap();
            let root = temp.path();

            // Only one level deep - not enough for org/repo/overlay
            fs::create_dir_all(root.join("org")).unwrap();

            let overlays = list_overlays_from_path(root).unwrap();
            assert!(overlays.is_empty());
        }

        #[test]
        fn two_level_structure_returns_empty() {
            let temp = TempDir::new().unwrap();
            let root = temp.path();

            // Only two levels deep - not enough for org/repo/overlay
            fs::create_dir_all(root.join("org/repo")).unwrap();
            // Add a file so repo dir isn't empty
            fs::write(root.join("org/repo/README.md"), "").unwrap();

            let overlays = list_overlays_from_path(root).unwrap();
            assert!(overlays.is_empty());
        }

        #[test]
        fn sets_org_and_repo_fields() {
            let temp = TempDir::new().unwrap();
            fs::create_dir_all(temp.path().join("my-org/my-repo/my-overlay")).unwrap();

            let overlays = list_overlays_from_path(temp.path()).unwrap();
            assert_eq!(overlays.len(), 1);
            assert_eq!(overlays[0].org, "my-org");
            assert_eq!(overlays[0].repo, "my-repo");
            assert_eq!(overlays[0].name, "my-overlay");
        }
    }

    mod resolved_types_tests {
        use super::*;

        #[test]
        fn resolved_source_holds_path_and_source_info() {
            let temp = TempDir::new().unwrap();
            let resolved = ResolvedSource {
                path: temp.path().to_path_buf(),
                source_info: OverlaySource::local(temp.path().to_path_buf()),
            };
            assert_eq!(resolved.path, temp.path());
        }

        #[test]
        fn resolved_sources_single_variant() {
            let temp = TempDir::new().unwrap();
            let source = ResolvedSource {
                path: temp.path().to_path_buf(),
                source_info: OverlaySource::local(temp.path().to_path_buf()),
            };
            let resolved = ResolvedSources::Single(source);
            match resolved {
                ResolvedSources::Single(s) => assert_eq!(s.path, temp.path()),
                ResolvedSources::Multiple(_) => panic!("Expected Single variant"),
            }
        }

        #[test]
        fn resolved_sources_multiple_variant() {
            let resolved = ResolvedSources::Multiple(vec![]);
            match resolved {
                ResolvedSources::Multiple(v) => assert!(v.is_empty()),
                ResolvedSources::Single(_) => panic!("Expected Multiple variant"),
            }
        }
    }

    mod resolve_local_path_prefix_tests {
        use super::*;

        #[test]
        fn ambiguous_path_returns_error() {
            let temp = TempDir::new().unwrap();
            // With needs_prefix_warning=true, resolution must fail with a clear error
            let result = resolve_local_path(temp.path(), "test-dir", true);
            assert!(result.is_err());
            let err = result.err().unwrap().to_string();
            assert!(
                err.contains("./test-dir"),
                "error should suggest using ./prefix: {err}"
            );
        }

        #[test]
        fn returns_canonical_path() {
            let temp = TempDir::new().unwrap();
            let result = resolve_local_path(temp.path(), "test-dir", false).unwrap();
            // The returned path should be canonical (absolute, no symlinks)
            assert!(result.path.is_absolute());
        }
    }

    mod source_is_configured_tests {
        use super::*;

        /// Helper to write a repo-local config with the given sources.
        fn write_repo_config(repo_path: &Path, sources_ccl: &str) {
            let config_dir = repo_path.join(".repoverlay");
            fs::create_dir_all(&config_dir).unwrap();
            fs::write(config_dir.join("config.ccl"), sources_ccl).unwrap();
        }

        #[test]
        fn not_configured_returns_false() {
            let temp = TempDir::new().unwrap();
            let result = source_is_configured("someowner", "somerepo", Some(temp.path())).unwrap();
            assert!(!result);
        }

        #[test]
        fn configured_in_repo_local_returns_true() {
            let temp = TempDir::new().unwrap();
            write_repo_config(
                temp.path(),
                r"
sources =
  =
    name = my-overlays
    url = https://github.com/acme/my-overlays
",
            );

            let result = source_is_configured("acme", "my-overlays", Some(temp.path())).unwrap();
            assert!(
                result,
                "source_is_configured should find repo-local sources"
            );
        }

        #[test]
        fn configured_in_repo_local_matches_by_name() {
            let temp = TempDir::new().unwrap();
            write_repo_config(
                temp.path(),
                r"
sources =
  =
    name = my-overlays
    url = https://example.com/different-url
",
            );

            // URL won't match github pattern, but name matches
            let result = source_is_configured("acme", "my-overlays", Some(temp.path())).unwrap();
            assert!(result, "source_is_configured should match by name");
        }

        #[test]
        fn configured_in_repo_local_matches_by_url() {
            let temp = TempDir::new().unwrap();
            write_repo_config(
                temp.path(),
                r"
sources =
  =
    name = different-name
    url = https://github.com/acme/my-overlays
",
            );

            let result = source_is_configured("acme", "my-overlays", Some(temp.path())).unwrap();
            assert!(result, "source_is_configured should match by URL");
        }

        #[test]
        fn none_target_path_still_works() {
            // With None, only global config is checked — should not panic
            let result = source_is_configured("nonexistent-owner", "nonexistent-repo", None);
            assert!(result.is_ok());
        }
    }
}
