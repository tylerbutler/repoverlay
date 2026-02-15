//! repoverlay - Overlay config files into git repositories without committing them.
//!
//! This is a CLI tool. There is no public library API.

mod cache;
mod cli;
mod config;
mod detection;
mod fuzzy;
mod github;
mod json_merge;
mod overlay_repo;
mod reference;
mod selection;
mod sources;
mod state;
#[cfg(test)]
mod testutil;
mod upstream;

/// Run the CLI application.
///
/// This is the only public entry point. All other functionality is internal.
pub fn run() -> anyhow::Result<()> {
    cli::run()
}

// Internal imports for use within the crate
use anyhow::{Context, Result, bail};
use colored::Colorize;
use log::{debug, trace};

use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use cache::CacheManager;
use fuzzy::OverlayMatcher;
use github::GitHubSource;
use json_merge::{is_json_file, merge_json_files};
use overlay_repo::copy_dir_recursive;
use reference::SourceReference;
use selection::is_interactive;
use state::{
    CONFIG_FILE, EntryType, FileEntry, GlobalMeta, LinkType, MANAGED_SECTION_NAME, META_FILE,
    OVERLAYS_DIR, OverlayConfig, OverlaySource, OverlayState, STATE_DIR, exclude_marker_end,
    exclude_marker_start, list_applied_overlays, load_all_overlay_targets, load_external_states,
    load_overlay_state, normalize_overlay_name, remove_external_state, save_external_state,
    save_overlay_state,
};
use upstream::detect_upstream;

/// Strategy for handling conflicts during overlay application.
///
/// Controls behavior when applying an overlay encounters conflicts with
/// existing files in the repository or with other applied overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ConflictStrategy {
    /// Fail immediately on any conflict (default behavior).
    #[default]
    Fail,

    /// Overwrite existing unmanaged files and re-apply same-name overlays.
    ///
    /// - For same-name overlays: removes existing overlay first, then re-applies
    /// - For existing repo files: overwrites them
    /// - For cross-overlay conflicts (files managed by another overlay): still fails
    ///   to prevent accidentally breaking other overlays
    Force,

    /// Skip conflicting files silently, continue with non-conflicting files.
    ///
    /// - For cross-overlay conflicts: skips the file with a warning
    /// - For existing repo files: skips the file with a warning
    /// - Logs skipped files but does not error
    SkipConflicts,
}

/// Canonicalize a path and return an error with a descriptive message if it fails.
pub(crate) fn canonicalize_path(path: &Path, description: &str) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("{} not found: {}", description, path.display()))
}

/// Validate that a path is a git repository (has a .git directory or file).
pub(crate) fn validate_git_repo(path: &Path) -> Result<()> {
    if !path.join(".git").exists() {
        bail!("Target is not a git repository: {}", path.display());
    }
    Ok(())
}

