//! repoverlay - Overlay config files into git repositories without committing them.
//!
//! This library provides the core functionality for applying, removing, and managing
//! file overlays in git repositories.

pub mod cache;
pub mod github;
pub mod state;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use cache::CacheManager;
use github::GitHubSource;
pub use state::{
    CONFIG_FILE, FileEntry, GIT_EXCLUDE, GlobalMeta, LinkType, MANAGED_SECTION_NAME, META_FILE,
    OVERLAYS_DIR, OverlayConfig, OverlaySource, OverlayState, STATE_DIR, StateMeta,
    exclude_marker_end, exclude_marker_start, list_applied_overlays, load_all_overlay_targets,
    load_external_states, load_overlay_state, normalize_overlay_name, remove_external_state,
    save_external_state,
};

/// Resolved source information for applying an overlay.
struct ResolvedSource {
    /// Local path to the overlay files
    path: PathBuf,
    /// Source information for state tracking
    source_info: OverlaySource,
}

/// Resolve a source string to a local path.
///
/// For GitHub URLs, downloads/updates the cache and returns the cached path.
/// For local paths, returns the path directly.
fn resolve_source(
    source_str: &str,
    ref_override: Option<&str>,
    update: bool,
) -> Result<ResolvedSource> {
    // Try to parse as GitHub URL
    if GitHubSource::is_github_url(source_str) {
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

    // Treat as local path
    let path = PathBuf::from(source_str);
    let canonical = path
        .canonicalize()
        .with_context(|| format!("Overlay source not found: {}", source_str))?;

    Ok(ResolvedSource {
        path: canonical.clone(),
        source_info: OverlaySource::local(canonical),
    })
}

/// Apply an overlay to a git repository.
///
/// # Arguments
/// * `source_str` - Path to overlay source directory OR GitHub URL
/// * `target` - Target repository directory
/// * `force_copy` - Force copy mode instead of symlinks
/// * `name_override` - Override the overlay name
/// * `ref_override` - Git ref to use (GitHub sources only)
/// * `update_cache` - Force update the cached repository
pub fn apply_overlay(
    source_str: &str,
    target: &Path,
    force_copy: bool,
    name_override: Option<String>,
    ref_override: Option<&str>,
    update_cache: bool,
) -> Result<()> {
    // Resolve source (handles GitHub URLs and local paths)
    let resolved = resolve_source(source_str, ref_override, update_cache)?;
    let source = &resolved.path;

    // Validate target exists and is a git repo
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

    if !target.join(".git").exists() {
        bail!("Target is not a git repository: {}", target.display());
    }

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
        toml::from_str(&content)
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
    let overlay_state_path = overlays_dir.join(format!("{}.toml", normalized_name));
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

    // Collect files to overlay
    let mut files: Vec<FileEntry> = Vec::new();
    let mut exclude_entries: Vec<String> = Vec::new();

    for entry in WalkDir::new(source)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel_path = entry.path().strip_prefix(source)?;

        // Skip the config file itself
        if rel_path == Path::new(CONFIG_FILE) {
            continue;
        }

        // Skip git directory and cache metadata
        let rel_str_check = rel_path.to_string_lossy();
        if rel_str_check.starts_with(".git/")
            || rel_str_check.starts_with(".git\\")
            || rel_str_check == ".git"
            || rel_str_check == ".repoverlay-cache-meta.toml"
        {
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

        files.push(FileEntry {
            source: rel_path.to_path_buf(),
            target: target_rel.clone(),
            link_type,
        });

        // Add to exclude list (use forward slashes for git)
        let exclude_path = target_rel.to_string_lossy().replace('\\', "/");
        exclude_entries.push(exclude_path);
    }

    if files.is_empty() {
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
        fs::write(&meta_path, toml::to_string_pretty(&global_meta)?)?;
    }

    // Save overlay state
    let state = OverlayState {
        meta: StateMeta {
            applied_at: Utc::now(),
            source: resolved.source_info,
            name: overlay_name.clone(),
        },
        files,
    };

    fs::write(&overlay_state_path, toml::to_string_pretty(&state)?)?;

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
        state.files.len(),
        overlay_name
    );

    Ok(())
}

/// Remove applied overlay(s) from a repository.
///
/// # Arguments
/// * `target` - Target repository directory
/// * `name` - Name of the overlay to remove (interactive if None and remove_all is false)
/// * `remove_all` - Remove all applied overlays
pub fn remove_overlay(target: &Path, name: Option<String>, remove_all: bool) -> Result<()> {
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    if remove_all {
        remove_all_overlays(&target, &overlays_dir, &applied_overlays)?;
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
        bail!("No overlay name specified and not in interactive mode");
    }

    Ok(())
}

/// Remove all overlays from a repository.
fn remove_all_overlays(target: &Path, overlays_dir: &Path, applied_overlays: &[String]) -> Result<()> {
    for overlay_name in applied_overlays {
        remove_single_overlay(target, overlays_dir, overlay_name)?;
    }

    // Clean up .repoverlay directory entirely
    fs::remove_dir_all(target.join(STATE_DIR))?;

    println!("\n{} Removed all overlays", "✓".green().bold());
    Ok(())
}

fn remove_single_overlay(target: &Path, overlays_dir: &Path, name: &str) -> Result<()> {
    let state_file = overlays_dir.join(format!("{}.toml", name));

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

    println!("{} overlay: {}", "Removing".red().bold(), state.meta.name);

    // Remove files
    for entry in &state.files {
        let file_path = target.join(&entry.target);

        if file_path.exists() || file_path.is_symlink() {
            fs::remove_file(&file_path)
                .with_context(|| format!("Failed to remove: {}", file_path.display()))?;
            println!("  {} {}", "-".red(), entry.target.display());

            // Remove empty parent directories (but not the target itself)
            remove_empty_parents(&file_path, target);
        }
    }

    // Update git exclude (remove this overlay's section)
    let exclude_entries: Vec<String> = state
        .files
        .iter()
        .map(|f| f.target.to_string_lossy().replace('\\', "/"))
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
        state.files.len(),
        state.meta.name
    );

    Ok(())
}

/// Remove empty parent directories up to (but not including) the target root.
fn remove_empty_parents(file_path: &Path, target: &Path) {
    let mut parent = file_path.parent();
    while let Some(dir) = parent {
        if dir == target {
            break;
        }
        let is_empty = dir
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if is_empty {
            fs::remove_dir(dir).ok();
            parent = dir.parent();
        } else {
            break;
        }
    }
}

/// Show the status of applied overlays.
pub fn show_status(target: &Path, filter_name: Option<String>) -> Result<()> {
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

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

fn show_single_overlay_status(target: &Path, name: &str) -> Result<()> {
    let state = load_overlay_state(target, name)?;

    println!("  {} {}", "Overlay:".bold(), state.meta.name.cyan());

    // Display source based on type
    match &state.meta.source {
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
    }

    println!(
        "    Applied: {}",
        state.meta.applied_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("    Files:   {}", state.files.len());

    for entry in &state.files {
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

        println!(
            "      {} {} ({})",
            status,
            entry.target.display(),
            type_str.dimmed()
        );
    }

    Ok(())
}

fn update_git_exclude(
    target: &Path,
    overlay_name: &str,
    entries: &[String],
    add: bool,
) -> Result<()> {
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

/// Remove a named overlay section from git exclude content.
pub fn remove_overlay_section(content: &str, name: &str) -> String {
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

fn any_overlay_sections_remain(content: &str) -> bool {
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
