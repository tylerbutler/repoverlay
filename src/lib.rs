//! repoverlay - Overlay config files into git repositories without committing them.
//!
//! This is a CLI tool. There is no public library API.

mod cache;
mod cli;
mod config;
mod detection;
mod github;
mod overlay_repo;
mod selection;
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
use github::GitHubSource;
use overlay_repo::copy_dir_recursive;
use state::{
    CONFIG_FILE, EntryType, FileEntry, GIT_EXCLUDE, GlobalMeta, LinkType, MANAGED_SECTION_NAME,
    META_FILE, OVERLAYS_DIR, OverlayConfig, OverlaySource, OverlayState, STATE_DIR,
    exclude_marker_end, exclude_marker_start, list_applied_overlays, load_all_overlay_targets,
    load_external_states, load_overlay_state, normalize_overlay_name, remove_external_state,
    save_external_state, save_overlay_state,
};
use upstream::detect_upstream;

/// Canonicalize a path and return an error with a descriptive message if it fails.
pub(crate) fn canonicalize_path(path: &Path, description: &str) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("{} not found: {}", description, path.display()))
}

/// Validate that a path is a git repository (has a .git directory).
pub(crate) fn validate_git_repo(path: &Path) -> Result<()> {
    if !path.join(".git").exists() {
        bail!("Target is not a git repository: {}", path.display());
    }
    Ok(())
}

/// Resolved source information for applying an overlay.
pub(crate) struct ResolvedSource {
    /// Local path to the overlay files
    pub path: PathBuf,
    /// Source information for state tracking
    pub source_info: OverlaySource,
}