/// Resolve the actual git directory for a repository.
///
/// In a regular git repository, `.git` is a directory containing the git database.
/// In a git worktree, `.git` is a file containing `gitdir: /path/to/git/dir`.
/// This function handles both cases and returns the path to the actual git directory.
pub(crate) fn resolve_git_dir(repo_path: &Path) -> Result<PathBuf> {
    let git_path = repo_path.join(".git");

    if git_path.is_dir() {
        // Regular git repository
        return Ok(git_path);
    }

    if git_path.is_file() {
        // Git worktree - .git is a file containing "gitdir: /path/to/git/dir"
        let content = fs::read_to_string(&git_path)
            .with_context(|| format!("Failed to read .git file: {}", git_path.display()))?;

        for line in content.lines() {
            let line = line.trim();
            if let Some(path_str) = line.strip_prefix("gitdir:") {
                let path_str = path_str.trim();
                let gitdir = PathBuf::from(path_str);

                // Handle relative paths (relative to repo_path)
                let gitdir = if gitdir.is_absolute() {
                    gitdir
                } else {
                    repo_path.join(gitdir)
                };

                return gitdir.canonicalize().with_context(|| {
                    format!("Failed to resolve gitdir path: {}", gitdir.display())
                });
            }
        }

        bail!(
            "Invalid .git file (no gitdir found): {}",
            git_path.display()
        );
    }

    bail!("Not a git repository: {}", repo_path.display());
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

    // Parse input into structured reference
    let reference = SourceReference::parse(source_str);
    debug!("parsed reference: {reference:?}");

    match reference {
        SourceReference::GitHubUrl(url) => {
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
        ),

        SourceReference::OnePart { username } => {
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
            )
        }
    }
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

    println!(
        "{} repository: {}/{}",
        if update { "Updating" } else { "Fetching" }.blue().bold(),
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
fn resolve_local_path(
    path: &Path,
    original_input: &str,
    needs_prefix_warning: bool,
) -> Result<ResolvedSource> {
    debug!("resolving local path: {}", path.display());

    // Emit deprecation warning for ambiguous paths
    // TODO: In a future version, require `./` prefix for local paths
    if needs_prefix_warning {
        eprintln!(
            "{}: Local path '{}' matched. In a future version, use './{original_input}' for local paths.",
            "Warning".yellow().bold(),
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
    _target_path: Option<&Path>,
    _source_filter: Option<&str>,
) -> Result<ResolvedSources> {
    debug!("resolving two-part reference (GitHub browse mode): {owner}/{repo}");

    // Create a GitHubSource to fetch/cache the repo
    let github_url = format!("https://github.com/{owner}/{repo}");
    let mut github_source = GitHubSource::parse(&github_url)?;

    if let Some(ref_str) = ref_override {
        github_source = github_source.with_ref_override(Some(ref_str))?;
    }

    // Fetch/cache the repository
    let cache = CacheManager::new()?;
    println!(
        "{} repository: {}/{}",
        if update { "Updating" } else { "Fetching" }.blue().bold(),
        owner,
        repo
    );
    let cached = cache.ensure_cached(&github_source, update)?;

    // List available overlays (returns full paths like "microsoft/FluidFramework/overlay-name")
    let available_overlays = list_overlays_from_cached_repo(owner, repo)?;

    if available_overlays.is_empty() {
        bail!(
            "No overlays found in {owner}/{repo}.\n\n\
             Make sure the repository contains overlay directories in the format:\n\
             <target-org>/<target-repo>/<overlay-name>/"
        );
    }

    // Select overlays based on interactivity
    let selected_overlays = if is_interactive() {
        select_overlays_interactive(owner, repo, &available_overlays)?
    } else {
        // Non-interactive mode - error with available overlays
        let overlay_list = available_overlays
            .iter()
            .map(|o| format!("  {}", format_overlay_path(o)))
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
            format_overlay_path(&selected_overlays[0])
        );
    } else {
        println!(
            "{} {} overlays:",
            "Selected".green().bold(),
            selected_overlays.len()
        );
        for overlay_path in &selected_overlays {
            println!("  - {}", format_overlay_path(overlay_path));
        }
    }

    // Resolve each selected overlay to a ResolvedSource
    let git_ref_str = github_source.git_ref.as_str().to_string();
    let commit = get_cached_repo_commit(&cached.path).unwrap_or_else(|| "unknown".to_string());

    let mut resolved_sources = Vec::with_capacity(selected_overlays.len());

    for selected_overlay in &selected_overlays {
        let (target_org, target_repo, overlay_name) = parse_overlay_path(selected_overlay)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid overlay path format: {selected_overlay}\n\
                     Expected: <target-org>/<target-repo>/<overlay-name>"
                )
            })?;

        let overlay_path = cached
            .path
            .join(target_org)
            .join(target_repo)
            .join(overlay_name);

        if !overlay_path.exists() {
            bail!("Overlay directory not found: {}", overlay_path.display());
        }

        resolved_sources.push(ResolvedSource {
            path: overlay_path,
            source_info: OverlaySource::github(
                github_url.clone(),
                owner.to_string(),
                repo.to_string(),
                git_ref_str.clone(),
                commit.clone(),
                Some(format!("{target_org}/{target_repo}/{overlay_name}")),
            ),
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

/// Get the current commit hash from a cached repository.
fn get_cached_repo_commit(repo_path: &Path) -> Option<String> {
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
/// Returns full paths like `microsoft/FluidFramework/vscode-setup`.
///
/// Note: This is the same structure used by `OverlayRepoManager::list_overlays()`,
/// but operates on the GitHub cache instead of managed overlay repositories.
fn list_overlays_from_cached_repo(owner: &str, repo: &str) -> Result<Vec<String>> {
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
fn list_overlays_from_path(repo_path: &Path) -> Result<Vec<String>> {
    let mut overlays = Vec::new();

    // Walk the three-level structure: org/repo/overlay
    for (org_path, org_name) in visible_subdirs(repo_path)? {
        for (repo_path, repo_name) in visible_subdirs(&org_path)? {
            for (_overlay_path, overlay_name) in visible_subdirs(&repo_path)? {
                overlays.push(format!("{org_name}/{repo_name}/{overlay_name}"));
            }
        }
    }

    overlays.sort();
    debug!("found {} overlays in path", overlays.len());
    Ok(overlays)
}

/// Returns visible (non-hidden) subdirectories with their names.
fn visible_subdirs(path: &Path) -> Result<Vec<(PathBuf, String)>> {
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

/// Parse an overlay path string into `(org, repo, overlay_name)` components.
///
/// Returns None if the path doesn't have exactly 3 components.
fn parse_overlay_path(path: &str) -> Option<(&str, &str, &str)> {
    let (org, rest) = path.split_once('/')?;
    let (repo, overlay) = rest.split_once('/')?;
    // Reject paths with more than 3 components
    if overlay.contains('/') {
        None
    } else {
        Some((org, repo, overlay))
    }
}

/// Format an overlay path for display with the overlay name in bold.
///
/// Input: `"microsoft/FluidFramework/vscode-setup"`
/// Output: `"microsoft/FluidFramework/vscode-setup"` (with "vscode-setup" in bold)
fn format_overlay_path(path: &str) -> String {
    if let Some((org, repo, overlay)) = parse_overlay_path(path) {
        format!("{org}/{repo}/{}", overlay.bold())
    } else {
        path.to_string()
    }
}

/// Present an interactive multi-select picker for overlays.
///
/// Uses `dialoguer::MultiSelect` to allow selecting one or more overlays.
/// Space toggles selection, Enter confirms.
fn select_overlays_interactive(
    owner: &str,
    repo: &str,
    overlays: &[String],
) -> Result<Vec<String>> {
    use dialoguer::{MultiSelect, theme::ColorfulTheme};

    println!(
        "\n{} Select overlay(s) from {}/{} (Space to toggle, Enter to confirm):\n",
        "?".cyan().bold(),
        owner,
        repo
    );

    // Format overlays for display with bold overlay names
    let display_items: Vec<String> = overlays.iter().map(|o| format_overlay_path(o)).collect();

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .items(&display_items)
        .interact_opt()
        .context("Failed to show overlay picker")?;

    match selections {
        Some(indices) if !indices.is_empty() => {
            Ok(indices.into_iter().map(|i| overlays[i].clone()).collect())
        }
        _ => bail!("No overlays selected"),
    }
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

    // Load config
    let config = config::load_config(None)?;

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
        );
    }

    // Fall back to legacy overlay_repo config. Will be removed in 1.0 (#79).
    let overlay_config = config.overlay_repo.ok_or_else(|| {
        anyhow::anyhow!(
            "Overlay repository not configured.\n\n\
             To apply overlays from a shared repository, first run:\n\
             repoverlay source add <url>\n\n\
             Or use a local path or GitHub URL instead."
        )
    })?;

    let manager = overlay_repo::OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    if update {
        println!("{} overlay repository...", "Updating".blue().bold());
        manager.pull()?;
    }

    // Try to resolve with fallback
    if let Ok((overlay_path, resolved_via)) =
        manager.get_overlay_path_with_fallback(org, repo, name, upstream.as_ref())
    {
        let commit = manager.get_current_commit()?;

        // Determine actual org/repo for state tracking
        let via_upstream = resolved_via == state::ResolvedVia::Upstream;
        let (actual_org, actual_repo) = match (&upstream, via_upstream) {
            (Some(up), true) => (up.org.clone(), up.repo.clone()),
            _ => (org.to_string(), repo.to_string()),
        };

        let via_suffix = if via_upstream {
            " (via upstream)".dimmed().to_string()
        } else {
            String::new()
        };
        println!(
            "{} overlay: {}/{}/{}{}",
            "Resolving".blue().bold(),
            actual_org,
            actual_repo,
            name,
            via_suffix
        );

        return Ok(ResolvedSource {
            path: overlay_path,
            source_info: OverlaySource::overlay_repo_with_resolution(
                actual_org,
                actual_repo,
                name.to_string(),
                commit,
                resolved_via,
            ),
        });
    }

    // Overlay not found - provide fuzzy suggestions
    let suggestions = get_fuzzy_suggestions_legacy(&manager, org, repo, name);
    let error_msg = format_not_found_error(org, repo, name, &suggestions, None);
    bail!("{error_msg}")
}

/// Resolve an overlay from configured sources with fuzzy suggestions on failure.
fn resolve_from_sources_with_suggestions(
    sources: &[config::Source],
    org: &str,
    repo: &str,
    name: &str,
    upstream: Option<&upstream::UpstreamInfo>,
    source_filter: Option<&str>,
    update: bool,
) -> Result<ResolvedSource> {
    let manager = sources::SourceManager::new(sources.to_vec())?;

    // Ensure all sources are cloned
    manager.ensure_all_cloned()?;

    if update {
        println!("{} overlay sources...", "Updating".blue().bold());
        manager.pull_all()?;
    }

    // Resolve overlay from sources
    if let Some(resolved) = manager.resolve(org, repo, name, upstream, source_filter)? {
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

/// Get fuzzy suggestions for overlay names from legacy single-source config.
fn get_fuzzy_suggestions_legacy(
    manager: &overlay_repo::OverlayRepoManager,
    org: &str,
    repo: &str,
    query: &str,
) -> Vec<String> {
    let available = match manager.list_overlays_for_repo(org, repo) {
        Ok(overlays) => overlays.into_iter().map(|o| o.name).collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };
    fuzzy_suggest(query, &available)
}

/// Get fuzzy suggestions for overlay names from multi-source config.
fn get_fuzzy_suggestions_multi_source(
    manager: &sources::SourceManager,
    org: &str,
    repo: &str,
    query: &str,
) -> Vec<String> {
    let available = manager.list_overlays_for_repo(org, repo);
    fuzzy_suggest(query, &available)
}

/// Find fuzzy matches for a query in the given candidates.
fn fuzzy_suggest(query: &str, candidates: &[String]) -> Vec<String> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let matcher = OverlayMatcher::new();
    matcher.suggest(query, candidates, 3)
}

/// Format a "not found" error message with optional fuzzy suggestions and source list.
fn format_not_found_error(
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

/// Apply an overlay to a target git repository.
///
/// # Workflow
///
/// 1. Resolve source location (local path, GitHub URL, or overlay repo)
/// 2. Validate target is a git repository
/// 3. Load overlay config (`repoverlay.ccl`) if present
/// 4. Determine overlay name (CLI override > config > directory name)
/// 5. Check for conflicts with existing overlays and files
/// 6. Create symlinks or copies for each file
/// 7. Update `.git/info/exclude` with overlay section
/// 8. Save state to `.repoverlay/overlays/<name>.ccl`
/// 9. Save external backup for restore capability
///
/// # Errors
///
/// Returns an error if:
/// - Source resolution fails
/// - Target is not a git repository
/// - Overlay with same name already exists (unless using `Force` strategy)
/// - File conflicts with existing overlay or repo file (unless using `Force` or `SkipConflicts`)
/// - No files found in overlay source
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub(crate) fn apply_overlay(
    source_str: &str,
    target: &Path,
    force_copy: bool,
    name_override: Option<String>,
    ref_override: Option<&str>,
    update_cache: bool,
    conflict_strategy: ConflictStrategy,
    merge: bool,
    source_filter: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    debug!(
        "apply_overlay: source={}, target={}, force_copy={}, name_override={:?}, conflict_strategy={:?}, dry_run={}",
        source_str,
        target.display(),
        force_copy,
        name_override,
        conflict_strategy,
        dry_run
    );

    // Resolve source (handles GitHub URLs and local paths)
    // Pass target to enable upstream detection for fork inheritance
    let resolved = resolve_source(
        source_str,
        ref_override,
        update_cache,
        Some(target),
        source_filter,
    )?;

    // Handle multi-select from browse mode
    let resolved = match resolved {
        ResolvedSources::Single(single) => single,
        ResolvedSources::Multiple(sources) => {
            return apply_multiple_overlays(
                &sources,
                target,
                force_copy,
                dry_run,
                conflict_strategy,
                merge,
            );
        }
    };

    if dry_run {
        println!("{} Dry run - no changes made.", "Note:".yellow());
        println!("\nWould apply overlay from: {}", resolved.path.display());
        return Ok(());
    }

    // Validate target exists and is a git repo
    let target = canonicalize_path(target, "Target directory")?;
    validate_git_repo(&target)?;

    apply_resolved_overlay(
        &resolved,
        &target,
        force_copy,
        name_override,
        conflict_strategy,
        merge,
    )
}

/// Apply a single resolved overlay to a target repository.
///
/// This contains the core overlay application logic, separated from source resolution
/// so it can be reused by both single-apply and multi-apply paths.
fn apply_resolved_overlay(
    resolved: &ResolvedSource,
    target: &Path,
    force_copy: bool,
    name_override: Option<String>,
    conflict_strategy: ConflictStrategy,
    merge: bool,
) -> Result<()> {
    let source = &resolved.path;
    debug!("resolved source path: {}", source.display());

    // Determine link type
    let link_type = if force_copy || cfg!(windows) {
        LinkType::Copy
    } else {
        LinkType::Symlink
    };

    // Load overlay config (optional)
    let config = load_overlay_config(source)?;

    // Determine overlay name (priority: CLI override > config > directory name)
    let overlay_name = resolve_overlay_display_name(&config, source, name_override);
    let normalized_name = normalize_overlay_name(&overlay_name)?;

    // Check if this specific overlay already exists
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    let overlay_state_path = overlays_dir.join(format!("{normalized_name}.ccl"));
    if overlay_state_path.exists() {
        match conflict_strategy {
            ConflictStrategy::Force => {
                println!(
                    "  {} Removing existing overlay '{}'",
                    "Force:".yellow(),
                    overlay_name
                );
                remove_single_overlay(target, &overlays_dir, &normalized_name)?;
            }
            ConflictStrategy::Fail | ConflictStrategy::SkipConflicts => {
                bail!(
                    "Overlay '{overlay_name}' is already applied. Run 'repoverlay remove {normalized_name}' first, or use --force."
                );
            }
        }
    }

    // Load all existing overlay targets to check for conflicts
    let existing_targets = load_all_overlay_targets(target)?;

    println!("{} overlay: {}", "Applying".green().bold(), overlay_name);

    // Collect files to overlay and build state
    let mut state = OverlayState::new(overlay_name.clone(), resolved.source_info.clone());
    let mut exclude_entries: Vec<String> = Vec::new();

    // Process directories first (symlink as units)
    for dir_name in &config.directories {
        let dir_path = PathBuf::from(dir_name);
        let source_dir = source.join(&dir_path);

        // Check if directory exists
        if !source_dir.exists() {
            eprintln!(
                "  {} Directory not found, skipping: {}",
                "Warning:".yellow(),
                dir_name
            );
            continue;
        }

        if !source_dir.is_dir() {
            eprintln!(
                "  {} Path is not a directory, skipping: {}",
                "Warning:".yellow(),
                dir_name
            );
            continue;
        }

        // Check for conflicts with existing overlays
        let dir_rel_str = dir_path.to_string_lossy().to_string();
        if let Some(conflicting_overlay) = existing_targets.get(&dir_rel_str) {
            match conflict_strategy {
                ConflictStrategy::SkipConflicts => {
                    eprintln!(
                        "  {} Skipping directory '{}' (managed by overlay '{}')",
                        "Skip:".yellow(),
                        dir_path.display(),
                        conflicting_overlay
                    );
                    continue;
                }
                ConflictStrategy::Fail | ConflictStrategy::Force => {
                    bail!(
                        "Conflict: directory '{}' is already managed by overlay '{}'\n\
                         Remove that overlay first, use --skip-conflicts, or use different file mappings.",
                        dir_path.display(),
                        conflicting_overlay
                    );
                }
            }
        }

        let target_dir = target.join(&dir_path);

        // Check for conflicts with existing files/dirs in repo
        if target_dir.exists() {
            match conflict_strategy {
                ConflictStrategy::Force => {
                    eprintln!(
                        "  {} Overwriting existing directory: {}",
                        "Force:".yellow(),
                        dir_path.display()
                    );
                    fs::remove_dir_all(&target_dir).with_context(|| {
                        format!(
                            "Failed to remove existing directory: {}",
                            target_dir.display()
                        )
                    })?;
                }
                ConflictStrategy::SkipConflicts => {
                    eprintln!(
                        "  {} Skipping directory '{}' (already exists)",
                        "Skip:".yellow(),
                        dir_path.display()
                    );
                    continue;
                }
                ConflictStrategy::Fail => {
                    bail!(
                        "Conflict: target path already exists: {}\n\
                         Remove it first, use --force, or use --skip-conflicts.",
                        target_dir.display()
                    );
                }
            }
        }

        // Create parent directories if needed
        if let Some(parent) = target_dir.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Create directory symlink or copy
        match link_type {
            LinkType::Symlink => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&source_dir, &target_dir).with_context(|| {
                    format!(
                        "Failed to create directory symlink: {}",
                        target_dir.display()
                    )
                })?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(&source_dir, &target_dir).with_context(|| {
                    format!(
                        "Failed to create directory symlink: {}",
                        target_dir.display()
                    )
                })?;
            }
            LinkType::Copy | LinkType::Merged => {
                // For copy/merged mode, create the target directory and recursively copy contents
                fs::create_dir_all(&target_dir).with_context(|| {
                    format!("Failed to create directory: {}", target_dir.display())
                })?;
                copy_dir_recursive(&source_dir, &target_dir).with_context(|| {
                    format!("Failed to copy directory: {}", target_dir.display())
                })?;
            }
        }

        println!("  {} {}/", "+".green(), dir_path.display());

        state.add_file(FileEntry {
            source: dir_path.clone(),
            target: dir_path.clone(),
            link_type,
            entry_type: EntryType::Directory,
        });

        // Add to exclude list with trailing slash for directories
        let exclude_path = format!("{}/", dir_path.to_string_lossy().replace('\\', "/"));
        exclude_entries.push(exclude_path);
    }

    for (rel_path, target_rel_str) in collect_overlay_files(source, &config) {
        let rel_str = rel_path.to_string_lossy().to_string();
        let target_rel = PathBuf::from(&target_rel_str);
        let source_file = source.join(&rel_path);
        let target_file = target.join(&target_rel);

        // Validate that the target file is within the target directory (prevent path traversal)
        // We need to resolve the path to handle .. components, but the file doesn't exist yet.
        // So we create parent dirs first (if needed) and then check the canonical path.
        // Alternative: check if the path contains .. that escapes the target.
        {
            // Normalize the path by iterating through components
            let mut normalized = target.to_path_buf();
            for component in target_rel.components() {
                use std::path::Component;
                match component {
                    Component::ParentDir => {
                        // Check if going up would escape the target directory
                        if !normalized.starts_with(target) || normalized == *target {
                            bail!(
                                "Path traversal detected: mapping '{}' -> '{}' would escape target directory",
                                rel_str,
                                target_rel.display()
                            );
                        }
                        normalized.pop();
                    }
                    Component::Normal(c) => {
                        normalized.push(c);
                    }
                    Component::CurDir => {} // Skip . components
                    Component::RootDir | Component::Prefix(_) => {
                        bail!(
                            "Absolute paths not allowed in mappings: '{}' -> '{}'",
                            rel_str,
                            target_rel.display()
                        );
                    }
                }
            }
            // After processing, ensure we're still within target
            if !normalized.starts_with(target) {
                bail!(
                    "Path traversal detected: mapping '{}' -> '{}' would escape target directory",
                    rel_str,
                    target_rel.display()
                );
            }
        }

        // Check for conflicts with existing overlays
        if let Some(conflicting_overlay) = existing_targets.get(&target_rel_str) {
            if merge && is_json_file(&target_rel) && target_file.exists() {
                eprintln!(
                    "  {} Merging '{}' (managed by overlay '{}')",
                    "Merge:".cyan(),
                    target_rel.display(),
                    conflicting_overlay
                );
                if let Some((entry, exclude_path)) =
                    try_merge_json(&target_file, &source_file, &target_rel, &rel_path)
                {
                    state.add_file(entry);
                    exclude_entries.push(exclude_path);
                    continue;
                }
                // Merge failed; fall through to existing conflict handling
            }
            match conflict_strategy {
                ConflictStrategy::SkipConflicts => {
                    eprintln!(
                        "  {} Skipping file '{}' (managed by overlay '{}')",
                        "Skip:".yellow(),
                        target_rel.display(),
                        conflicting_overlay
                    );
                    continue;
                }
                ConflictStrategy::Fail | ConflictStrategy::Force => {
                    bail!(
                        "Conflict: file '{}' is already managed by overlay '{}'\n\
                         Remove that overlay first, use --skip-conflicts, or use different file mappings.",
                        target_rel.display(),
                        conflicting_overlay
                    );
                }
            }
        }

        // Check for conflicts with existing files in repo
        if target_file.exists() {
            if merge && is_json_file(&target_rel) {
                eprintln!(
                    "  {} Merging '{}' with existing repo file",
                    "Merge:".cyan(),
                    target_rel.display()
                );
                if let Some((entry, exclude_path)) =
                    try_merge_json(&target_file, &source_file, &target_rel, &rel_path)
                {
                    state.add_file(entry);
                    exclude_entries.push(exclude_path);
                    continue;
                }
                // Merge failed; fall through to existing conflict handling
            }
            match conflict_strategy {
                ConflictStrategy::Force => {
                    eprintln!(
                        "  {} Overwriting existing file: {}",
                        "Force:".yellow(),
                        target_rel.display()
                    );
                    fs::remove_file(&target_file).with_context(|| {
                        format!("Failed to remove existing file: {}", target_file.display())
                    })?;
                }
                ConflictStrategy::SkipConflicts => {
                    eprintln!(
                        "  {} Skipping file '{}' (already exists)",
                        "Skip:".yellow(),
                        target_rel.display()
                    );
                    continue;
                }
                ConflictStrategy::Fail => {
                    bail!(
                        "Conflict: target file already exists: {}\n\
                         Remove it first, use --force, or use --skip-conflicts.",
                        target_file.display()
                    );
                }
            }
        }

        // Create parent directories if needed
        if let Some(parent) = target_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Create symlink or copy
        trace!(
            "linking {} -> {} ({:?})",
            source_file.display(),
            target_file.display(),
            link_type
        );
        match link_type {
            LinkType::Symlink => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&source_file, &target_file).with_context(|| {
                    format!("Failed to create symlink: {}", target_file.display())
                })?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(&source_file, &target_file).with_context(
                    || format!("Failed to create symlink: {}", target_file.display()),
                )?;
            }
            LinkType::Copy => {
                fs::copy(&source_file, &target_file)
                    .with_context(|| format!("Failed to copy file: {}", target_file.display()))?;
            }
            LinkType::Merged => {
                // Merged files are handled earlier in the conflict resolution path.
                unreachable!("Merged link type should not reach file copy path");
            }
        }

        println!("  {} {}", "+".green(), target_rel.display());

        state.add_file(FileEntry {
            source: rel_path.clone(),
            target: target_rel.clone(),
            link_type,
            entry_type: EntryType::File,
        });

        // Add to exclude list (use forward slashes for git)
        let exclude_path = target_rel.to_string_lossy().replace('\\', "/");
        exclude_entries.push(exclude_path);
    }

    if state.file_count() == 0 {
        bail!("No files found in overlay source: {}", source.display());
    }

    // Update .git/info/exclude with this overlay's entries
    update_git_exclude(target, &normalized_name, &exclude_entries, true)?;

    // Ensure state directories exist
    fs::create_dir_all(&overlays_dir)?;

    // Write global meta if this is the first overlay
    let meta_path = target.join(STATE_DIR).join(META_FILE);
    if !meta_path.exists() {
        let global_meta = GlobalMeta::default();
        let meta_content =
            sickle::to_string(&global_meta).context("Failed to serialize global meta")?;
        fs::write(&meta_path, meta_content)?;
    }

    // Save overlay state to in-repo location
    save_overlay_state(target, &state)?;

    // Save external backup for restore capability
    if let Err(e) = save_external_state(target, &normalized_name, &state) {
        eprintln!(
            "  {} Could not save external backup: {}",
            "Warning:".yellow(),
            e
        );
    }

    println!(
        "\n{} Applied {} file(s) from '{}'",
        "✓".green().bold(),
        state.file_count(),
        overlay_name
    );

    Ok(())
}

/// Apply multiple overlays atomically.
///
/// Pre-checks for conflicts between the selected overlays and with existing overlays,
/// then applies each overlay in sequence. If any overlay fails to apply, all previously
/// applied overlays from this batch are rolled back.
fn apply_multiple_overlays(
    sources: &[ResolvedSource],
    target: &Path,
    force_copy: bool,
    dry_run: bool,
    conflict_strategy: ConflictStrategy,
    merge: bool,
) -> Result<()> {
    let target = canonicalize_path(target, "Target directory")?;
    validate_git_repo(&target)?;

    println!(
        "\n{} Preparing to apply {} overlay(s)...",
        "Batch:".blue().bold(),
        sources.len()
    );

    // Phase 1: Check for conflicts between selected overlays
    check_overlay_conflicts(sources)?;

    // Phase 2: Check for conflicts with already-applied overlays
    let mut existing_targets = load_all_overlay_targets(&target)?;
    for resolved in sources {
        let config = load_overlay_config(&resolved.path)?;
        let overlay_name = determine_overlay_name(&config, &resolved.path, None)?;

        let overlay_state_path = target
            .join(STATE_DIR)
            .join(OVERLAYS_DIR)
            .join(format!("{overlay_name}.ccl"));

        if overlay_state_path.exists() {
            match conflict_strategy {
                ConflictStrategy::Force => {
                    println!(
                        "  {} Removing existing overlay '{}' (batch mode)",
                        "Force:".yellow(),
                        overlay_name
                    );
                    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
                    remove_single_overlay(&target, &overlays_dir, &overlay_name)?;
                    // Reload targets so subsequent conflict checks see fresh state
                    existing_targets = load_all_overlay_targets(&target)?;
                }
                ConflictStrategy::Fail | ConflictStrategy::SkipConflicts => {
                    bail!(
                        "Overlay '{overlay_name}' is already applied. \
                         Run 'repoverlay remove {overlay_name}' first, or use --force."
                    );
                }
            }
        }

        // Check files in this overlay against existing overlay targets.
        // Only run for Fail strategy — Force and SkipConflicts delegate to
        // apply_resolved_overlay which loads fresh targets and handles per-file decisions.
        if conflict_strategy == ConflictStrategy::Fail {
            check_files_against_existing(&resolved.path, &config, &existing_targets)?;
        }
    }

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        println!("\nWould apply {} overlay(s):", sources.len());
        for resolved in sources {
            println!("  - {}", resolved.path.display());
        }
        return Ok(());
    }

    // Phase 3: Apply each overlay, tracking for rollback
    let mut applied: Vec<String> = Vec::new();

    for resolved in sources {
        match apply_resolved_overlay(
            resolved,
            &target,
            force_copy,
            None,
            conflict_strategy,
            merge,
        ) {
            Ok(()) => {
                let config = load_overlay_config(&resolved.path)?;
                let name = determine_overlay_name(&config, &resolved.path, None)?;
                applied.push(name);
            }
            Err(e) => {
                // Rollback all previously applied overlays from this batch
                eprintln!(
                    "\n{} Failed to apply overlay from '{}': {e}",
                    "Error:".red().bold(),
                    resolved.path.display()
                );

                if !applied.is_empty() {
                    eprintln!(
                        "{} Rolling back {} previously applied overlay(s)...",
                        "Rollback:".yellow().bold(),
                        applied.len()
                    );

                    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
                    for name in &applied {
                        if let Err(rollback_err) =
                            remove_single_overlay(&target, &overlays_dir, name)
                        {
                            eprintln!(
                                "  {} Failed to rollback '{name}': {rollback_err}",
                                "Warning:".yellow()
                            );
                        }
                    }

                    // Clean up .repoverlay directory if no overlays remain
                    let remaining = list_applied_overlays(&target).unwrap_or_default();
                    if remaining.is_empty() {
                        let _ = fs::remove_dir_all(target.join(STATE_DIR));
                    }
                }

                bail!("Batch overlay application failed and was rolled back: {e}");
            }
        }
    }

    println!(
        "\n{} Successfully applied {} overlay(s)",
        "✓".green().bold(),
        applied.len()
    );

    Ok(())
}

/// Collect overlay file entries, applying filtering and path mapping.
///
/// Walks the overlay source directory and returns `(source_rel_path, mapped_target_path)` pairs
/// for each file that should be overlaid. Skips config files, `.git`, cache metadata, and files
/// within directories being symlinked as units.
fn collect_overlay_files(source: &Path, config: &OverlayConfig) -> Vec<(PathBuf, String)> {
    let dir_set: std::collections::HashSet<PathBuf> =
        config.directories.iter().map(PathBuf::from).collect();

    let mut files = Vec::new();

    for entry in WalkDir::new(source)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let Ok(rel_path) = entry.path().strip_prefix(source) else {
            continue;
        };

        let rel_str = rel_path.to_string_lossy();
        if rel_path == Path::new(CONFIG_FILE)
            || rel_str.starts_with(".git/")
            || rel_str.starts_with(".git\\")
            || rel_str == ".git"
            || rel_str == ".repoverlay-cache-meta.ccl"
        {
            continue;
        }

        if dir_set.iter().any(|dir| rel_path.starts_with(dir)) {
            continue;
        }

        let rel_string = rel_str.to_string();
        let target_rel = config
            .mappings
            .get(&rel_string)
            .map_or_else(|| rel_string.clone(), Clone::clone);

        files.push((rel_path.to_path_buf(), target_rel));
    }

    files
}

/// Check for file path conflicts across multiple overlay sources.
///
/// Walks each overlay's files and directories to build a map of target paths.
/// Returns an error if any target path would be claimed by more than one overlay.
fn check_overlay_conflicts(sources: &[ResolvedSource]) -> Result<()> {
    use std::collections::{HashMap, HashSet};

    let mut target_files: HashMap<String, String> = HashMap::new();
    let mut target_dirs: HashSet<String> = HashSet::new();

    for resolved in sources {
        let source = &resolved.path;
        let config = load_overlay_config(source)?;
        let overlay_name = determine_overlay_name(&config, source, None)?;

        // Check configured directories
        for dir_name in &config.directories {
            let dir_str = dir_name.clone();
            if let Some(conflicting) = target_files.get(&dir_str) {
                bail!(
                    "Conflict between selected overlays: directory '{dir_name}' is claimed by \
                     both '{conflicting}' and '{overlay_name}'.\n\
                     Cannot apply overlays with overlapping file paths."
                );
            }

            // Check if any existing file falls under this directory
            let dir_prefix = format!("{dir_str}/");
            for (existing_file, existing_owner) in &target_files {
                if existing_file.starts_with(&dir_prefix) {
                    bail!(
                        "Conflict between selected overlays: directory '{dir_name}' \
                         (from '{overlay_name}') would overlap with file '{existing_file}' \
                         (from '{existing_owner}').\n\
                         Cannot apply overlays with overlapping file paths."
                    );
                }
            }

            target_files.insert(dir_str.clone(), overlay_name.clone());
            target_dirs.insert(dir_str);
        }

        // Check individual files
        for (_rel_path, target_rel) in collect_overlay_files(source, &config) {
            if let Some(conflicting) = target_files.get(&target_rel) {
                bail!(
                    "Conflict between selected overlays: file '{target_rel}' is claimed by \
                     both '{conflicting}' and '{overlay_name}'.\n\
                     Cannot apply overlays with overlapping file paths."
                );
            }

            // Check if this file falls under a directory claimed by another overlay
            for dir in &target_dirs {
                let dir_prefix = format!("{dir}/");
                if target_rel.starts_with(&dir_prefix) {
                    let dir_owner = &target_files[dir];
                    if *dir_owner != overlay_name {
                        bail!(
                            "Conflict between selected overlays: file '{target_rel}' \
                             (from '{overlay_name}') falls within directory '{dir}' \
                             (from '{dir_owner}').\n\
                             Cannot apply overlays with overlapping file paths."
                        );
                    }
                }
            }

            target_files.insert(target_rel, overlay_name.clone());
        }
    }

    Ok(())
}

/// Load an overlay configuration from a source directory.
fn load_overlay_config(source: &Path) -> Result<OverlayConfig> {
    let config_path = source.join(CONFIG_FILE);
    if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
        sickle::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", config_path.display()))
    } else {
        Ok(OverlayConfig::default())
    }
}

/// Resolve the raw overlay display name from config and source path.
///
/// Priority: CLI override > config name > directory name.
fn resolve_overlay_display_name(
    config: &OverlayConfig,
    source: &Path,
    name_override: Option<String>,
) -> String {
    name_override
        .or_else(|| config.overlay.name.clone())
        .unwrap_or_else(|| {
            source.file_name().map_or_else(
                || "unnamed".to_string(),
                |n| n.to_string_lossy().to_string(),
            )
        })
}

/// Determine the normalized overlay name from config and source path.
fn determine_overlay_name(
    config: &OverlayConfig,
    source: &Path,
    name_override: Option<String>,
) -> Result<String> {
    let overlay_name = resolve_overlay_display_name(config, source, name_override);
    normalize_overlay_name(&overlay_name)
}

/// Check overlay files for conflicts with existing (already-applied) overlay targets.
fn check_files_against_existing(
    source: &Path,
    config: &OverlayConfig,
    existing_targets: &std::collections::HashMap<String, String>,
) -> Result<()> {
    // Check configured directories
    for dir_name in &config.directories {
        if let Some(conflicting) = existing_targets.get(dir_name.as_str()) {
            bail!(
                "Conflict: directory '{dir_name}' is already managed by overlay '{conflicting}'.\n\
                 Remove that overlay first."
            );
        }
    }

    // Check individual files
    for (_rel_path, target_rel) in collect_overlay_files(source, config) {
        if let Some(conflicting) = existing_targets.get(&target_rel) {
            bail!(
                "Conflict: file '{target_rel}' is already managed by overlay '{conflicting}'.\n\
                 Remove that overlay first."
            );
        }
    }

    Ok(())
}

/// Remove applied overlay(s) from a target repository.
///
/// # Workflow
///
/// 1. Load overlay state from `.repoverlay/overlays/<name>.ccl`
/// 2. Remove each file/symlink managed by the overlay
/// 3. Clean up empty parent directories
/// 4. Remove overlay section from `.git/info/exclude`
/// 5. Delete state file
/// 6. Remove external backup
/// 7. If no overlays remain, remove `.repoverlay/` directory
pub(crate) fn remove_overlay(
    target: &Path,
    name: Option<String>,
    remove_all: bool,
    dry_run: bool,
) -> Result<()> {
    debug!(
        "remove_overlay: target={}, name={:?}, remove_all={}, dry_run={}",
        target.display(),
        name,
        remove_all,
        dry_run
    );

    if dry_run {
        let target = canonicalize_path(target, "Target directory")?;
        let applied_overlays = list_applied_overlays(&target)?;

        if remove_all {
            println!("{} Dry run - would remove all overlays:", "Note:".yellow());
            for overlay_name in &applied_overlays {
                println!("  - {overlay_name}");
            }
        } else if let Some(ref name) = name {
            println!(
                "{} Dry run - would remove overlay '{}'",
                "Note:".yellow(),
                name
            );
        }
        return Ok(());
    }
    let target = canonicalize_path(target, "Target directory")?;
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    if remove_all {
        // Remove all overlays
        for overlay_name in &applied_overlays {
            remove_single_overlay(&target, &overlays_dir, overlay_name)?;
        }

        // Clean up .repoverlay directory entirely
        fs::remove_dir_all(target.join(STATE_DIR))?;

        println!("\n{} Removed all overlays", "✓".green().bold());
    } else if let Some(name) = name {
        let normalized_name = normalize_overlay_name(&name)?;
        remove_single_overlay(&target, &overlays_dir, &normalized_name)?;

        // Check if any overlays remain
        let remaining = list_applied_overlays(&target)?;
        if remaining.is_empty() {
            // No overlays left, clean up .repoverlay directory
            fs::remove_dir_all(target.join(STATE_DIR))?;
        }
    } else {
        // This path should not be reached from non-interactive contexts
        bail!("No overlay name specified. Use --all to remove all overlays, or specify a name.");
    }

    Ok(())
}

/// Remove a single overlay by name.
pub(crate) fn remove_single_overlay(target: &Path, overlays_dir: &Path, name: &str) -> Result<()> {
    debug!("remove_single_overlay: {name}");
    let state_file = overlays_dir.join(format!("{name}.ccl"));

    if !state_file.exists() {
        // List available overlays for helpful error message
        let available = list_applied_overlays(target)?;

        if available.is_empty() {
            bail!("No overlays are currently applied");
        }
        bail!(
            "Overlay '{}' not found. Available overlays: {}",
            name,
            available.join(", ")
        );
    }

    let state = load_overlay_state(target, name)?;

    println!("{} overlay: {}", "Removing".red().bold(), state.name);

    // Remove files and directories
    for entry in state.file_entries() {
        let file_path = target.join(&entry.target);
        trace!("removing: {}", file_path.display());

        if file_path.exists() || file_path.is_symlink() {
            match entry.entry_type {
                EntryType::Directory => {
                    // For directory entries, check if it's a symlink or a real directory
                    if file_path.is_symlink() {
                        // Remove symlink (use remove_file on Unix, remove_dir on Windows for dir symlinks)
                        #[cfg(unix)]
                        fs::remove_file(&file_path).with_context(|| {
                            format!(
                                "Failed to remove directory symlink: {}",
                                file_path.display()
                            )
                        })?;
                        #[cfg(windows)]
                        fs::remove_dir(&file_path).with_context(|| {
                            format!(
                                "Failed to remove directory symlink: {}",
                                file_path.display()
                            )
                        })?;
                    } else {
                        // It's a copied directory, remove recursively
                        fs::remove_dir_all(&file_path).with_context(|| {
                            format!("Failed to remove directory: {}", file_path.display())
                        })?;
                    }
                    println!("  {} {}/", "-".red(), entry.target.display());
                }
                EntryType::File => {
                    fs::remove_file(&file_path)
                        .with_context(|| format!("Failed to remove: {}", file_path.display()))?;
                    println!("  {} {}", "-".red(), entry.target.display());
                }
            }

            // Remove empty parent directories (but not the target itself)
            let mut parent = file_path.parent();
            while let Some(dir) = parent {
                if dir == target {
                    break;
                }
                if dir
                    .read_dir()
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
                {
                    fs::remove_dir(dir).ok();
                    parent = dir.parent();
                } else {
                    break;
                }
            }
        }
    }

    // Update git exclude (remove this overlay's section)
    let exclude_entries: Vec<String> = state
        .file_entries()
        .iter()
        .map(|e| {
            let path = e.target.to_string_lossy().replace('\\', "/");
            // Add trailing slash for directories in git exclude
            match e.entry_type {
                EntryType::Directory => format!("{path}/"),
                EntryType::File => path,
            }
        })
        .collect();
    update_git_exclude(target, name, &exclude_entries, false)?;

    // Remove state file
    fs::remove_file(&state_file)?;

    // Remove external backup
    if let Err(e) = remove_external_state(target, name) {
        eprintln!(
            "  {} Could not remove external backup: {}",
            "Warning:".yellow(),
            e
        );
    }

    println!(
        "\n{} Removed {} file(s) from '{}'",
        "✓".green().bold(),
        state.file_count(),
        state.name
    );

    Ok(())
}

/// Show the status of applied overlays.
pub(crate) fn show_status(target: &Path, filter_name: Option<String>) -> Result<()> {
    let target = canonicalize_path(target, "Target directory")?;

    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        println!("{} No overlays are currently applied.", "Status:".bold());
        return Ok(());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        println!("{} No overlays are currently applied.", "Status:".bold());
        return Ok(());
    }

    // If filtering by name, show just that overlay
    if let Some(filter) = filter_name {
        let normalized = normalize_overlay_name(&filter)?;

        if !applied_overlays.contains(&normalized) {
            bail!(
                "Overlay '{}' is not applied. Available: {}",
                filter,
                applied_overlays.join(", ")
            );
        }

        show_single_overlay_status(&target, &normalized)?;
        return Ok(());
    }

    // Show summary header
    println!(
        "{} ({} overlay(s) applied)",
        "Overlay Status".bold(),
        applied_overlays.len()
    );
    println!();

    for overlay_name in &applied_overlays {
        show_single_overlay_status(&target, overlay_name)?;
        println!();
    }

    Ok(())
}