/// Resolve a source string to a local path.
///
/// Resolution order:
/// 1. GitHub URL (`https://github.com/...`) - downloads to cache, returns cached path
/// 2. Local path (`./path` or `/path`) - returns path directly after validation
/// 3. Overlay repo reference (`org/repo/name`) - resolves from configured shared repository
///    - First tries exact match (org/repo/name)
///    - Falls back to upstream if target_path has an upstream remote
///
/// # Errors
///
/// Returns an error if:
/// - The source doesn't match any valid format
/// - A local path doesn't exist
/// - GitHub fetch fails
/// - Overlay repo is not configured (for org/repo/name format)
pub(crate) fn resolve_source(
    source_str: &str,
    ref_override: Option<&str>,
    update: bool,
    target_path: Option<&Path>,
) -> Result<ResolvedSource> {
    debug!(
        "resolve_source: {} (ref_override={:?}, update={})",
        source_str, ref_override, update
    );

    // Try to parse as GitHub URL
    if GitHubSource::is_github_url(source_str) {
        debug!("detected GitHub URL");
        let mut github_source = GitHubSource::parse(source_str)?;

        // Apply ref override if provided
        if let Some(ref_str) = ref_override {
            github_source = github_source.with_ref_override(Some(ref_str));
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

        return Ok(ResolvedSource {
            path: cached.path,
            source_info: OverlaySource::github(
                source_str.to_string(),
                github_source.owner,
                github_source.repo,
                github_source.git_ref.as_str().to_string(),
                cached.commit,
                github_source
                    .subpath
                    .map(|p| p.to_string_lossy().to_string()),
            ),
        });
    }

    // Try to parse as local path first
    let path = PathBuf::from(source_str);
    if path.exists() {
        debug!("resolved as local path: {}", path.display());
        let canonical = path
            .canonicalize()
            .with_context(|| format!("Overlay source not found: {}", source_str))?;

        return Ok(ResolvedSource {
            path: canonical.clone(),
            source_info: OverlaySource::local(canonical),
        });
    }

    // Try to parse as overlay repo reference (org/repo/name)
    if let Some((org, repo, name)) = overlay_repo::parse_overlay_reference(source_str) {
        debug!(
            "parsed as overlay repo reference: {}/{}/{}",
            org, repo, name
        );
        // Load config and create overlay repo manager
        let config = config::load_config(None)?;
        let overlay_config = config.overlay_repo.ok_or_else(|| {
            anyhow::anyhow!(
                "Overlay repository not configured.\n\n\
                 To apply overlays from a shared repository, first run:\n\
                 repoverlay init-repo <url>\n\n\
                 Or use a local path or GitHub URL instead."
            )
        })?;

        let manager = overlay_repo::OverlayRepoManager::new(overlay_config)?;
        manager.ensure_cloned()?;

        if update {
            println!("{} overlay repository...", "Updating".blue().bold());
            manager.pull()?;
        }

        // Detect upstream for fallback resolution
        let upstream = target_path.and_then(|p| detect_upstream(p).ok()).flatten();

        // Try to resolve with fallback
        let (overlay_path, resolved_via) =
            manager.get_overlay_path_with_fallback(&org, &repo, &name, upstream.as_ref())?;

        let commit = manager.get_current_commit()?;

        // Determine actual org/repo for state tracking
        let via_upstream = resolved_via == state::ResolvedVia::Upstream;
        let (actual_org, actual_repo) = match (&upstream, via_upstream) {
            (Some(up), true) => (up.org.clone(), up.repo.clone()),
            _ => (org.clone(), repo.clone()),
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
                name,
                commit,
                resolved_via,
            ),
        });
    }

    // Nothing matched
    bail!(
        "Overlay source not found: {}\n\n\
         Valid formats:\n\
         - Local path: ./my-overlay\n\
         - GitHub URL: https://github.com/owner/repo\n\
         - Overlay repo: org/repo/name",
        source_str
    )
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
/// - Overlay with same name already exists
/// - File conflicts with existing overlay or repo file
/// - No files found in overlay source
pub(crate) fn apply_overlay(
    source_str: &str,
    target: &Path,
    force_copy: bool,
    name_override: Option<String>,
    ref_override: Option<&str>,
    update_cache: bool,
) -> Result<()> {
    debug!(
        "apply_overlay: source={}, target={}, force_copy={}, name_override={:?}",
        source_str,
        target.display(),
        force_copy,
        name_override
    );

    // Resolve source (handles GitHub URLs and local paths)
    // Pass target to enable upstream detection for fork inheritance
    let resolved = resolve_source(source_str, ref_override, update_cache, Some(target))?;
    let source = &resolved.path;
    debug!("resolved source path: {}", source.display());

    // Validate target exists and is a git repo
    let target = canonicalize_path(target, "Target directory")?;
    validate_git_repo(&target)?;

    // Determine link type
    let link_type = if force_copy || cfg!(windows) {
        LinkType::Copy
    } else {
        LinkType::Symlink
    };

    // Load overlay config (optional)
    let config_path = source.join(CONFIG_FILE);
    let config: OverlayConfig = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
        sickle::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", config_path.display()))?
    } else {
        OverlayConfig::default()
    };

    // Determine overlay name (priority: CLI override > config > directory name)
    let overlay_name = name_override
        .or(config.overlay.name.clone())
        .unwrap_or_else(|| {
            source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unnamed".to_string())
        });
    let normalized_name = normalize_overlay_name(&overlay_name)?;

    // Check if this specific overlay already exists
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    let overlay_state_path = overlays_dir.join(format!("{}.ccl", normalized_name));
    if overlay_state_path.exists() {
        bail!(
            "Overlay '{}' is already applied. Run 'repoverlay remove {}' first.",
            overlay_name,
            normalized_name
        );
    }

    // Load all existing overlay targets to check for conflicts
    let existing_targets = load_all_overlay_targets(&target)?;

    println!("{} overlay: {}", "Applying".green().bold(), overlay_name);

    // Collect files to overlay and build state
    let mut state = OverlayState::new(overlay_name.clone(), resolved.source_info);
    let mut exclude_entries: Vec<String> = Vec::new();

    // Build set of directories to symlink as units
    let dir_set: std::collections::HashSet<PathBuf> =
        config.directories.iter().map(PathBuf::from).collect();

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
            bail!(
                "Conflict: directory '{}' is already managed by overlay '{}'\n\
                 Remove that overlay first or use different file mappings.",
                dir_path.display(),
                conflicting_overlay
            );
        }

        let target_dir = target.join(&dir_path);

        // Check for conflicts with existing files/dirs in repo
        if target_dir.exists() {
            bail!(
                "Conflict: target path already exists: {}\n\
                 Remove it first to apply the overlay.",
                target_dir.display()
            );
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
            LinkType::Copy => {
                // For copy mode, create the target directory and recursively copy contents
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

    for entry in WalkDir::new(source)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel_path = entry.path().strip_prefix(source)?;

        // Skip the config file
        if rel_path == Path::new(CONFIG_FILE) {
            continue;
        }

        // Skip git directory and cache metadata
        let rel_str_check = rel_path.to_string_lossy();
        if rel_str_check.starts_with(".git/")
            || rel_str_check.starts_with(".git\\")
            || rel_str_check == ".git"
            || rel_str_check == ".repoverlay-cache-meta.ccl"
        {
            continue;
        }

        // Skip files that are within directories being symlinked as units
        let mut skip_file = false;
        for dir in &dir_set {
            if rel_path.starts_with(dir) {
                skip_file = true;
                break;
            }
        }
        if skip_file {
            continue;
        }

        let rel_str = rel_path.to_string_lossy().to_string();

        // Apply path mapping if defined
        let target_rel = config
            .mappings
            .get(&rel_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| rel_path.to_path_buf());

        let target_rel_str = target_rel.to_string_lossy().to_string();
        let source_file = entry.path().to_path_buf();
        let target_file = target.join(&target_rel);

        // Validate that the target file is within the target directory (prevent path traversal)
        // We need to resolve the path to handle .. components, but the file doesn't exist yet.
        // So we create parent dirs first (if needed) and then check the canonical path.
        // Alternative: check if the path contains .. that escapes the target.
        {
            // Normalize the path by iterating through components
            let mut normalized = target.clone();
            for component in target_rel.components() {
                use std::path::Component;
                match component {
                    Component::ParentDir => {
                        // Check if going up would escape the target directory
                        if !normalized.starts_with(&target) || normalized == target {
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
            if !normalized.starts_with(&target) {
                bail!(
                    "Path traversal detected: mapping '{}' -> '{}' would escape target directory",
                    rel_str,
                    target_rel.display()
                );
            }
        }

        // Check for conflicts with existing overlays
        if let Some(conflicting_overlay) = existing_targets.get(&target_rel_str) {
            bail!(
                "Conflict: file '{}' is already managed by overlay '{}'\n\
                 Remove that overlay first or use different file mappings.",
                target_rel.display(),
                conflicting_overlay
            );
        }

        // Check for conflicts with existing files in repo
        if target_file.exists() {
            bail!(
                "Conflict: target file already exists: {}\n\
                 Remove it first or add a mapping to rename the overlay file.",
                target_file.display()
            );
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
        }

        println!("  {} {}", "+".green(), target_rel.display());

        state.add_file(FileEntry {
            source: rel_path.to_path_buf(),
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
    update_git_exclude(&target, &normalized_name, &exclude_entries, true)?;

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
    save_overlay_state(&target, &state)?;

    // Save external backup for restore capability
    if let Err(e) = save_external_state(&target, &normalized_name, &state) {
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
pub(crate) fn remove_overlay(target: &Path, name: Option<String>, remove_all: bool) -> Result<()> {
    debug!(
        "remove_overlay: target={}, name={:?}, remove_all={}",
        target.display(),
        name,
        remove_all
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
    debug!("remove_single_overlay: {}", name);
    let state_file = overlays_dir.join(format!("{}.ccl", name));

    if !state_file.exists() {
        // List available overlays for helpful error message
        let available = list_applied_overlays(target)?;

        if available.is_empty() {
            bail!("No overlays are currently applied");
        } else {
            bail!(
                "Overlay '{}' not found. Available overlays: {}",
                name,
                available.join(", ")
            );
        }
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
                EntryType::Directory => format!("{}/", path),
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
            println!("    Ref:     {}", git_ref);
            println!("    Commit:  {}", &commit[..12.min(commit.len())]);
            if let Some(sp) = subpath {
                println!("    Subpath: {}", sp);
            }
        }
        OverlaySource::OverlayRepo {
            org,
            repo,
            name: overlay_name,
            commit,
            resolved_via,
        } => {
            let via_upstream = matches!(resolved_via, Some(state::ResolvedVia::Upstream));
            let via_str = if via_upstream {
                format!(" {}", "(via upstream)".yellow())
            } else {
                String::new()
            };
            println!(
                "    Source:  {}/{}/{}{} {}",
                org,
                repo,
                overlay_name,
                via_str,
                "(overlay repo)".dimmed()
            );
            println!("    Commit:  {}", &commit[..12.min(commit.len())]);
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
pub(crate) fn restore_overlays(target: &Path, dry_run: bool) -> Result<()> {
    debug!(
        "restore_overlays: target={}, dry_run={}",
        target.display(),
        dry_run
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
                println!("    Source: {} ({})", url, git_ref);
            }
            OverlaySource::OverlayRepo {
                org,
                repo,
                name: overlay_name,
                ..
            } => {
                println!(
                    "    Source: {}/{}/{} (overlay repo)",
                    org, repo, overlay_name
                );
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
                format!("{}/{}/{}", org, repo, overlay_name)
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
pub(crate) fn update_overlays(target: &Path, name: Option<String>, dry_run: bool) -> Result<()> {
    debug!(
        "update_overlays: target={}, name={:?}, dry_run={}",
        target.display(),
        name,
        dry_run
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
                git_ref: git_ref.parse().unwrap(),
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
/// 1. If an overlay repo is configured (`init-repo` was run), the overlay is
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
        match &output {
            Some(p) => (p.clone(), None),
            None => {
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
                output_dir.clone()
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
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                {
                    let rel = entry.path().strip_prefix(source).unwrap_or(entry.path());
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
                .filter_map(|e| e.ok())
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
        r#"/= Overlay configuration file.
/= This file describes an overlay and how it should be applied.

overlay =
  /= name: Display name for this overlay.
  /= Used in status output and when listing overlays.
  name = {}

/= mappings (optional): Remap file paths when applying the overlay.
/= Keys are source paths (in the overlay), values are target paths (in the repo).
/= Use this to rename files or place them in different locations.
/= mappings =
/=   .envrc.template = .envrc
"#,
        name
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
) -> Result<()> {
    validate_git_repo(target)?;

    // Check if any overlays are currently applied
    let state_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    let has_overlays = state_dir.exists() && fs::read_dir(&state_dir)?.next().is_some();

    if has_overlays {
        println!("{} existing overlays...", "Removing".yellow().bold());
        // Remove all existing overlays
        remove_overlay(target, None, true)?;
    }

    // Apply the new overlay
    println!("{} new overlay...", "Applying".blue().bold());
    apply_overlay(source, target, copy, name, ref_override, false)?;

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
    let exclude_path = target.join(GIT_EXCLUDE);

    // Ensure the .git/info directory exists
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
            anyhow::anyhow!("Could not parse git remote URL: {}", url)
        } else {
            anyhow::anyhow!(
                "Could not detect target repository from git remote.\n\
                 Non-GitHub remotes are not supported for auto-detection.\n\
                 Please specify --target org/repo"
            )
        }
    })
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
    }
}