/// Show status for a single overlay.
pub(crate) fn show_single_overlay_status(target: &Path, name: &str) -> Result<()> {
    let state = load_overlay_state(target, name)?;

    println!("  {} {}", "Overlay:".bold(), state.name.cyan());

    // Display source based on type
    match &state.source {
        OverlaySource::Local { path } => {
            println!("    Source:  {}", path.display());
        }
        OverlaySource::GitHub {
            url,
            git_ref,
            commit,
            subpath,
            ..
        } => {
            println!("    Source:  {} {}", url, "(GitHub)".dimmed());
            println!("    Ref:     {git_ref}");
            let short_commit = &commit[..12.min(commit.len())];
            println!("    Commit:  {short_commit}");
            if let Some(sp) = subpath {
                println!("    Subpath: {sp}");
            }
        }
        OverlaySource::OverlayRepo {
            org,
            repo,
            name: overlay_name,
            commit,
            resolved_via,
            source_name,
        } => {
            let via_upstream = matches!(resolved_via, Some(state::ResolvedVia::Upstream));
            let via_str = if via_upstream {
                format!(" {}", "(via upstream)".yellow())
            } else {
                String::new()
            };
            println!("    Source:  {org}/{repo}/{overlay_name}{via_str}");
            let short_commit = &commit[..12.min(commit.len())];
            println!("    Commit:  {short_commit}");
            if let Some(source) = source_name {
                println!("    From:    {}", source.cyan());
            }
        }
    }

    println!(
        "    Applied: {}",
        state.applied_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("    Files:   {}", state.file_count());

    for entry in state.file_entries() {
        let target_path = target.join(&entry.target);
        let status = if target_path.exists() || target_path.is_symlink() {
            "✓".green()
        } else {
            "✗".red()
        };

        let type_str = match entry.link_type {
            LinkType::Symlink => "symlink",
            LinkType::Copy => "copy",
            LinkType::Merged => "merged",
        };

        // Add trailing slash and [dir] marker for directories
        let (path_display, dir_marker) = match entry.entry_type {
            EntryType::Directory => (format!("{}/", entry.target.display()), " [dir]"),
            EntryType::File => (entry.target.display().to_string(), ""),
        };

        println!(
            "      {} {}{} ({})",
            status,
            path_display,
            dir_marker.magenta(),
            type_str.dimmed()
        );
    }

    Ok(())
}

/// Restore overlays after git clean or other removal.
///
/// Uses external state backup (`~/.local/share/repoverlay/applied/`) to recover
/// overlays that were removed by `git clean -fdx` or similar operations.
///
/// # Workflow
///
/// 1. Load external state backup for the target repository
/// 2. For each saved overlay state, re-apply using original source
pub(crate) fn restore_overlays(
    target: &Path,
    dry_run: bool,
    conflict_strategy: ConflictStrategy,
    merge: bool,
) -> Result<()> {
    debug!(
        "restore_overlays: target={}, dry_run={}, conflict_strategy={:?}",
        target.display(),
        dry_run,
        conflict_strategy
    );
    let target = canonicalize_path(target, "Target directory")?;
    validate_git_repo(&target)?;

    // Load external state
    let external_states = load_external_states(&target)?;
    debug!("found {} external states to restore", external_states.len());

    if external_states.is_empty() {
        println!("{} No overlays to restore.", "Status:".bold());
        println!("  No external backup found for this repository.");
        return Ok(());
    }

    println!(
        "{} {} overlay(s) to restore:",
        "Found".blue().bold(),
        external_states.len()
    );

    for state in &external_states {
        println!("  - {}", state.name);
        match &state.source {
            OverlaySource::Local { path } => {
                println!("    Source: {}", path.display());
            }
            OverlaySource::GitHub { url, git_ref, .. } => {
                println!("    Source: {url} ({git_ref})");
            }
            OverlaySource::OverlayRepo {
                org,
                repo,
                name: overlay_name,
                ..
            } => {
                println!("    Source: {org}/{repo}/{overlay_name} (overlay repo)");
            }
        }
    }

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        return Ok(());
    }

    println!();

    // Restore each overlay
    for state in external_states {
        let source_str = match &state.source {
            OverlaySource::Local { path } => path.to_string_lossy().to_string(),
            OverlaySource::GitHub { url, .. } => url.clone(),
            OverlaySource::OverlayRepo {
                org,
                repo,
                name: overlay_name,
                ..
            } => {
                format!("{org}/{repo}/{overlay_name}")
            }
        };

        let ref_override = match &state.source {
            OverlaySource::GitHub { git_ref, .. } => Some(git_ref.as_str()),
            OverlaySource::Local { .. } | OverlaySource::OverlayRepo { .. } => None,
        };

        // Re-apply the overlay
        match apply_overlay(
            &source_str,
            &target,
            false, // Use symlinks by default
            Some(state.name.clone()),
            ref_override,
            true, // Update cache
            conflict_strategy,
            merge,
            None,  // Use default source resolution for restore
            false, // Not a dry run
        ) {
            Ok(()) => {}
            Err(e) => {
                eprintln!(
                    "  {} Failed to restore '{}': {}",
                    "Error:".red(),
                    state.name,
                    e
                );
            }
        }
    }

    Ok(())
}

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
    let overlays_to_check: Vec<_> = if let Some(ref name) = name {
        let normalized = normalize_overlay_name(name)?;
        if !applied_overlays.contains(&normalized) {
            bail!(
                "Overlay '{}' is not applied. Available: {}",
                name,
                applied_overlays.join(", ")
            );
        }
        vec![normalized]
    } else {
        applied_overlays
    };

    let cache = CacheManager::new()?;
    let mut updates_available = Vec::new();

    // Check for updates
    for overlay_name in &overlays_to_check {
        let state = load_overlay_state(&target, overlay_name)?;

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
        } else {
            println!(
                "  {} {} is a local overlay (not updatable)",
                "-".dimmed(),
                state.name
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
        println!("    {}  →  {}", &old_commit[..7], &new_commit[..7]);
        println!("    {}", url.dimmed());
    }

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        return Ok(());
    }

    println!();

    // Apply updates
    for (normalized_name, _, _, _, _) in &updates_available {
        let state = load_overlay_state(&target, normalized_name)?;

        if let OverlaySource::GitHub { url, git_ref, .. } = &state.source {
            // Remove old overlay
            let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
            remove_single_overlay(&target, &overlays_dir, normalized_name)?;

            // Re-apply with update
            apply_overlay(
                url,
                &target,
                false,
                Some(state.name.clone()),
                Some(git_ref.as_str()),
                true,
                conflict_strategy,
                merge,
                None,  // Use default source resolution for update
                false, // Not a dry run
            )?;
        }
    }

    Ok(())
}

/// Detect org/repo from git remote origin.
///
/// Returns `None` if the remote cannot be detected (e.g., no remote, non-GitHub).
fn detect_target_from_git_remote(repo_path: &Path) -> Option<(String, String)> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8(output.stdout).ok()?.trim().to_string();
    parse_github_owner_repo(&url).ok()
}

/// Create a new overlay from files in a repository.
///
/// # Modes
///
/// - **Discovery mode** (no `--include`): Scans repository for candidate files
///   (AI configs, gitignored, untracked) and presents interactive selection
/// - **Explicit mode** (`--include` flags): Copies specified files directly
///
/// # Output Directory Resolution
///
/// When `output` is `None`, the output directory is determined as follows:
/// 1. If an overlay source is configured (`source add` was run), the overlay is
///    created directly in the overlay repo at `<org>/<repo>/<name>/`, where
///    org/repo is detected from the source repository's git remote origin.
/// 2. If no overlay repo is configured (or git remote detection fails), falls
///    back to `~/.local/share/repoverlay/overlays/<repo-name>`.
///
/// # Workflow
///
/// 1. Validate source is a git repository
/// 2. If no includes specified, discover candidate files
/// 3. Interactive selection or use pre-selected AI configs (with `--yes`)
/// 4. Copy selected files to output directory
/// 5. Generate `repoverlay.ccl` config file
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn create_overlay(
    source: &Path,
    output: Option<PathBuf>,
    include: &[PathBuf],
    name: Option<String>,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    // Verify source is a git repository
    if !source.join(".git").exists() {
        bail!(
            "Source directory is not a git repository: {}",
            source.display()
        );
    }

    // Determine output directory
    // Priority: explicit --local > overlay repo (if configured) > local fallback
    // Also track overlay repo info for better prompts: (repo_root, org, repo, overlay_name)
    let (output_dir, overlay_repo_info): (PathBuf, Option<(PathBuf, String, String, String)>) =
        if let Some(p) = &output {
            (p.clone(), None)
        } else {
            // Check if overlay repo is configured
            let config = config::load_config(None).ok();
            let overlay_repo_config = config.as_ref().and_then(|c| c.overlay_repo.as_ref());

            if let Some(repo_config) = overlay_repo_config {
                // Try to detect org/repo from git remote
                if let Some((org, repo)) = detect_target_from_git_remote(source) {
                    // Determine overlay name
                    let overlay_name = name.clone().unwrap_or_else(|| {
                        source
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("overlay")
                            .to_string()
                    });

                    // Use overlay repo path: <repo_path>/<org>/<repo>/<name>
                    let manager = overlay_repo::OverlayRepoManager::new(repo_config.clone())
                        .expect("Failed to create overlay repo manager");
                    manager
                        .ensure_cloned()
                        .expect("Failed to ensure overlay repo is cloned");
                    let repo_root = manager.path().to_path_buf();
                    let full_path = repo_root.join(&org).join(&repo).join(&overlay_name);
                    (full_path, Some((repo_root, org, repo, overlay_name)))
                } else {
                    // Couldn't detect target, fall back to local
                    eprintln!(
                        "{} Could not detect target from git remote, using local storage.",
                        "Warning:".yellow()
                    );
                    let repo_name = source
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("overlay");
                    let proj_dirs = directories::ProjectDirs::from("", "", "repoverlay")
                        .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;
                    (proj_dirs.data_dir().join("overlays").join(repo_name), None)
                }
            } else {
                // No overlay repo configured, use local fallback
                let repo_name = source
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("overlay");
                let proj_dirs = directories::ProjectDirs::from("", "", "repoverlay")
                    .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;
                (proj_dirs.data_dir().join("overlays").join(repo_name), None)
            }
        };

    // If no includes specified, run discovery mode
    if include.is_empty() {
        // Discover files in the repository
        print!(
            "{} Scanning for overlay candidates...",
            "Discovery:".cyan().bold()
        );
        std::io::Write::flush(&mut std::io::stdout())?;

        let discovered = detection::discover_files(source);

        // Show discovery summary
        let ai_count = discovered
            .iter()
            .filter(|f| f.category == detection::FileCategory::AiConfig)
            .count();
        let gi_count = discovered
            .iter()
            .filter(|f| f.category == detection::FileCategory::Gitignored)
            .count();
        let ut_count = discovered
            .iter()
            .filter(|f| f.category == detection::FileCategory::Untracked)
            .count();
        println!(
            " found {} AI, {} gitignored, {} untracked",
            selection::humanize_count(ai_count).green(),
            selection::humanize_count(gi_count).yellow(),
            selection::humanize_count(ut_count).blue()
        );

        if discovered.is_empty() {
            bail!(
                "No files discovered and none specified.\n\n\
                 Use --include to specify files to include in the overlay.\n\
                 Example:\n  repoverlay create my-overlay --include .claude/ --include CLAUDE.md"
            );
        }

        // In dry-run mode without includes, show discovered files
        if dry_run {
            println!(
                "{} Discovered files in: {}",
                "Discovery:".cyan().bold(),
                source.display()
            );
            println!();

            let groups = detection::group_by_category(&discovered);
            for (category, files) in groups {
                let category_name = match category {
                    detection::FileCategory::AiConfig => "AI Configurations".green(),
                    detection::FileCategory::AiConfigDirectory => "AI Config Directories".magenta(),
                    detection::FileCategory::Gitignored => "Gitignored".yellow(),
                    detection::FileCategory::Untracked => "Untracked".blue(),
                };
                let preselected_note = if files.iter().any(|f| f.preselected) {
                    " (pre-selected)"
                } else {
                    ""
                };
                println!("{}{}:", category_name.bold(), preselected_note.dimmed());
                for file in files {
                    let marker = if file.preselected { "[x]" } else { "[ ]" };
                    println!("  {} {}", marker, file.path.display());
                }
                println!();
            }

            println!(
                "{}",
                "Use --include to specify which files to include:".dimmed()
            );
            // Suggest command based on discovered AI configs
            let ai_configs: Vec<_> = discovered
                .iter()
                .filter(|f| f.category == detection::FileCategory::AiConfig)
                .collect();
            if !ai_configs.is_empty() {
                let includes: Vec<_> = ai_configs
                    .iter()
                    .map(|f| format!("--include {}", f.path.display()))
                    .collect();
                println!("  repoverlay create my-overlay {}", includes.join(" "));
            }
            return Ok(());
        }

        // Interactive mode: let user select files
        if !yes {
            use selection::{SelectionConfig, select_files};

            let config = SelectionConfig::default();
            let result = select_files(&discovered, config)?;

            if result.cancelled {
                bail!("Selection cancelled.");
            }

            if result.selected_files.is_empty() {
                bail!("No files selected. Aborting.");
            }

            // Get output directory from user if not specified
            let final_output = if output.is_none() {
                use dialoguer::Input;

                if let Some((repo_root, org, repo, default_name)) = &overlay_repo_info {
                    // Show overlay repo context
                    println!("{} {}/{}", "Target:".bold(), org.cyan(), repo.cyan());

                    let overlay_name: String = Input::new()
                        .with_prompt("Overlay name")
                        .default(default_name.clone())
                        .interact_text()?;

                    repo_root.join(org).join(repo).join(overlay_name)
                } else {
                    // Local storage - show full path
                    println!(
                        "Where should the overlay be created?\n\
                         (This directory will contain the overlay files and config)"
                    );

                    let path_str: String = Input::new()
                        .with_prompt("Overlay directory")
                        .default(output_dir.display().to_string())
                        .interact_text()?;

                    PathBuf::from(path_str)
                }
            } else {
                output_dir
            };

            // Now create the overlay with selected files
            return create_overlay_with_files(source, &final_output, &result.selected_files, name);
        }

        // With --yes flag but no includes, use pre-selected files (AI configs)
        let preselected: Vec<PathBuf> = discovered
            .iter()
            .filter(|f| f.preselected)
            .map(|f| f.path.clone())
            .collect();

        if preselected.is_empty() {
            bail!(
                "No files specified and no AI configs found to auto-select.\n\n\
                 Use --include to specify files:\n  repoverlay create my-overlay --include .envrc"
            );
        }

        println!(
            "{} Using {} pre-selected AI config file(s)",
            "Auto-select:".cyan().bold(),
            preselected.len()
        );

        return create_overlay_with_files(source, &output_dir, &preselected, name);
    }

    // Validate all include paths exist
    for path in include {
        let full_path = source.join(path);
        if !full_path.exists() {
            bail!("Include path does not exist: {}", path.display());
        }
    }

    if dry_run {
        println!(
            "{} Would create overlay at: {}",
            "Dry run:".yellow().bold(),
            output_dir.display()
        );
        println!();
        println!("Files to include:");
        for path in include {
            let full_path = source.join(path);
            if full_path.is_dir() {
                for entry in walkdir::WalkDir::new(&full_path)
                    .into_iter()
                    .filter_map(std::result::Result::ok)
                    .filter(|e| e.file_type().is_file())
                {
                    let rel = entry
                        .path()
                        .strip_prefix(source)
                        .unwrap_or_else(|_| entry.path());
                    println!("  + {}", rel.display());
                }
            } else {
                println!("  + {}", path.display());
            }
        }
        return Ok(());
    }

    // Use shared helper to copy files and generate config
    create_overlay_with_files(source, &output_dir, include, name)
}

/// Copy files from source to output directory.
pub(crate) fn copy_files_to_overlay(
    source: &Path,
    output_dir: &Path,
    include: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)?;

    let mut copied_files = Vec::new();
    for path in include {
        let src_path = source.join(path);
        if src_path.is_dir() {
            for entry in walkdir::WalkDir::new(&src_path)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                let rel_path = entry.path().strip_prefix(source)?;
                let dest_path = output_dir.join(rel_path);
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(entry.path(), &dest_path)?;
                copied_files.push(rel_path.to_path_buf());
            }
        } else {
            let dest_path = output_dir.join(path);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src_path, &dest_path)?;
            copied_files.push(path.clone());
        }
    }

    Ok(copied_files)
}

/// Generate overlay config file content.
pub(crate) fn generate_overlay_config(name: &str) -> String {
    format!(
        r"/= Overlay configuration file.
/= This file describes an overlay and how it should be applied.

overlay =
  /= name: Display name for this overlay.
  /= Used in status output and when listing overlays.
  name = {name}

/= mappings (optional): Remap file paths when applying the overlay.
/= Keys are source paths (in the overlay), values are target paths (in the repo).
/= Use this to rename files or place them in different locations.
/= mappings =
/=   .envrc.template = .envrc
"
    )
}

/// Print overlay creation success message.
pub(crate) fn print_overlay_created(output_dir: &Path, copied_files: &[PathBuf]) {
    println!(
        "{} overlay at: {}",
        "Created".green().bold(),
        output_dir.display()
    );
    println!();
    println!("Files included:");
    for file in copied_files {
        println!("  + {}", file.display());
    }
    println!();
    println!(
        "Apply with: {} {} {}",
        "repoverlay apply".cyan(),
        output_dir.display(),
        "--target <repo>".dimmed()
    );
}

/// Helper to create overlay with specified files.
pub(crate) fn create_overlay_with_files(
    source: &Path,
    output_dir: &Path,
    include: &[PathBuf],
    name: Option<String>,
) -> Result<()> {
    let copied_files = copy_files_to_overlay(source, output_dir, include)?;

    let overlay_name = name.unwrap_or_else(|| {
        output_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("overlay")
            .to_string()
    });

    fs::write(
        output_dir.join("repoverlay.ccl"),
        generate_overlay_config(&overlay_name),
    )?;
    print_overlay_created(output_dir, &copied_files);

    Ok(())
}

/// Switch to a different overlay by removing all existing overlays first.
///
/// Atomic replacement of all overlays - useful for switching between different
/// configurations (e.g., different AI agent setups).
///
/// # Workflow
///
/// 1. Remove all existing overlays (if any)
/// 2. Apply the new overlay
pub(crate) fn switch_overlay(
    source: &str,
    target: &Path,
    copy: bool,
    name: Option<String>,
    ref_override: Option<&str>,
    conflict_strategy: ConflictStrategy,
    merge: bool,
) -> Result<()> {
    validate_git_repo(target)?;

    // Check if any overlays are currently applied
    let state_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    let has_overlays = state_dir.exists() && fs::read_dir(&state_dir)?.next().is_some();

    if has_overlays {
        println!("{} existing overlays...", "Removing".yellow().bold());
        // Remove all existing overlays
        remove_overlay(target, None, true, false)?;
    }

    // Apply the new overlay
    println!("{} new overlay...", "Applying".blue().bold());
    apply_overlay(
        source,
        target,
        copy,
        name,
        ref_override,
        false,
        conflict_strategy,
        merge,
        None,
        false,
    )?;

    Ok(())
}

/// Update .git/info/exclude file.
pub(crate) fn update_git_exclude(
    target: &Path,
    overlay_name: &str,
    entries: &[String],
    add: bool,
) -> Result<()> {
    debug!(
        "update_git_exclude: overlay={}, add={}, entries={}",
        overlay_name,
        add,
        entries.len()
    );

    // Resolve the actual git directory (handles worktrees where .git is a file)
    let git_dir = resolve_git_dir(target)?;
    let exclude_path = git_dir.join("info").join("exclude");

    // Ensure the info directory exists
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = fs::read_to_string(&exclude_path).unwrap_or_default();

    // Remove existing section for this overlay
    content = remove_overlay_section(&content, overlay_name);

    if add {
        // Add new section for this overlay
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&exclude_marker_start(overlay_name));
        content.push('\n');
        for entry in entries {
            content.push_str(entry);
            content.push('\n');
        }
        content.push_str(&exclude_marker_end(overlay_name));
        content.push('\n');

        // Ensure managed section exists (for .repoverlay itself)
        if !content.contains(&exclude_marker_start(MANAGED_SECTION_NAME)) {
            content.push_str(&exclude_marker_start(MANAGED_SECTION_NAME));
            content.push('\n');
            content.push_str(STATE_DIR);
            content.push('\n');
            content.push_str(&exclude_marker_end(MANAGED_SECTION_NAME));
            content.push('\n');
        }
    } else {
        // Check if any overlay sections remain (excluding managed)
        if !any_overlay_sections_remain(&content) {
            // Remove the managed section too
            content = remove_overlay_section(&content, MANAGED_SECTION_NAME);
        }
    }

    // Clean up excessive newlines
    while content.ends_with("\n\n") {
        content.pop();
    }

    fs::write(&exclude_path, content)?;
    Ok(())
}

/// Remove an overlay section from git exclude content.
pub(crate) fn remove_overlay_section(content: &str, name: &str) -> String {
    let start_marker = exclude_marker_start(name);
    let end_marker = exclude_marker_end(name);

    let mut result = String::new();
    let mut in_section = false;

    for line in content.lines() {
        if line.trim() == start_marker {
            in_section = true;
            continue;
        }
        if line.trim() == end_marker {
            in_section = false;
            continue;
        }
        if !in_section {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Remove trailing newlines
    while result.ends_with("\n\n") {
        result.pop();
    }

    result
}

/// Check if any overlay sections remain in git exclude content.
pub(crate) fn any_overlay_sections_remain(content: &str) -> bool {
    // Check for any repoverlay sections except "managed"
    for line in content.lines() {
        if line.starts_with("# repoverlay:")
            && line.ends_with(" start")
            && !line.contains(MANAGED_SECTION_NAME)
        {
            return true;
        }
    }
    false
}

/// Parse owner/repo from a GitHub URL (HTTPS or SSH format).
pub(crate) fn parse_github_owner_repo(url: &str) -> Result<(String, String)> {
    github::parse_remote_url(url).ok_or_else(|| {
        if url.contains("github.com") {
            anyhow::anyhow!("Could not parse git remote URL: {url}")
        } else {
            anyhow::anyhow!(
                "Could not detect target repository from git remote.\n\
                 Non-GitHub remotes are not supported for auto-detection.\n\
                 Please specify --target org/repo"
            )
        }
    })
}

/// Attempt to deep merge a JSON overlay file into an existing target file.
///
/// Returns `Some((file_entry, exclude_path))` on success, or `None` if the merge
/// failed (with a warning printed to stderr). The caller should add the entry to
/// state and the exclude path to the exclusion list when `Some` is returned.
fn try_merge_json(
    target_file: &Path,
    source_file: &Path,
    target_rel: &Path,
    rel_path: &Path,
) -> Option<(FileEntry, String)> {
    match merge_json_files(target_file, source_file, target_file) {
        Ok(result) => {
            log_merge_result(target_rel, &result);
            let entry = FileEntry {
                source: rel_path.to_path_buf(),
                target: target_rel.to_path_buf(),
                link_type: LinkType::Merged,
                entry_type: EntryType::File,
            };
            let exclude_path = target_rel.to_string_lossy().replace('\\', "/");
            Some((entry, exclude_path))
        }
        Err(e) => {
            eprintln!(
                "  {} JSON merge failed for '{}': {e}",
                "Warning:".yellow(),
                target_rel.display()
            );
            None
        }
    }
}

/// Log detailed merge results for a JSON file.
fn log_merge_result(path: &Path, result: &json_merge::MergeResult) {
    println!(
        "  {} {} ({} added, {} overridden, {} type {})",
        "~".cyan(),
        path.display(),
        result.keys_added,
        result.keys_overridden,
        result.type_mismatches.len(),
        if result.type_mismatches.len() == 1 {
            "mismatch"
        } else {
            "mismatches"
        }
    );

    for mismatch in &result.type_mismatches {
        eprintln!(
            "    {} Type mismatch at '{}': {} -> {} (overlay wins)",
            "Warning:".yellow(),
            mismatch.key_path,
            mismatch.base_type,
            mismatch.overlay_type
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    // Helper to create a test git repository
    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to init git repo");
        dir
    }

    // Tests for parse_github_owner_repo
    mod parse_github_owner_repo_tests {
        use super::*;

        #[test]
        fn parses_https_url() {
            let result = parse_github_owner_repo("https://github.com/owner/repo").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn parses_https_url_with_git_suffix() {
            let result = parse_github_owner_repo("https://github.com/owner/repo.git").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn parses_ssh_url() {
            let result = parse_github_owner_repo("git@github.com:owner/repo").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn parses_ssh_url_with_git_suffix() {
            let result = parse_github_owner_repo("git@github.com:owner/repo.git").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn parses_http_url() {
            let result = parse_github_owner_repo("http://github.com/owner/repo").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn handles_url_with_extra_path() {
            let result =
                parse_github_owner_repo("https://github.com/owner/repo/tree/main/path").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn fails_on_non_github_url() {
            let result = parse_github_owner_repo("https://gitlab.com/owner/repo");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Non-GitHub"));
        }

        #[test]
        fn fails_on_empty_owner() {
            let result = parse_github_owner_repo("https://github.com//repo");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_empty_repo() {
            let result = parse_github_owner_repo("https://github.com/owner/");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_malformed_url() {
            let result = parse_github_owner_repo("https://github.com/onlyowner");
            assert!(result.is_err());
        }
    }

    // Tests for any_overlay_sections_remain
    mod any_overlay_sections_remain_tests {
        use super::*;

        #[test]
        fn returns_false_for_empty_content() {
            assert!(!any_overlay_sections_remain(""));
        }

        #[test]
        fn returns_false_for_no_sections() {
            let content = "*.log\n.DS_Store\n";
            assert!(!any_overlay_sections_remain(content));
        }

        #[test]
        fn returns_false_for_only_managed_section() {
            let content = "# repoverlay:managed start\n.repoverlay\n# repoverlay:managed end\n";
            assert!(!any_overlay_sections_remain(content));
        }

        #[test]
        fn returns_true_for_overlay_section() {
            let content = "# repoverlay:my-overlay start\n.envrc\n# repoverlay:my-overlay end\n";
            assert!(any_overlay_sections_remain(content));
        }

        #[test]
        fn returns_true_for_overlay_plus_managed_section() {
            let content = "# repoverlay:my-overlay start\n.envrc\n# repoverlay:my-overlay end\n\
                           # repoverlay:managed start\n.repoverlay\n# repoverlay:managed end\n";
            assert!(any_overlay_sections_remain(content));
        }

        #[test]
        fn returns_true_for_multiple_overlay_sections() {
            let content = "# repoverlay:overlay-a start\n.envrc\n# repoverlay:overlay-a end\n\
                           # repoverlay:overlay-b start\n.env\n# repoverlay:overlay-b end\n";
            assert!(any_overlay_sections_remain(content));
        }

        #[test]
        fn ignores_partial_markers() {
            // Line that starts with "# repoverlay:" but doesn't end with " start"
            let content = "# repoverlay:something else\n";
            assert!(!any_overlay_sections_remain(content));
        }
    }

    // Tests for update_git_exclude
    mod update_git_exclude_tests {
        use super::*;

        #[test]
        fn creates_exclude_file_if_missing() {
            let repo = create_test_repo();
            let entries = vec![".envrc".to_string()];

            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            assert!(exclude_path.exists());

            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains("# repoverlay:test-overlay start"));
            assert!(content.contains(".envrc"));
            assert!(content.contains("# repoverlay:test-overlay end"));
        }

        #[test]
        fn appends_to_existing_exclude_file() {
            let repo = create_test_repo();

            // Create existing exclude content
            let exclude_path = repo.path().join(".git/info/exclude");
            fs::create_dir_all(exclude_path.parent().unwrap()).unwrap();
            fs::write(&exclude_path, "*.log\n").unwrap();

            let entries = vec![".envrc".to_string()];
            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains("*.log"));
            assert!(content.contains("# repoverlay:test-overlay start"));
        }

        #[test]
        fn removes_section_when_add_is_false() {
            let repo = create_test_repo();

            // First add a section
            let entries = vec![".envrc".to_string()];
            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            // Then remove it
            update_git_exclude(repo.path(), "test-overlay", &entries, false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(!content.contains("# repoverlay:test-overlay"));
        }

        #[test]
        fn adds_managed_section_with_first_overlay() {
            let repo = create_test_repo();
            let entries = vec![".envrc".to_string()];

            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains("# repoverlay:managed start"));
            assert!(content.contains(".repoverlay"));
        }

        #[test]
        fn removes_managed_section_when_last_overlay_removed() {
            let repo = create_test_repo();

            // Add an overlay
            let entries = vec![".envrc".to_string()];
            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            // Remove it
            update_git_exclude(repo.path(), "test-overlay", &entries, false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(!content.contains("# repoverlay:managed"));
        }

        #[test]
        fn writes_to_correct_location_in_worktree() {
            let temp = TempDir::new().unwrap();
            let worktree_path = temp.path();

            // Simulate a worktree: .git is a file pointing to the actual git dir
            let actual_git_dir = temp.path().join("actual-git-dir");
            fs::create_dir_all(&actual_git_dir).unwrap();

            let git_file_content = format!("gitdir: {}\n", actual_git_dir.display());
            fs::write(worktree_path.join(".git"), git_file_content).unwrap();

            let entries = vec![".envrc".to_string()];
            update_git_exclude(worktree_path, "test-overlay", &entries, true).unwrap();

            // Exclude file should be in the actual git dir, not worktree_path/.git/info/exclude
            let exclude_path = actual_git_dir.join("info").join("exclude");
            assert!(
                exclude_path.exists(),
                "exclude file should exist in actual git dir"
            );

            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains("# repoverlay:test-overlay start"));
            assert!(content.contains(".envrc"));

            // Verify it was NOT written to the worktree's .git path
            let wrong_path = worktree_path.join(".git").join("info").join("exclude");
            assert!(
                !wrong_path.exists(),
                "exclude file should not be at worktree .git path"
            );
        }
    }

    // Tests for validate_git_repo
    mod validate_git_repo_tests {
        use super::*;

        #[test]
        fn succeeds_on_git_repo() {
            let repo = create_test_repo();
            assert!(validate_git_repo(repo.path()).is_ok());
        }

        #[test]
        fn fails_on_non_git_directory() {
            let dir = TempDir::new().unwrap();
            let result = validate_git_repo(dir.path());
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not a git repository")
            );
        }
    }

    // Tests for canonicalize_path
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

    // Tests for copy_files_to_overlay
    mod copy_files_to_overlay_tests {
        use super::*;

        #[test]
        fn copies_single_file() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join("file.txt"), "content").unwrap();

            let copied =
                copy_files_to_overlay(source.path(), output.path(), &[PathBuf::from("file.txt")])
                    .unwrap();

            assert_eq!(copied.len(), 1);
            assert!(output.path().join("file.txt").exists());
        }

        #[test]
        fn copies_directory_recursively() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            fs::create_dir_all(source.path().join("dir/subdir")).unwrap();
            fs::write(source.path().join("dir/file1.txt"), "content1").unwrap();
            fs::write(source.path().join("dir/subdir/file2.txt"), "content2").unwrap();

            let copied =
                copy_files_to_overlay(source.path(), output.path(), &[PathBuf::from("dir")])
                    .unwrap();

            assert_eq!(copied.len(), 2);
            assert!(output.path().join("dir/file1.txt").exists());
            assert!(output.path().join("dir/subdir/file2.txt").exists());
        }

        #[test]
        fn creates_parent_directories() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            fs::create_dir_all(source.path().join("deep/nested")).unwrap();
            fs::write(source.path().join("deep/nested/file.txt"), "content").unwrap();

            copy_files_to_overlay(
                source.path(),
                output.path(),
                &[PathBuf::from("deep/nested/file.txt")],
            )
            .unwrap();

            assert!(output.path().join("deep/nested/file.txt").exists());
        }
    }

    // Tests for generate_overlay_config
    mod generate_overlay_config_tests {
        use super::*;

        #[test]
        fn includes_overlay_name() {
            let config = generate_overlay_config("my-overlay");
            assert!(config.contains("name = my-overlay"));
        }

        #[test]
        fn includes_commented_mappings() {
            let config = generate_overlay_config("test");
            assert!(config.contains("/= mappings"));
        }

        #[test]
        fn generates_valid_ccl() {
            let config = generate_overlay_config("test-name");
            // Basic structure check
            assert!(config.contains("overlay ="));
        }
    }

    // Tests for remove_overlay_section (additional edge cases)
    mod remove_overlay_section_additional_tests {
        use super::*;

        #[test]
        fn handles_windows_line_endings() {
            let content = "*.log\r\n# repoverlay:test start\r\n.envrc\r\n# repoverlay:test end\r\n.DS_Store\r\n";
            let result = remove_overlay_section(content, "test");
            // Should still work even though line endings differ
            assert!(!result.contains("repoverlay:test"));
        }

        #[test]
        fn handles_whitespace_around_markers() {
            let content = "  # repoverlay:test start  \n.envrc\n  # repoverlay:test end  \n";
            let result = remove_overlay_section(content, "test");
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn preserves_content_before_and_after() {
            let content = "before\n# repoverlay:test start\n.envrc\n# repoverlay:test end\nafter\n";
            let result = remove_overlay_section(content, "test");
            assert!(result.contains("before"));
            assert!(result.contains("after"));
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn handles_empty_section() {
            let content = "# repoverlay:empty start\n# repoverlay:empty end\n";
            let result = remove_overlay_section(content, "empty");
            assert!(!result.contains("repoverlay:empty"));
        }

        #[test]
        fn removes_only_specified_overlay() {
            let content = "# repoverlay:a start\n.a\n# repoverlay:a end\n\
                          # repoverlay:b start\n.b\n# repoverlay:b end\n";
            let result = remove_overlay_section(content, "a");
            assert!(!result.contains(".a"));
            assert!(result.contains(".b"));
            assert!(result.contains("# repoverlay:b"));
        }

        #[test]
        fn handles_similar_named_overlays() {
            let content = "# repoverlay:test start\n.test\n# repoverlay:test end\n\
                          # repoverlay:test-extended start\n.extended\n# repoverlay:test-extended end\n";
            let result = remove_overlay_section(content, "test");
            assert!(!result.contains(".test\n"));
            assert!(result.contains(".extended"));
        }
    }

    // Tests for update_git_exclude with multiple overlays
    mod update_git_exclude_multiple_tests {
        use super::*;

        #[test]
        fn handles_multiple_overlays() {
            let repo = create_test_repo();

            // Add first overlay
            update_git_exclude(repo.path(), "overlay-a", &[".envrc".to_string()], true).unwrap();

            // Add second overlay
            update_git_exclude(repo.path(), "overlay-b", &[".env.local".to_string()], true)
                .unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(content.contains("# repoverlay:overlay-a start"));
            assert!(content.contains(".envrc"));
            assert!(content.contains("# repoverlay:overlay-b start"));
            assert!(content.contains(".env.local"));
        }

        #[test]
        fn keeps_managed_section_when_one_overlay_remains() {
            let repo = create_test_repo();

            // Add two overlays
            update_git_exclude(repo.path(), "overlay-a", &[".envrc".to_string()], true).unwrap();
            update_git_exclude(repo.path(), "overlay-b", &[".env".to_string()], true).unwrap();

            // Remove one overlay
            update_git_exclude(repo.path(), "overlay-a", &[".envrc".to_string()], false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            // Managed section should remain because overlay-b is still there
            assert!(content.contains("# repoverlay:managed start"));
            assert!(content.contains("# repoverlay:overlay-b start"));
            assert!(!content.contains("# repoverlay:overlay-a"));
        }

        #[test]
        fn updates_existing_overlay_section() {
            let repo = create_test_repo();

            // Add overlay with one file
            update_git_exclude(repo.path(), "test", &[".envrc".to_string()], true).unwrap();

            // "Update" same overlay with different files (add=true replaces)
            update_git_exclude(
                repo.path(),
                "test",
                &[".env".to_string(), ".env.local".to_string()],
                true,
            )
            .unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            // Should have new entries, old should be gone
            assert!(content.contains(".env"));
            assert!(content.contains(".env.local"));
            // Should only have one test section
            assert_eq!(content.matches("# repoverlay:test start").count(), 1);
        }

        #[test]
        fn handles_multiple_entries_per_overlay() {
            let repo = create_test_repo();

            update_git_exclude(
                repo.path(),
                "test",
                &[
                    ".envrc".to_string(),
                    ".env.local".to_string(),
                    ".vscode/settings.json".to_string(),
                ],
                true,
            )
            .unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(content.contains(".envrc"));
            assert!(content.contains(".env.local"));
            assert!(content.contains(".vscode/settings.json"));
        }
    }

    // Tests for copy_files_to_overlay additional cases
    mod copy_files_to_overlay_additional_tests {
        use super::*;

        #[test]
        fn copies_multiple_files() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join("a.txt"), "a").unwrap();
            fs::write(source.path().join("b.txt"), "b").unwrap();
            fs::write(source.path().join("c.txt"), "c").unwrap();

            let copied = copy_files_to_overlay(
                source.path(),
                output.path(),
                &[
                    PathBuf::from("a.txt"),
                    PathBuf::from("b.txt"),
                    PathBuf::from("c.txt"),
                ],
            )
            .unwrap();

            assert_eq!(copied.len(), 3);
            assert_eq!(
                fs::read_to_string(output.path().join("a.txt")).unwrap(),
                "a"
            );
            assert_eq!(
                fs::read_to_string(output.path().join("b.txt")).unwrap(),
                "b"
            );
            assert_eq!(
                fs::read_to_string(output.path().join("c.txt")).unwrap(),
                "c"
            );
        }

        #[test]
        fn creates_output_dir_if_missing() {
            let source = TempDir::new().unwrap();
            let temp = TempDir::new().unwrap();
            let output = temp.path().join("nested/output/dir");

            fs::write(source.path().join("file.txt"), "content").unwrap();

            let copied =
                copy_files_to_overlay(source.path(), &output, &[PathBuf::from("file.txt")])
                    .unwrap();

            assert_eq!(copied.len(), 1);
            assert!(output.join("file.txt").exists());
        }

        #[test]
        fn preserves_file_content() {
            let source = TempDir::new().unwrap();
            let output = TempDir::new().unwrap();

            let content = "line1\nline2\nline3\n特殊字符\n";
            fs::write(source.path().join("file.txt"), content).unwrap();

            copy_files_to_overlay(source.path(), output.path(), &[PathBuf::from("file.txt")])
                .unwrap();

            let read_content = fs::read_to_string(output.path().join("file.txt")).unwrap();
            assert_eq!(read_content, content);
        }
    }

    // Tests for generate_overlay_config additional cases
    mod generate_overlay_config_additional_tests {
        use super::*;

        #[test]
        fn handles_special_characters_in_name() {
            let config = generate_overlay_config("test-overlay_123");
            assert!(config.contains("name = test-overlay_123"));
        }

        #[test]
        fn includes_comment_header() {
            let config = generate_overlay_config("test");
            assert!(config.contains("/= Overlay configuration file"));
        }

        #[test]
        fn includes_mappings_example() {
            let config = generate_overlay_config("test");
            assert!(config.contains(".envrc.template = .envrc"));
        }
    }

    // Tests for ResolvedSource
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
                OverlaySource::Local { path } => {
                    assert_eq!(path, PathBuf::from("/origin"));
                }
                _ => panic!("Expected Local source"),
            }
        }
    }

    // Tests for ResolvedSources enum
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

    // Additional edge case tests for line ending handling
    mod line_ending_edge_cases {
        use super::*;

        #[test]
        fn remove_overlay_section_with_mixed_line_endings() {
            // Mix of LF and CRLF within the same file
            let content =
                "before\n# repoverlay:test start\r\n.envrc\n# repoverlay:test end\r\nafter\n";
            let result = remove_overlay_section(content, "test");
            assert!(result.contains("before"));
            assert!(result.contains("after"));
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn remove_overlay_section_with_only_crlf() {
            let content = "*.log\r\n# repoverlay:test start\r\n.envrc\r\n# repoverlay:test end\r\n";
            let result = remove_overlay_section(content, "test");
            assert!(result.contains("*.log"));
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn remove_overlay_section_preserves_trailing_newline() {
            let content = "before\n# repoverlay:test start\n.envrc\n# repoverlay:test end\n";
            let result = remove_overlay_section(content, "test");
            assert!(result.ends_with('\n'));
        }

        #[test]
        fn remove_overlay_section_with_no_trailing_newline() {
            let content = "# repoverlay:test start\n.envrc\n# repoverlay:test end";
            let result = remove_overlay_section(content, "test");
            // Should handle content without trailing newline
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn update_git_exclude_with_existing_crlf_content() {
            let repo = create_test_repo();
            let exclude_path = repo.path().join(".git/info/exclude");

            // Create exclude file with CRLF line endings
            fs::create_dir_all(exclude_path.parent().unwrap()).unwrap();
            fs::write(&exclude_path, "*.log\r\n.DS_Store\r\n").unwrap();

            update_git_exclude(repo.path(), "test", &[".envrc".to_string()], true).unwrap();

            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains(".envrc"));
            assert!(content.contains("# repoverlay:test start"));
        }
    }

    // Tests for duplicate/malformed section markers
    mod malformed_section_tests {
        use super::*;

        #[test]
        fn remove_overlay_section_with_duplicate_start_markers() {
            // Two start markers, only one end marker
            let content =
                "# repoverlay:test start\n# repoverlay:test start\n.envrc\n# repoverlay:test end\n";
            let result = remove_overlay_section(content, "test");
            // Should remove everything between first start and end
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn remove_overlay_section_with_unclosed_section() {
            // Start marker but no end marker
            let content = "before\n# repoverlay:test start\n.envrc\nafter\n";
            let result = remove_overlay_section(content, "test");
            // Content after start should be removed (no end marker means section continues)
            assert!(result.contains("before"));
            assert!(!result.contains(".envrc"));
            assert!(!result.contains("after"));
        }

        #[test]
        fn remove_overlay_section_with_nested_markers() {
            // Nested markers (shouldn't happen, but test robustness)
            let content = "# repoverlay:outer start\n# repoverlay:inner start\n.envrc\n# repoverlay:inner end\n# repoverlay:outer end\n";
            let result = remove_overlay_section(content, "outer");
            assert!(!result.contains(".envrc"));
            assert!(!result.contains("repoverlay:inner"));
        }

        #[test]
        fn any_overlay_sections_remain_with_malformed_marker() {
            // Marker with only "start" but not in correct format
            let content = "# repoverlay start\n.envrc\n";
            assert!(!any_overlay_sections_remain(content));
        }

        #[test]
        fn any_overlay_sections_remain_with_extra_spaces() {
            // Extra spaces in marker
            let content = "#  repoverlay:test  start\n.envrc\n# repoverlay:test end\n";
            // Should not match due to different spacing
            assert!(!any_overlay_sections_remain(content));
        }

        #[test]
        fn remove_overlay_section_cleans_multiple_trailing_newlines() {
            // Content with empty line before section creates multiple trailing newlines after removal
            let content = "line1\n\n# repoverlay:test start\n.envrc\n# repoverlay:test end\n";
            let result = remove_overlay_section(content, "test");
            // Should clean up the double newline at the end
            assert!(result.contains("line1"));
            assert!(!result.contains(".envrc"));
            assert!(
                !result.ends_with("\n\n"),
                "Should not end with double newline"
            );
            assert!(result.ends_with('\n'), "Should end with single newline");
        }

        #[test]
        fn remove_overlay_section_cleans_many_trailing_newlines() {
            // Multiple empty lines before section
            let content = "line1\n\n\n# repoverlay:test start\n.envrc\n# repoverlay:test end\n";
            let result = remove_overlay_section(content, "test");
            // Should clean up all excess trailing newlines
            assert!(
                !result.ends_with("\n\n"),
                "Should not end with double newline"
            );
        }
    }

    // Tests for path validation edge cases
    mod path_validation_tests {
        use super::*;

        #[test]
        fn canonicalize_path_with_nonexistent_path() {
            let result = canonicalize_path(Path::new("/nonexistent/path/xyz"), "Test");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not found"));
        }

        #[test]
        fn validate_git_repo_fails_on_non_git_directory() {
            let temp = TempDir::new().unwrap();
            let result = validate_git_repo(temp.path());
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not a git repository")
            );
        }

        #[test]
        fn validate_git_repo_succeeds_on_git_directory() {
            let repo = create_test_repo();
            let result = validate_git_repo(repo.path());
            assert!(result.is_ok());
        }

        #[test]
        fn resolve_git_dir_returns_git_directory_for_regular_repo() {
            let repo = create_test_repo();
            let result = resolve_git_dir(repo.path());
            assert!(result.is_ok());
            let git_dir = result.unwrap();
            assert!(git_dir.ends_with(".git"));
            assert!(git_dir.is_dir());
        }

        #[test]
        fn resolve_git_dir_handles_worktree() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            // Create a .git file (as in a worktree)
            let worktree_git_dir = temp.path().join("actual-git-dir");
            fs::create_dir_all(&worktree_git_dir).unwrap();

            let git_file_content = format!("gitdir: {}\n", worktree_git_dir.display());
            fs::write(repo_path.join(".git"), git_file_content).unwrap();

            let result = resolve_git_dir(repo_path);
            assert!(result.is_ok());
            let resolved = result.unwrap();
            assert_eq!(
                resolved.canonicalize().unwrap(),
                worktree_git_dir.canonicalize().unwrap()
            );
        }

        #[test]
        fn resolve_git_dir_handles_relative_gitdir_path() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            // Create a .git file with a relative path
            let worktree_git_dir = temp.path().join("actual-git-dir");
            fs::create_dir_all(&worktree_git_dir).unwrap();

            // Use a relative path in the gitdir
            let git_file_content = "gitdir: actual-git-dir\n";
            fs::write(repo_path.join(".git"), git_file_content).unwrap();

            let result = resolve_git_dir(repo_path);
            assert!(result.is_ok());
            let resolved = result.unwrap();
            assert_eq!(
                resolved.canonicalize().unwrap(),
                worktree_git_dir.canonicalize().unwrap()
            );
        }

        #[test]
        fn resolve_git_dir_fails_on_non_git_directory() {
            let temp = TempDir::new().unwrap();
            let result = resolve_git_dir(temp.path());
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Not a git repository")
            );
        }

        #[test]
        fn resolve_git_dir_fails_on_invalid_git_file() {
            let temp = TempDir::new().unwrap();
            // Create a .git file without gitdir line
            fs::write(temp.path().join(".git"), "invalid content\n").unwrap();

            let result = resolve_git_dir(temp.path());
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("no gitdir found"));
        }
    }

    // Tests for browse mode functions
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
            assert_eq!(overlays[0], "target-org/target-repo/test-overlay");
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
            assert_eq!(overlays[0], "microsoft/FluidFramework/ci-config");
            assert_eq!(overlays[1], "microsoft/FluidFramework/vscode-setup");
            assert_eq!(overlays[2], "tylerbutler/some-repo/my-overlay");
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
            assert_eq!(overlays[0], "org/repo/overlay");
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
            assert_eq!(overlays[0], "org/repo/overlay");
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
            assert_eq!(overlays[0], "org/repo/overlay");
        }

        #[test]
        fn parse_overlay_path_valid_three_parts() {
            let result = parse_overlay_path("microsoft/FluidFramework/vscode-setup");
            assert_eq!(
                result,
                Some(("microsoft", "FluidFramework", "vscode-setup"))
            );
        }

        #[test]
        fn parse_overlay_path_too_few_parts() {
            assert_eq!(parse_overlay_path("only-one"), None);
            assert_eq!(parse_overlay_path("only/two"), None);
        }

        #[test]
        fn parse_overlay_path_too_many_parts() {
            assert_eq!(parse_overlay_path("one/two/three/four"), None);
        }

        #[test]
        fn parse_overlay_path_empty() {
            assert_eq!(parse_overlay_path(""), None);
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

    // Tests for fuzzy suggestion helpers
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

    // Tests for visible_subdirs
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

    // Tests for format_overlay_path
    mod format_overlay_path_tests {
        use super::*;

        #[test]
        fn formats_valid_three_part_path() {
            let result = format_overlay_path("microsoft/FluidFramework/vscode-setup");
            // Should contain all parts
            assert!(result.contains("microsoft"));
            assert!(result.contains("FluidFramework"));
            assert!(result.contains("vscode-setup"));
        }

        #[test]
        fn returns_unchanged_for_invalid_path() {
            let result = format_overlay_path("just-one-part");
            assert_eq!(result, "just-one-part");
        }

        #[test]
        fn returns_unchanged_for_two_parts() {
            let result = format_overlay_path("only/two");
            assert_eq!(result, "only/two");
        }

        #[test]
        fn returns_unchanged_for_empty_string() {
            let result = format_overlay_path("");
            assert_eq!(result, "");
        }
    }

    // Tests for resolve_local_path
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

    // Tests for detect_target_from_git_remote
    mod detect_target_tests {
        use super::*;

        #[test]
        fn returns_none_for_non_git_directory() {
            let temp = TempDir::new().unwrap();
            let result = detect_target_from_git_remote(temp.path());
            assert!(result.is_none());
        }

        #[test]
        fn returns_none_for_repo_without_remote() {
            let repo = create_test_repo();
            let result = detect_target_from_git_remote(repo.path());
            assert!(result.is_none());
        }

        #[test]
        fn detects_github_https_remote() {
            let repo = create_test_repo();

            // Add a GitHub remote
            Command::new("git")
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/owner/repo.git",
                ])
                .current_dir(repo.path())
                .output()
                .unwrap();

            let result = detect_target_from_git_remote(repo.path());
            assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
        }

        #[test]
        fn detects_github_ssh_remote() {
            let repo = create_test_repo();

            Command::new("git")
                .args(["remote", "add", "origin", "git@github.com:owner/repo.git"])
                .current_dir(repo.path())
                .output()
                .unwrap();

            let result = detect_target_from_git_remote(repo.path());
            assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
        }

        #[test]
        fn returns_none_for_non_github_remote() {
            let repo = create_test_repo();

            Command::new("git")
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://gitlab.com/owner/repo.git",
                ])
                .current_dir(repo.path())
                .output()
                .unwrap();

            let result = detect_target_from_git_remote(repo.path());
            assert!(result.is_none());
        }
    }

    // Tests for restore_overlays
    mod restore_overlays_tests {
        use super::*;
        use crate::state::{OverlayState, external_state_dir_for_target, load_external_states};
        use crate::testutil::TestContext;

        #[test]
        fn does_not_restore_explicitly_removed_overlay() {
            // This test verifies that `restore` does not re-apply overlays that were
            // explicitly removed via `repoverlay remove`. The issue is that if external
            // state exists but in-repo state was intentionally deleted (via remove command),
            // restore should NOT re-apply the overlay.
            //
            // The fix marks external state with `removed_at` timestamp when an overlay
            // is explicitly removed, so that `restore` knows to skip it.

            let ctx = TestContext::new().with_overlay(&[(".envrc", "export FOO=bar")]);

            // Use canonical path consistently (this is what restore_overlays does internally)
            let canonical_repo_path = ctx.repo_path().canonicalize().unwrap();

            // Step 1: Apply the overlay
            apply_overlay(
                ctx.overlay_source(),
                ctx.repo_path(),
                false, // use symlinks
                None,  // auto-name
                None,  // no ref override
                false, // don't update cache
                ConflictStrategy::default(),
                false,
                None,  // default source resolution
                false, // not dry run
            )
            .expect("apply should succeed");

            // Verify overlay was applied
            assert!(
                ctx.file_exists(".envrc"),
                "overlay file should exist after apply"
            );
            assert!(
                ctx.overlay_state_exists("test-overlay") || ctx.state_dir_exists(),
                "in-repo state should exist"
            );

            // Verify external state was saved (before removal)
            let ext_dir = external_state_dir_for_target(&canonical_repo_path).unwrap();
            assert!(ext_dir.exists(), "external state directory should exist");

            // Step 2: Remove the overlay (this simulates explicit user removal)
            // This should mark the external state with `removed_at` instead of deleting it.
            let applied = list_applied_overlays(ctx.repo_path()).expect("list should work");
            assert!(
                !applied.is_empty(),
                "at least one overlay should be applied"
            );
            let overlay_name = &applied[0];

            remove_overlay(ctx.repo_path(), Some(overlay_name.clone()), false, false)
                .expect("remove should succeed");

            // Verify overlay was removed from in-repo state
            assert!(!ctx.file_exists(".envrc"), "overlay file should be removed");
            assert!(
                !ctx.overlay_state_exists(overlay_name),
                "in-repo state should be removed"
            );

            // Verify external state file still exists (with removed_at marker)
            let ext_state_file = ext_dir.join(format!("{overlay_name}.ccl"));
            assert!(
                ext_state_file.exists(),
                "external state file should still exist (as tombstone)"
            );

            // Read the external state and verify it has removed_at set
            let content = fs::read_to_string(&ext_state_file).unwrap();
            let ext_state: OverlayState = sickle::from_str(&content).unwrap();
            assert!(
                ext_state.removed_at.is_some(),
                "external state should have removed_at marker"
            );

            // Verify load_external_states skips removed overlays
            let external_states =
                load_external_states(&canonical_repo_path).expect("load should work");
            assert_eq!(
                external_states.len(),
                0,
                "load_external_states should skip removed overlays"
            );

            // Step 3: Call restore - this SHOULD NOT restore the overlay
            // because it was explicitly removed (has removed_at marker).
            restore_overlays(ctx.repo_path(), false, ConflictStrategy::default(), false)
                .expect("restore should succeed");

            // Step 4: Verify the overlay was NOT restored
            assert!(
                !ctx.file_exists(".envrc"),
                "overlay file should NOT be restored after explicit removal"
            );
        }

        #[test]
        fn restores_overlay_after_git_clean() {
            // This test verifies that `restore` DOES re-apply overlays when
            // in-repo state is missing due to `git clean -fdx` (not explicit removal).
            //
            // The external state should NOT have `removed_at` set because the
            // overlay was not explicitly removed.

            let ctx = TestContext::new().with_overlay(&[(".envrc", "export FOO=bar")]);
            let canonical_repo_path = ctx.repo_path().canonicalize().unwrap();

            // Step 1: Apply the overlay
            apply_overlay(
                ctx.overlay_source(),
                ctx.repo_path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .expect("apply should succeed");

            let applied = list_applied_overlays(ctx.repo_path()).expect("list should work");
            assert!(!applied.is_empty());

            // Verify external state exists and doesn't have removed_at
            let ext_states = load_external_states(&canonical_repo_path).unwrap();
            assert_eq!(ext_states.len(), 1);
            assert!(
                ext_states[0].removed_at.is_none(),
                "external state should NOT have removed_at"
            );

            // Step 2: Simulate `git clean -fdx` by removing only in-repo state
            // This does NOT call remove_overlay, so external state stays intact.
            fs::remove_dir_all(ctx.repo_path().join(".repoverlay")).unwrap();
            // Also remove the overlay files (as git clean would)
            fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();

            // Verify in-repo state is gone
            assert!(!ctx.state_dir_exists(), "in-repo state should be removed");
            assert!(
                !ctx.file_exists(".envrc"),
                "overlay files should be removed"
            );

            // External state should still be loadable (no removed_at marker)
            let ext_states_after = load_external_states(&canonical_repo_path).unwrap();
            assert_eq!(
                ext_states_after.len(),
                1,
                "external state should still be loadable"
            );

            // Step 3: Call restore - this SHOULD restore the overlay
            restore_overlays(ctx.repo_path(), false, ConflictStrategy::default(), false)
                .expect("restore should succeed");

            // Step 4: Verify the overlay WAS restored
            assert!(
                ctx.file_exists(".envrc"),
                "overlay file should be restored after git clean"
            );
        }

        #[test]
        fn reapplying_overlay_clears_removed_marker() {
            // This test verifies that re-applying an overlay clears the removed_at marker
            // in case the user changes their mind after removal.

            let ctx = TestContext::new().with_overlay(&[(".envrc", "export FOO=bar")]);
            let canonical_repo_path = ctx.repo_path().canonicalize().unwrap();

            // Step 1: Apply the overlay
            apply_overlay(
                ctx.overlay_source(),
                ctx.repo_path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .expect("apply should succeed");

            let applied = list_applied_overlays(ctx.repo_path()).expect("list should work");
            let overlay_name = &applied[0];

            // Step 2: Remove the overlay (marks removed_at)
            remove_overlay(ctx.repo_path(), Some(overlay_name.clone()), false, false)
                .expect("remove should succeed");

            // Verify removed_at is set
            let ext_dir = external_state_dir_for_target(&canonical_repo_path).unwrap();
            let ext_state_file = ext_dir.join(format!("{overlay_name}.ccl"));
            let content = fs::read_to_string(&ext_state_file).unwrap();
            let ext_state: OverlayState = sickle::from_str(&content).unwrap();
            assert!(ext_state.removed_at.is_some());

            // Step 3: Re-apply the overlay
            apply_overlay(
                ctx.overlay_source(),
                ctx.repo_path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .expect("re-apply should succeed");

            // Step 4: Verify removed_at is cleared
            let content_after = fs::read_to_string(&ext_state_file).unwrap();
            let ext_state_after: OverlayState = sickle::from_str(&content_after).unwrap();
            assert!(
                ext_state_after.removed_at.is_none(),
                "removed_at should be cleared after re-apply"
            );

            // Verify restore would now restore this overlay
            // (if git clean happened again)
            let ext_states = load_external_states(&canonical_repo_path).unwrap();
            assert_eq!(ext_states.len(), 1, "external state should be loadable");
        }
    }

    mod check_overlay_conflicts_tests {
        use super::*;

        fn make_overlay(dir: &Path, files: &[&str]) {
            for file in files {
                let file_path = dir.join(file);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(file_path, "content").unwrap();
            }
        }

        #[test]
        fn no_conflicts_between_non_overlapping_overlays() {
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[".envrc"]);
            make_overlay(overlay_b.path(), &["config.json"]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            assert!(check_overlay_conflicts(&sources).is_ok());
        }

        #[test]
        fn detects_file_conflict_between_overlays() {
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[".envrc", "unique-a.txt"]);
            make_overlay(overlay_b.path(), &[".envrc", "unique-b.txt"]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let result = check_overlay_conflicts(&sources);
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(
                err_msg.contains(".envrc"),
                "error should mention conflicting file"
            );
            assert!(
                err_msg.contains("Conflict"),
                "error should mention conflict"
            );
        }

        #[test]
        fn detects_directory_conflict_between_overlays() {
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            // Both overlays declare ".claude" as a directory in their config
            make_overlay(overlay_a.path(), &[".claude/CLAUDE.md"]);
            make_overlay(overlay_b.path(), &[".claude/other.md"]);

            let config_content = "overlay =\n  name = test\n\ndirectories =\n  = .claude\n";
            fs::write(overlay_a.path().join(CONFIG_FILE), config_content).unwrap();
            fs::write(overlay_b.path().join(CONFIG_FILE), config_content).unwrap();

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let result = check_overlay_conflicts(&sources);
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(
                err_msg.contains(".claude"),
                "error should mention conflicting directory"
            );
        }

        #[test]
        fn skips_config_and_git_files_in_conflict_check() {
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            // Both overlays have repoverlay.ccl - should not conflict
            make_overlay(overlay_a.path(), &["file-a.txt"]);
            make_overlay(overlay_b.path(), &["file-b.txt"]);
            fs::write(overlay_a.path().join(CONFIG_FILE), "").unwrap();
            fs::write(overlay_b.path().join(CONFIG_FILE), "").unwrap();

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            assert!(check_overlay_conflicts(&sources).is_ok());
        }
    }

    mod check_overlay_conflicts_edge_cases {
        use super::*;

        fn make_overlay(dir: &Path, files: &[&str]) {
            for file in files {
                let file_path = dir.join(file);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(file_path, "content").unwrap();
            }
        }

        #[test]
        fn detects_directory_overlapping_existing_file() {
            // Overlay A has a file ".claude/settings.json"
            // Overlay B declares ".claude" as a directory
            // This should conflict because the directory subsumes the file
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[".claude/settings.json"]);
            // Overlay B declares .claude as a managed directory
            let config_b = "overlay =\n  name = overlay-b\n\ndirectories =\n  = .claude\n";
            make_overlay(overlay_b.path(), &[".claude/other.md"]);
            fs::write(overlay_b.path().join(CONFIG_FILE), config_b).unwrap();

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let result = check_overlay_conflicts(&sources);
            assert!(
                result.is_err(),
                "should detect directory-over-file conflict"
            );
            let err_msg = result.unwrap_err().to_string();
            assert!(
                err_msg.contains(".claude"),
                "error should mention the conflicting path: {err_msg}"
            );
        }

        #[test]
        fn detects_file_under_claimed_directory() {
            // Overlay A declares ".claude" as a managed directory
            // Overlay B has a file ".claude/commands.md"
            // This should conflict
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            let config_a = "overlay =\n  name = overlay-a\n\ndirectories =\n  = .claude\n";
            make_overlay(overlay_a.path(), &[".claude/settings.json"]);
            fs::write(overlay_a.path().join(CONFIG_FILE), config_a).unwrap();

            make_overlay(overlay_b.path(), &[".claude/commands.md"]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let result = check_overlay_conflicts(&sources);
            assert!(
                result.is_err(),
                "should detect file-under-directory conflict"
            );
            let err_msg = result.unwrap_err().to_string();
            assert!(
                err_msg.contains(".claude"),
                "error should mention the directory: {err_msg}"
            );
        }

        #[test]
        fn allows_file_under_own_directory() {
            // Same overlay declares ".claude" as directory AND has files under it
            // This is normal and should NOT conflict
            let overlay_a = TempDir::new().unwrap();

            let config_a = "overlay =\n  name = overlay-a\n\ndirectories =\n  = .claude\n";
            make_overlay(overlay_a.path(), &[".claude/settings.json", ".envrc"]);
            fs::write(overlay_a.path().join(CONFIG_FILE), config_a).unwrap();

            let sources = vec![ResolvedSource {
                path: overlay_a.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
            }];

            assert!(
                check_overlay_conflicts(&sources).is_ok(),
                "files under own directory should not conflict"
            );
        }

        #[test]
        fn single_source_never_conflicts() {
            let overlay = TempDir::new().unwrap();
            make_overlay(
                overlay.path(),
                &[".envrc", "config.json", ".claude/settings.json"],
            );

            let sources = vec![ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::local(overlay.path().to_path_buf()),
            }];

            assert!(check_overlay_conflicts(&sources).is_ok());
        }

        #[test]
        fn three_overlays_no_conflict() {
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();
            let overlay_c = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &["file-a.txt"]);
            make_overlay(overlay_b.path(), &["file-b.txt"]);
            make_overlay(overlay_c.path(), &["file-c.txt"]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_c.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_c.path().to_path_buf()),
                },
            ];

            assert!(check_overlay_conflicts(&sources).is_ok());
        }

        #[test]
        fn three_overlays_with_conflict_in_third() {
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();
            let overlay_c = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &["file-a.txt"]);
            make_overlay(overlay_b.path(), &["file-b.txt"]);
            make_overlay(overlay_c.path(), &["file-a.txt"]); // conflicts with overlay_a

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_c.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_c.path().to_path_buf()),
                },
            ];

            let result = check_overlay_conflicts(&sources);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("file-a.txt"));
        }
    }

    mod apply_multiple_overlays_tests {
        use super::*;

        fn make_overlay(dir: &Path, files: &[(&str, &str)]) {
            for (name, content) in files {
                let file_path = dir.join(name);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(file_path, content).unwrap();
            }
        }

        #[test]
        fn applies_multiple_non_conflicting_overlays() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);
            make_overlay(overlay_b.path(), &[("config.json", "{}")]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_ok(), "multi-apply should succeed: {result:?}");

            // Both overlays should be applied
            let applied = list_applied_overlays(&canonical).unwrap();
            assert_eq!(applied.len(), 2, "should have 2 applied overlays");

            // Files should exist
            assert!(canonical.join(".envrc").exists(), ".envrc should exist");
            assert!(
                canonical.join("config.json").exists(),
                "config.json should exist"
            );
        }

        #[test]
        fn rejects_conflicting_overlays_before_applying() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            // Both overlays have the same file
            make_overlay(overlay_a.path(), &[(".envrc", "version a")]);
            make_overlay(overlay_b.path(), &[(".envrc", "version b")]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_err(), "should fail due to conflict");

            // No overlays should be applied
            let applied = list_applied_overlays(&canonical).unwrap();
            assert!(
                applied.is_empty(),
                "no overlays should be applied after conflict"
            );

            // No files should exist
            assert!(
                !canonical.join(".envrc").exists(),
                ".envrc should not exist"
            );
        }

        #[test]
        fn dry_run_does_not_apply() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);

            let sources = vec![ResolvedSource {
                path: overlay_a.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
            }];

            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                false,
                true,
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_ok(), "dry run should succeed");

            // No files should be applied
            assert!(
                !canonical.join(".envrc").exists(),
                ".envrc should not exist in dry run"
            );
        }

        #[test]
        fn rolls_back_on_failure() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);
            make_overlay(overlay_b.path(), &[("config.json", "{}")]);

            // Pre-create config.json in the repo to cause a conflict when the second
            // overlay tries to apply (existing file conflict)
            fs::write(repo.path().join("config.json"), "existing").unwrap();

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(
                result.is_err(),
                "should fail because config.json already exists"
            );

            // First overlay should be rolled back
            let applied = list_applied_overlays(&canonical).unwrap();
            assert!(
                applied.is_empty(),
                "first overlay should be rolled back, but found: {applied:?}"
            );

            // The first overlay's file should be cleaned up
            assert!(
                !canonical.join(".envrc").is_symlink(),
                ".envrc symlink should be removed during rollback"
            );
        }
    }

    mod apply_multiple_overlays_edge_cases {
        use super::*;

        fn make_overlay(dir: &Path, files: &[(&str, &str)]) {
            for (name, content) in files {
                let file_path = dir.join(name);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(file_path, content).unwrap();
            }
        }

        #[test]
        fn rejects_already_applied_overlay() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);
            make_overlay(overlay_b.path(), &[("config.json", "{}")]);

            let canonical = repo.path().canonicalize().unwrap();

            // First, apply overlay_a individually
            let first_sources = vec![ResolvedSource {
                path: overlay_a.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
            }];
            let result = apply_multiple_overlays(
                &first_sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_ok(), "first apply should succeed: {result:?}");

            // Now try to apply both, including the already-applied overlay_a
            let second_sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];
            let result = apply_multiple_overlays(
                &second_sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(
                result.is_err(),
                "should fail because overlay is already applied"
            );
            let err_msg = result.unwrap_err().to_string();
            assert!(
                err_msg.contains("already applied"),
                "error should mention already applied: {err_msg}"
            );
        }

        #[test]
        fn dry_run_with_multiple_overlays() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);
            make_overlay(overlay_b.path(), &[("config.json", "{}")]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                false,
                true,
                ConflictStrategy::default(),
                false,
            );
            assert!(
                result.is_ok(),
                "dry run with multiple should succeed: {result:?}"
            );

            // No files should be applied
            assert!(
                !canonical.join(".envrc").exists(),
                ".envrc should not exist in dry run"
            );
            assert!(
                !canonical.join("config.json").exists(),
                "config.json should not exist in dry run"
            );

            // No overlays should be recorded
            let applied = list_applied_overlays(&canonical).unwrap();
            assert!(
                applied.is_empty(),
                "no overlays should be recorded in dry run"
            );
        }

        #[test]
        fn applies_three_overlays_successfully() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();
            let overlay_c = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);
            make_overlay(overlay_b.path(), &[("config.json", "{}")]);
            make_overlay(overlay_c.path(), &[("setup.sh", "#!/bin/bash")]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_c.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_c.path().to_path_buf()),
                },
            ];

            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_ok(), "three overlays should succeed: {result:?}");

            let applied = list_applied_overlays(&canonical).unwrap();
            assert_eq!(applied.len(), 3, "should have 3 applied overlays");

            assert!(canonical.join(".envrc").exists());
            assert!(canonical.join("config.json").exists());
            assert!(canonical.join("setup.sh").exists());
        }

        #[test]
        fn force_copy_applies_as_copies_not_symlinks() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);
            make_overlay(overlay_b.path(), &[("config.json", "{}")]);

            let sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];

            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                true,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(
                result.is_ok(),
                "force_copy multi-apply should succeed: {result:?}"
            );

            // Files should exist and NOT be symlinks (they should be copies)
            let envrc_path = canonical.join(".envrc");
            assert!(envrc_path.exists(), ".envrc should exist");
            assert!(
                !envrc_path.is_symlink(),
                ".envrc should not be a symlink with force_copy"
            );

            let config_path = canonical.join("config.json");
            assert!(config_path.exists(), "config.json should exist");
            assert!(
                !config_path.is_symlink(),
                "config.json should not be a symlink with force_copy"
            );
        }
    }

    mod apply_multiple_overlays_conflict_strategy {
        use super::*;

        fn make_overlay(dir: &Path, files: &[(&str, &str)]) {
            for (name, content) in files {
                let file_path = dir.join(name);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(file_path, content).unwrap();
            }
        }

        #[test]
        fn force_reapplies_already_applied_overlay_in_batch() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);
            make_overlay(overlay_b.path(), &[("config.json", "{}")]);

            let canonical = repo.path().canonicalize().unwrap();

            // First, apply overlay_a individually
            let first_sources = vec![ResolvedSource {
                path: overlay_a.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
            }];
            let result = apply_multiple_overlays(
                &first_sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_ok(), "first apply should succeed: {result:?}");

            // Now re-apply overlay_a along with overlay_b using Force
            let second_sources = vec![
                ResolvedSource {
                    path: overlay_a.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
                },
                ResolvedSource {
                    path: overlay_b.path().to_path_buf(),
                    source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
                },
            ];
            let result = apply_multiple_overlays(
                &second_sources,
                &canonical,
                false,
                false,
                ConflictStrategy::Force,
                false,
            );
            assert!(
                result.is_ok(),
                "force should allow re-applying in batch: {result:?}"
            );

            let applied = list_applied_overlays(&canonical).unwrap();
            assert_eq!(applied.len(), 2, "should have 2 applied overlays");
        }

        #[test]
        fn force_overwrites_existing_repo_files_in_batch() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "overlay content")]);

            // Create existing repo file
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let canonical = repo.path().canonicalize().unwrap();

            let sources = vec![ResolvedSource {
                path: overlay_a.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
            }];
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                false,
                false,
                ConflictStrategy::Force,
                false,
            );
            assert!(
                result.is_ok(),
                "force should overwrite in batch: {result:?}"
            );

            // File should be a symlink now
            assert!(
                canonical.join(".envrc").is_symlink(),
                ".envrc should be a symlink"
            );
        }

        #[test]
        fn skip_conflicts_skips_existing_repo_files_in_batch() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();

            make_overlay(
                overlay_a.path(),
                &[(".envrc", "overlay content"), ("other.txt", "other")],
            );

            // Create existing repo file
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let canonical = repo.path().canonicalize().unwrap();

            let sources = vec![ResolvedSource {
                path: overlay_a.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
            }];
            let result = apply_multiple_overlays(
                &sources,
                &canonical,
                false,
                false,
                ConflictStrategy::SkipConflicts,
                false,
            );
            assert!(
                result.is_ok(),
                "skip_conflicts should succeed in batch: {result:?}"
            );

            // .envrc should NOT be a symlink (kept existing)
            assert!(
                !canonical.join(".envrc").is_symlink(),
                ".envrc should NOT be a symlink"
            );
            assert_eq!(
                fs::read_to_string(canonical.join(".envrc")).unwrap(),
                "existing content",
                ".envrc should have original content"
            );

            // other.txt should be applied
            assert!(
                canonical.join("other.txt").exists(),
                "other.txt should exist"
            );
        }

        #[test]
        fn skip_conflicts_still_rejects_already_applied_overlay_in_batch() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "export FOO=bar")]);

            let canonical = repo.path().canonicalize().unwrap();

            // First, apply overlay_a individually
            let first_sources = vec![ResolvedSource {
                path: overlay_a.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
            }];
            let result = apply_multiple_overlays(
                &first_sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_ok(), "first apply should succeed: {result:?}");

            // Try re-applying with SkipConflicts — should still fail for same-name
            let result = apply_multiple_overlays(
                &first_sources,
                &canonical,
                false,
                false,
                ConflictStrategy::SkipConflicts,
                false,
            );
            assert!(
                result.is_err(),
                "skip_conflicts should fail on already-applied overlay"
            );
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("already applied"),
                "error should mention already applied: {err}"
            );
        }

        #[test]
        fn skip_conflicts_bypasses_cross_overlay_file_check_in_batch() {
            let repo = create_test_repo();
            let overlay_a = TempDir::new().unwrap();
            let overlay_b = TempDir::new().unwrap();

            make_overlay(overlay_a.path(), &[(".envrc", "first")]);
            make_overlay(
                overlay_b.path(),
                &[(".envrc", "second"), ("unique.txt", "unique")],
            );

            let canonical = repo.path().canonicalize().unwrap();

            // Apply overlay_a first
            let first_sources = vec![ResolvedSource {
                path: overlay_a.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_a.path().to_path_buf()),
            }];
            apply_multiple_overlays(
                &first_sources,
                &canonical,
                false,
                false,
                ConflictStrategy::default(),
                false,
            )
            .unwrap();

            // Apply overlay_b with SkipConflicts — should skip .envrc but apply unique.txt
            let second_sources = vec![ResolvedSource {
                path: overlay_b.path().to_path_buf(),
                source_info: OverlaySource::local(overlay_b.path().to_path_buf()),
            }];
            let result = apply_multiple_overlays(
                &second_sources,
                &canonical,
                false,
                false,
                ConflictStrategy::SkipConflicts,
                false,
            );
            assert!(
                result.is_ok(),
                "skip_conflicts should succeed with cross-overlay conflict: {result:?}"
            );

            // unique.txt should be applied
            assert!(
                canonical.join("unique.txt").exists(),
                "unique.txt should be applied"
            );
        }
    }

    mod path_traversal_tests {
        use super::*;

        fn make_overlay_with_config(dir: &Path, files: &[(&str, &str)], config: &str) {
            for (name, content) in files {
                let file_path = dir.join(name);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(file_path, content).unwrap();
            }
            fs::write(dir.join("repoverlay.ccl"), config).unwrap();
        }

        fn try_apply(overlay: &Path, target: &Path) -> Result<()> {
            let resolved = ResolvedSource {
                path: overlay.to_path_buf(),
                source_info: OverlaySource::local(overlay.to_path_buf()),
            };
            let canonical = target.canonicalize().unwrap();
            apply_resolved_overlay(
                &resolved,
                &canonical,
                true,
                None,
                ConflictStrategy::default(),
            )
        }

        #[test]
        fn rejects_escape_at_root() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("secret.txt", "payload")],
                "mappings =\n  secret.txt = ../etc/passwd\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            assert!(result.is_err(), "should reject ../etc/passwd mapping");
            assert!(
                result.unwrap_err().to_string().contains("Path traversal"),
                "error should mention path traversal"
            );
        }

        #[test]
        fn rejects_escape_through_parent() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("secret.txt", "payload")],
                "mappings =\n  secret.txt = foo/../../etc/passwd\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            assert!(
                result.is_err(),
                "should reject foo/../../etc/passwd mapping"
            );
        }

        #[test]
        fn allows_traversal_within_target() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("file.txt", "content")],
                "mappings =\n  file.txt = foo/../bar\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            assert!(result.is_ok(), "foo/../bar should be allowed: {result:?}");
            let canonical = repo.path().canonicalize().unwrap();
            assert!(
                canonical.join("bar").exists(),
                "bar should exist after apply"
            );
        }

        #[test]
        fn allows_deeper_traversal_within_target() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("file.txt", "content")],
                "mappings =\n  file.txt = foo/bar/../../baz\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            assert!(
                result.is_ok(),
                "foo/bar/../../baz should be allowed: {result:?}"
            );
            let canonical = repo.path().canonicalize().unwrap();
            assert!(
                canonical.join("baz").exists(),
                "baz should exist after apply"
            );
        }
    }

    mod symlink_escape_tests {
        use super::*;

        #[test]
        #[cfg(unix)]
        fn symlinks_in_overlay_source_are_not_copied() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create a real file and a symlink in the overlay
            fs::write(overlay.path().join("real.txt"), "real content").unwrap();
            std::os::unix::fs::symlink("/etc/passwd", overlay.path().join("evil_link")).unwrap();

            let resolved = ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::local(overlay.path().to_path_buf()),
            };
            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_resolved_overlay(
                &resolved,
                &canonical,
                true,
                None,
                ConflictStrategy::default(),
            );
            assert!(result.is_ok(), "apply should succeed: {result:?}");

            // Real file should be copied
            assert!(
                canonical.join("real.txt").exists(),
                "real.txt should be applied"
            );
            // Symlink should NOT be copied (WalkDir skips symlinks by default)
            assert!(
                !canonical.join("evil_link").exists(),
                "symlink should not be copied to target"
            );
        }
    }
}
