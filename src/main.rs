mod cache;
mod github;
mod state;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use cache::CacheManager;
use github::GitHubSource;
use state::{
    CONFIG_FILE, FileEntry, GIT_EXCLUDE, GlobalMeta, LinkType, MANAGED_SECTION_NAME, META_FILE,
    OVERLAYS_DIR, OverlayConfig, OverlaySource, OverlayState, STATE_DIR, StateMeta,
    exclude_marker_end, exclude_marker_start, list_applied_overlays, load_all_overlay_targets,
    load_external_states, load_overlay_state, normalize_overlay_name, remove_external_state,
    save_external_state,
};

/// Overlay config files into git repositories without committing them
#[derive(Parser)]
#[command(name = "repoverlay")]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Apply an overlay to a git repository
    Apply {
        /// Path to overlay source directory OR GitHub URL
        ///
        /// Examples:
        ///   ./my-overlay
        ///   https://github.com/owner/repo
        ///   https://github.com/owner/repo/tree/main/overlays/rust
        source: String,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Force copy mode instead of symlinks (default on Windows)
        #[arg(long)]
        copy: bool,

        /// Override the overlay name (defaults to config name or directory name)
        #[arg(short, long)]
        name: Option<String>,

        /// Git ref (branch, tag, or commit) to use (GitHub sources only)
        #[arg(short, long, value_name = "REF")]
        r#ref: Option<String>,

        /// Force update the cached repository before applying (GitHub sources only)
        #[arg(long)]
        update: bool,
    },

    /// Remove applied overlay(s)
    Remove {
        /// Name of the overlay to remove (interactive if not specified)
        name: Option<String>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Remove all applied overlays
        #[arg(long)]
        all: bool,
    },

    /// Show the status of applied overlays
    Status {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show only a specific overlay
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Restore overlays after git clean or other removal
    Restore {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would be restored without applying
        #[arg(long)]
        dry_run: bool,
    },

    /// Update applied overlays from remote sources
    Update {
        /// Name of the overlay to update (updates all GitHub overlays if not specified)
        name: Option<String>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Check for updates without applying them
        #[arg(long)]
        dry_run: bool,
    },

    /// Create a new overlay from files in a repository
    Create {
        /// Source repository to extract files from (defaults to current directory)
        #[arg(short, long)]
        source: Option<PathBuf>,

        /// Output directory for the new overlay
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Include specific files or directories (can be specified multiple times)
        #[arg(short, long)]
        include: Vec<PathBuf>,

        /// Overlay name (defaults to output directory name)
        #[arg(short, long)]
        name: Option<String>,

        /// Show what would be created without creating files
        #[arg(long)]
        dry_run: bool,

        /// Skip interactive prompts, use defaults
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Switch to a different overlay (removes all existing overlays first)
    Switch {
        /// Path to overlay source directory OR GitHub URL
        source: String,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Force copy mode instead of symlinks (default on Windows)
        #[arg(long)]
        copy: bool,

        /// Override the overlay name
        #[arg(short, long)]
        name: Option<String>,

        /// Git ref (branch, tag, or commit) to use (GitHub sources only)
        #[arg(short, long, value_name = "REF")]
        r#ref: Option<String>,
    },

    /// Manage the overlay cache
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
}

#[derive(Subcommand)]
enum CacheCommand {
    /// List cached repositories
    List,

    /// Clear all cached repositories
    Clear {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Remove a specific cached repository
    Remove {
        /// Repository to remove (format: owner/repo)
        repo: String,
    },

    /// Show cache location
    Path,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Apply {
            source,
            target,
            copy,
            name,
            r#ref,
            update,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            apply_overlay(&source, &target, copy, name, r#ref.as_deref(), update)?;
        }
        Commands::Remove { name, target, all } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            remove_overlay(&target, name, all)?;
        }
        Commands::Status { target, name } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            show_status(&target, name)?;
        }
        Commands::Restore { target, dry_run } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            restore_overlays(&target, dry_run)?;
        }
        Commands::Update {
            name,
            target,
            dry_run,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            update_overlays(&target, name, dry_run)?;
        }
        Commands::Create {
            source,
            output,
            include,
            name,
            dry_run,
            yes,
        } => {
            let source = source.unwrap_or_else(|| PathBuf::from("."));
            create_overlay(&source, output, &include, name, dry_run, yes)?;
        }
        Commands::Switch {
            source,
            target,
            copy,
            name,
            r#ref,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            switch_overlay(&source, &target, copy, name, r#ref.as_deref())?;
        }
        Commands::Cache { command } => {
            handle_cache_command(command)?;
        }
    }

    Ok(())
}

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

fn apply_overlay(
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

fn remove_overlay(target: &Path, name: Option<String>, remove_all: bool) -> Result<()> {
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
        // Interactive selection
        println!("{}", "Select overlay to remove:".bold());
        println!();
        for (i, name) in applied_overlays.iter().enumerate() {
            println!("  {}. {}", i + 1, name);
        }
        println!(
            "  {}. {} (remove all)",
            applied_overlays.len() + 1,
            "all".bold()
        );
        println!();

        print!("Enter selection (1-{}): ", applied_overlays.len() + 1);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if let Ok(selection) = input.parse::<usize>() {
            if selection == applied_overlays.len() + 1 {
                // Remove all
                for overlay_name in &applied_overlays {
                    remove_single_overlay(&target, &overlays_dir, overlay_name)?;
                }
                fs::remove_dir_all(target.join(STATE_DIR))?;
                println!("\n{} Removed all overlays", "✓".green().bold());
            } else if selection >= 1 && selection <= applied_overlays.len() {
                let overlay_name = &applied_overlays[selection - 1];
                remove_single_overlay(&target, &overlays_dir, overlay_name)?;

                let remaining = list_applied_overlays(&target)?;
                if remaining.is_empty() {
                    fs::remove_dir_all(target.join(STATE_DIR))?;
                }
            } else {
                bail!("Invalid selection: {}", selection);
            }
        } else if input.eq_ignore_ascii_case("all") {
            for overlay_name in &applied_overlays {
                remove_single_overlay(&target, &overlays_dir, overlay_name)?;
            }
            fs::remove_dir_all(target.join(STATE_DIR))?;
            println!("\n{} Removed all overlays", "✓".green().bold());
        } else {
            bail!("Invalid selection: {}", input);
        }
    }

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

fn show_status(target: &Path, filter_name: Option<String>) -> Result<()> {
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

fn restore_overlays(target: &Path, dry_run: bool) -> Result<()> {
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

    if !target.join(".git").exists() {
        bail!("Target is not a git repository: {}", target.display());
    }

    // Load external state
    let external_states = load_external_states(&target)?;

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
        println!("  - {}", state.meta.name);
        match &state.meta.source {
            OverlaySource::Local { path } => {
                println!("    Source: {}", path.display());
            }
            OverlaySource::GitHub { url, git_ref, .. } => {
                println!("    Source: {} ({})", url, git_ref);
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
        let source_str = match &state.meta.source {
            OverlaySource::Local { path } => path.to_string_lossy().to_string(),
            OverlaySource::GitHub { url, .. } => url.clone(),
        };

        let ref_override = match &state.meta.source {
            OverlaySource::GitHub { git_ref, .. } => Some(git_ref.as_str()),
            _ => None,
        };

        // Re-apply the overlay
        match apply_overlay(
            &source_str,
            &target,
            false, // Use symlinks by default
            Some(state.meta.name.clone()),
            ref_override,
            true, // Update cache
        ) {
            Ok(()) => {}
            Err(e) => {
                eprintln!(
                    "  {} Failed to restore '{}': {}",
                    "Error:".red(),
                    state.meta.name,
                    e
                );
            }
        }
    }

    Ok(())
}

fn update_overlays(target: &Path, name: Option<String>, dry_run: bool) -> Result<()> {
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
        } = &state.meta.source
        {
            let source = GitHubSource {
                owner: owner.clone(),
                repo: repo.clone(),
                git_ref: github::GitRef::from_str(git_ref),
                subpath: subpath.as_ref().map(PathBuf::from),
            };

            match cache.check_for_updates(&source) {
                Ok(Some(new_commit)) => {
                    updates_available.push((
                        overlay_name.clone(),
                        state.meta.name.clone(),
                        url.clone(),
                        commit.clone(),
                        new_commit,
                    ));
                }
                Ok(None) => {
                    println!("  {} {} is up to date", "✓".green(), state.meta.name);
                }
                Err(e) => {
                    println!(
                        "  {} Could not check {} for updates: {}",
                        "?".yellow(),
                        state.meta.name,
                        e
                    );
                }
            }
        } else {
            println!(
                "  {} {} is a local overlay (not updatable)",
                "-".dimmed(),
                state.meta.name
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
    for (normalized_name, _, _, _, _) in updates_available {
        let state = load_overlay_state(&target, &normalized_name)?;

        if let OverlaySource::GitHub { url, git_ref, .. } = &state.meta.source {
            // Remove old overlay
            let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
            remove_single_overlay(&target, &overlays_dir, &normalized_name)?;

            // Re-apply with update
            apply_overlay(
                url,
                &target,
                false,
                Some(state.meta.name.clone()),
                Some(git_ref),
                true,
            )?;
        }
    }

    Ok(())
}

/// Create a new overlay from files in a repository.
///
/// Discovers AI config files, gitignored files, and untracked files,
/// then copies selected files to the output directory.
fn create_overlay(
    source: &Path,
    output: Option<PathBuf>,
    include: &[PathBuf],
    name: Option<String>,
    dry_run: bool,
    _yes: bool,
) -> Result<()> {
    // Verify source is a git repository
    if !source.join(".git").exists() {
        bail!(
            "Source directory is not a git repository: {}",
            source.display()
        );
    }

    // Determine output directory
    let output_dir = match output {
        Some(p) => p,
        None => {
            // Default to ~/.local/share/repoverlay/overlays/<repo-name>
            let repo_name = source
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("overlay");
            let proj_dirs = directories::ProjectDirs::from("", "", "repoverlay")
                .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;
            proj_dirs.data_dir().join("overlays").join(repo_name)
        }
    };

    // For now, require explicit --include files (interactive mode comes later)
    if include.is_empty() {
        bail!(
            "No files specified. Use --include to specify files to include in the overlay.\n\n\
             Example:\n  repoverlay create --include .claude/ --include CLAUDE.md --output ~/overlays/my-config"
        );
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

    // Create output directory
    fs::create_dir_all(&output_dir)?;

    // Copy files to output directory
    let mut copied_files = Vec::new();
    for path in include {
        let src_path = source.join(path);
        if src_path.is_dir() {
            // Copy directory recursively
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
            // Copy single file
            let dest_path = output_dir.join(path);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src_path, &dest_path)?;
            copied_files.push(path.clone());
        }
    }

    // Generate repoverlay.toml
    let overlay_name = name.unwrap_or_else(|| {
        output_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("overlay")
            .to_string()
    });

    let config_content = format!(
        r#"[overlay]
name = "{}"

[mappings]
# Add path mappings here if needed
# "source_path" = "target_path"
"#,
        overlay_name
    );
    fs::write(output_dir.join("repoverlay.toml"), config_content)?;

    println!(
        "{} overlay at: {}",
        "Created".green().bold(),
        output_dir.display()
    );
    println!();
    println!("Files included:");
    for file in &copied_files {
        println!("  + {}", file.display());
    }
    println!();
    println!(
        "Apply with: {} {} {}",
        "repoverlay apply".cyan(),
        output_dir.display(),
        "--target <repo>".dimmed()
    );

    Ok(())
}

/// Switch to a different overlay by removing all existing overlays first.
///
/// This is equivalent to `repoverlay remove --all && repoverlay apply <source>`,
/// but performed atomically.
fn switch_overlay(
    source: &str,
    target: &Path,
    copy: bool,
    name: Option<String>,
    ref_override: Option<&str>,
) -> Result<()> {
    // Verify target is a git repository
    if !target.join(".git").exists() {
        bail!("Target is not a git repository: {}", target.display());
    }

    // Check if any overlays are currently applied
    let state_dir = target.join(".repoverlay/overlays");
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

fn handle_cache_command(command: CacheCommand) -> Result<()> {
    let cache = CacheManager::new()?;

    match command {
        CacheCommand::List => {
            let repos = cache.list_cached()?;

            if repos.is_empty() {
                println!("{} No repositories cached.", "Cache:".bold());
                return Ok(());
            }

            println!("{} {} cached repository(s):", "Cache:".bold(), repos.len());
            println!();

            for repo in repos {
                println!("  {}/{}", repo.owner.cyan(), repo.repo);
                if let Some(meta) = repo.meta {
                    println!("    Ref:     {}", meta.requested_ref);
                    println!("    Commit:  {}", &meta.commit[..12.min(meta.commit.len())]);
                    println!(
                        "    Fetched: {}",
                        meta.last_fetched.format("%Y-%m-%d %H:%M UTC")
                    );
                }
                println!("    Path:    {}", repo.path.display());
                println!();
            }
        }

        CacheCommand::Clear { yes } => {
            if !yes {
                print!("Clear entire cache? [y/N] ");
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled.");
                    return Ok(());
                }
            }

            let count = cache.clear_cache()?;
            println!(
                "{} Cleared {} cached repository(s).",
                "✓".green().bold(),
                count
            );
        }

        CacheCommand::Remove { repo } => {
            let parts: Vec<&str> = repo.split('/').collect();
            if parts.len() != 2 {
                bail!("Invalid repository format. Use: owner/repo");
            }

            let (owner, repo_name) = (parts[0], parts[1]);

            if cache.remove_cached(owner, repo_name)? {
                println!(
                    "{} Removed {}/{} from cache.",
                    "✓".green().bold(),
                    owner,
                    repo_name
                );
            } else {
                println!("{}/{} is not cached.", owner, repo_name);
            }
        }

        CacheCommand::Path => {
            println!("{}", cache.cache_dir().display());
        }
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

fn remove_overlay_section(content: &str, name: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
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

    // Helper to create a test overlay directory with files
    fn create_test_overlay(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (path, content) in files {
            let file_path = dir.path().join(path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(file_path, content).unwrap();
        }
        dir
    }

    // Unit tests for remove_overlay_section
    mod remove_section {
        use super::*;

        #[test]
        fn empty_content() {
            let result = remove_overlay_section("", "test-overlay");
            assert_eq!(result, "");
        }

        #[test]
        fn no_section_present() {
            let content = "*.log\n.DS_Store\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "*.log\n.DS_Store\n");
        }

        #[test]
        fn section_at_end() {
            let content = "*.log\n# repoverlay:test-overlay start\n.envrc\n.repoverlay\n# repoverlay:test-overlay end\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "*.log\n");
        }

        #[test]
        fn section_at_beginning() {
            let content =
                "# repoverlay:test-overlay start\n.envrc\n# repoverlay:test-overlay end\n*.log\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "*.log\n");
        }

        #[test]
        fn section_in_middle() {
            let content = "*.log\n# repoverlay:test-overlay start\n.envrc\n# repoverlay:test-overlay end\n.DS_Store\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "*.log\n.DS_Store\n");
        }

        #[test]
        fn only_section() {
            let content = "# repoverlay:test-overlay start\n.envrc\n.repoverlay\n# repoverlay:test-overlay end\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "");
        }

        #[test]
        fn removes_only_specified_overlay() {
            let content = "# repoverlay:overlay-a start\n.envrc\n# repoverlay:overlay-a end\n# repoverlay:overlay-b start\n.env\n# repoverlay:overlay-b end\n";
            let result = remove_overlay_section(content, "overlay-a");
            assert!(result.contains("overlay-b"));
            assert!(!result.contains("overlay-a"));
        }
    }

    // Integration tests for apply command
    mod apply {
        use super::*;

        #[test]
        fn applies_single_file() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
            );
            assert!(result.is_ok(), "apply_overlay failed: {:?}", result);

            // Check symlink was created
            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists(), ".envrc should exist");
            assert!(target_file.is_symlink(), ".envrc should be a symlink");

            // Check content is correct
            let content = fs::read_to_string(&target_file).unwrap();
            assert_eq!(content, "export FOO=bar");

            // Check state was saved in new location
            let overlays_dir = repo.path().join(".repoverlay/overlays");
            assert!(overlays_dir.exists(), "overlays dir should exist");
        }

        #[test]
        fn applies_nested_files() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
            ]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
            );
            assert!(result.is_ok());

            assert!(repo.path().join(".envrc").exists());
            assert!(repo.path().join(".vscode/settings.json").exists());
        }

        #[test]
        fn applies_with_copy_mode() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                None,
                None,
                false,
            );
            assert!(result.is_ok());

            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists());
            assert!(
                !target_file.is_symlink(),
                ".envrc should NOT be a symlink in copy mode"
            );
        }

        #[test]
        fn updates_git_exclude_with_overlay_section() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
            )
            .unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            // New per-overlay marker format
            assert!(content.contains("# repoverlay:"));
            assert!(content.contains(" start"));
            assert!(content.contains(".envrc"));
            assert!(content.contains(" end"));
            // Managed section for .repoverlay
            assert!(content.contains("# repoverlay:managed start"));
            assert!(content.contains(".repoverlay"));
        }

        #[test]
        fn respects_path_mappings() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (
                    "repoverlay.toml",
                    r#"
[mappings]
".envrc" = ".env"
"#,
                ),
            ]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
            )
            .unwrap();

            assert!(
                !repo.path().join(".envrc").exists(),
                ".envrc should not exist"
            );
            assert!(repo.path().join(".env").exists(), ".env should exist");
        }

        #[test]
        fn uses_overlay_name_from_config() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (
                    "repoverlay.toml",
                    r#"
[overlay]
name = "my-custom-overlay"
"#,
                ),
            ]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
            )
            .unwrap();

            // State file should be named after the normalized overlay name
            let state_file = repo
                .path()
                .join(".repoverlay/overlays/my-custom-overlay.toml");
            assert!(state_file.exists(), "state file should use overlay name");
        }

        #[test]
        fn uses_name_override() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("custom-name".to_string()),
                None,
                false,
            )
            .unwrap();

            let state_file = repo.path().join(".repoverlay/overlays/custom-name.toml");
            assert!(state_file.exists(), "state file should use override name");
        }

        #[test]
        fn fails_on_non_git_directory() {
            let dir = TempDir::new().unwrap();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                dir.path(),
                false,
                None,
                None,
                false,
            );
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not a git repository")
            );
        }

        #[test]
        fn fails_on_duplicate_overlay_name() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                false,
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                false,
            );
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("already applied"));
        }

        #[test]
        fn fails_on_file_conflict_with_repo() {
            let repo = create_test_repo();
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let overlay = create_test_overlay(&[(".envrc", "new content")]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
            );
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Conflict"));
        }

        #[test]
        fn fails_on_file_conflict_between_overlays() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "first")]);
            let overlay2 = create_test_overlay(&[(".envrc", "second")]);

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("first".to_string()),
                None,
                false,
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("second".to_string()),
                None,
                false,
            );
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Conflict") || err.contains("already managed"));
        }

        #[test]
        fn fails_on_empty_overlay() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
            );
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No files found"));
        }

        #[test]
        fn fails_on_nonexistent_source() {
            let repo = create_test_repo();
            let result = apply_overlay("/nonexistent/path", repo.path(), false, None, None, false);
            assert!(result.is_err());
        }
    }

    // Integration tests for remove command
    mod remove {
        use super::*;

        #[test]
        fn removes_overlay_by_name() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (".vscode/settings.json", r#"{"key": "value"}"#),
            ]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test-overlay".to_string()),
                None,
                false,
            )
            .unwrap();
            remove_overlay(repo.path(), Some("test-overlay".to_string()), false).unwrap();

            assert!(!repo.path().join(".envrc").exists());
            assert!(!repo.path().join(".vscode/settings.json").exists());
            assert!(!repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn removes_all_overlays() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
                None,
                false,
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
                None,
                false,
            )
            .unwrap();

            assert!(repo.path().join(".envrc").exists());
            assert!(repo.path().join(".env.local").exists());

            remove_overlay(repo.path(), None, true).unwrap();

            assert!(!repo.path().join(".envrc").exists());
            assert!(!repo.path().join(".env.local").exists());
            assert!(!repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn removes_one_overlay_preserves_others() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
                None,
                false,
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
                None,
                false,
            )
            .unwrap();

            remove_overlay(repo.path(), Some("overlay-a".to_string()), false).unwrap();

            assert!(!repo.path().join(".envrc").exists());
            assert!(repo.path().join(".env.local").exists());
            assert!(repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn removes_empty_parent_directories() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".vscode/settings.json", r#"{"key": "value"}"#)]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test".to_string()),
                None,
                false,
            )
            .unwrap();
            assert!(repo.path().join(".vscode").exists());

            remove_overlay(repo.path(), Some("test".to_string()), false).unwrap();
            assert!(
                !repo.path().join(".vscode").exists(),
                ".vscode should be removed"
            );
        }

        #[test]
        fn preserves_non_empty_parent_directories() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".vscode/settings.json", r#"{"key": "value"}"#)]);

            // Create another file in .vscode that isn't from the overlay
            fs::create_dir_all(repo.path().join(".vscode")).unwrap();
            fs::write(repo.path().join(".vscode/other.json"), "{}").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test".to_string()),
                None,
                false,
            )
            .unwrap();
            remove_overlay(repo.path(), Some("test".to_string()), false).unwrap();

            assert!(
                repo.path().join(".vscode").exists(),
                ".vscode should remain"
            );
            assert!(repo.path().join(".vscode/other.json").exists());
        }

        #[test]
        fn cleans_git_exclude_for_overlay() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test".to_string()),
                None,
                false,
            )
            .unwrap();
            remove_overlay(repo.path(), Some("test".to_string()), false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(!content.contains("# repoverlay:test start"));
            assert!(!content.contains(".envrc"));
            assert!(!content.contains("# repoverlay:managed"));
        }

        #[test]
        fn fails_when_no_overlay_applied() {
            let repo = create_test_repo();

            let result = remove_overlay(repo.path(), Some("nonexistent".to_string()), false);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No overlay"));
        }

        #[test]
        fn fails_on_unknown_overlay_name() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("real-overlay".to_string()),
                None,
                false,
            )
            .unwrap();

            let result = remove_overlay(repo.path(), Some("fake-overlay".to_string()), false);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not found"));
        }

        #[test]
        fn handles_already_deleted_files() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test".to_string()),
                None,
                false,
            )
            .unwrap();

            // Manually delete the file
            fs::remove_file(repo.path().join(".envrc")).unwrap();

            // Remove should still succeed
            let result = remove_overlay(repo.path(), Some("test".to_string()), false);
            assert!(result.is_ok());
        }
    }

    // Integration tests for status command
    mod status {
        use super::*;

        #[test]
        fn shows_no_overlay_when_none_applied() {
            let repo = create_test_repo();
            let result = show_status(repo.path(), None);
            assert!(result.is_ok());
        }

        #[test]
        fn shows_status_when_overlay_applied() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test".to_string()),
                None,
                false,
            )
            .unwrap();

            let result = show_status(repo.path(), None);
            assert!(result.is_ok());
        }

        #[test]
        fn shows_status_for_multiple_overlays() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
                None,
                false,
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
                None,
                false,
            )
            .unwrap();

            let result = show_status(repo.path(), None);
            assert!(result.is_ok());
        }

        #[test]
        fn shows_status_for_specific_overlay() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
                None,
                false,
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
                None,
                false,
            )
            .unwrap();

            let result = show_status(repo.path(), Some("overlay-a".to_string()));
            assert!(result.is_ok());
        }

        #[test]
        fn fails_on_unknown_overlay_filter() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("real".to_string()),
                None,
                false,
            )
            .unwrap();

            let result = show_status(repo.path(), Some("fake".to_string()));
            assert!(result.is_err());
        }
    }

    // CLI integration tests using assert_cmd
    mod cli {
        use super::*;
        use assert_cmd::assert::OutputAssertExt;
        use predicates::prelude::*;

        fn repoverlay_cmd() -> std::process::Command {
            let path = std::env::var("CARGO_BIN_EXE_repoverlay").unwrap_or_else(|_| {
                env!("CARGO_MANIFEST_DIR").to_string() + "/target/debug/repoverlay"
            });
            std::process::Command::new(path)
        }

        #[test]
        fn help_displays() {
            repoverlay_cmd()
                .arg("--help")
                .assert()
                .success()
                .stdout(predicate::str::contains("Overlay config files"));
        }

        #[test]
        fn version_displays() {
            repoverlay_cmd()
                .arg("--version")
                .assert()
                .success()
                .stdout(predicate::str::contains("repoverlay"));
        }

        #[test]
        fn apply_help_displays() {
            repoverlay_cmd()
                .args(["apply", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Apply an overlay"));
        }

        #[test]
        fn remove_help_displays() {
            repoverlay_cmd()
                .args(["remove", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Remove"));
        }

        #[test]
        fn status_help_displays() {
            repoverlay_cmd()
                .args(["status", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("status"));
        }

        #[test]
        fn cache_help_displays() {
            repoverlay_cmd()
                .args(["cache", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("cache"));
        }

        #[test]
        fn restore_help_displays() {
            repoverlay_cmd()
                .args(["restore", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Restore"));
        }

        #[test]
        fn update_help_displays() {
            repoverlay_cmd()
                .args(["update", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Update"));
        }

        #[test]
        fn apply_requires_source_argument() {
            repoverlay_cmd()
                .arg("apply")
                .assert()
                .failure()
                .stderr(predicate::str::contains("required"));
        }

        #[test]
        fn apply_and_remove_workflow() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            // Apply with explicit name
            repoverlay_cmd()
                .args(["apply", overlay.path().to_str().unwrap()])
                .args(["--target", repo.path().to_str().unwrap()])
                .args(["--name", "test-overlay"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Applying"));

            assert!(repo.path().join(".envrc").exists());

            // Status
            repoverlay_cmd()
                .args(["status", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("Overlay Status"));

            // Remove by name
            repoverlay_cmd()
                .args([
                    "remove",
                    "test-overlay",
                    "--target",
                    repo.path().to_str().unwrap(),
                ])
                .assert()
                .success()
                .stdout(predicate::str::contains("Removing"));

            assert!(!repo.path().join(".envrc").exists());
        }

        #[test]
        fn apply_and_remove_all_workflow() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            // Apply
            repoverlay_cmd()
                .args(["apply", overlay.path().to_str().unwrap()])
                .args(["--target", repo.path().to_str().unwrap()])
                .assert()
                .success();

            assert!(repo.path().join(".envrc").exists());

            // Remove with --all
            repoverlay_cmd()
                .args(["remove", "--all", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("Removed all"));

            assert!(!repo.path().join(".envrc").exists());
        }

        #[test]
        fn apply_with_copy_flag() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            repoverlay_cmd()
                .args(["apply", overlay.path().to_str().unwrap()])
                .args(["--target", repo.path().to_str().unwrap()])
                .arg("--copy")
                .assert()
                .success();

            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists());
            assert!(!target_file.is_symlink());
        }

        #[test]
        fn status_when_no_overlay() {
            let repo = create_test_repo();

            repoverlay_cmd()
                .args(["status", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("No overlay"));
        }

        #[test]
        fn remove_when_no_overlay() {
            let repo = create_test_repo();

            // Use --all to avoid interactive prompt
            repoverlay_cmd()
                .args(["remove", "--all", "--target", repo.path().to_str().unwrap()])
                .assert()
                .failure()
                .stderr(predicate::str::contains("No overlay"));
        }

        #[test]
        fn cache_list_empty() {
            repoverlay_cmd().args(["cache", "list"]).assert().success();
        }

        #[test]
        fn cache_path_shows_location() {
            repoverlay_cmd()
                .args(["cache", "path"])
                .assert()
                .success()
                .stdout(predicate::str::contains("repoverlay"));
        }
    }

    // Integration tests for create command
    mod create {
        use super::*;

        #[test]
        fn creates_overlay_with_single_file() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            // Create a file in the source repo
            fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("test-overlay")),
                &[PathBuf::from(".envrc")],
                None,
                false,
                false,
            );
            assert!(result.is_ok(), "create_overlay failed: {:?}", result);

            // Check file was copied
            let overlay_file = output.path().join("test-overlay/.envrc");
            assert!(overlay_file.exists(), ".envrc should exist in overlay");

            // Check content is correct
            let content = fs::read_to_string(&overlay_file).unwrap();
            assert_eq!(content, "export FOO=bar");

            // Check repoverlay.toml was generated
            let config_file = output.path().join("test-overlay/repoverlay.toml");
            assert!(config_file.exists(), "repoverlay.toml should exist");
        }

        #[test]
        fn creates_overlay_with_directory() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            // Create a directory with files
            fs::create_dir_all(source.path().join(".claude")).unwrap();
            fs::write(
                source.path().join(".claude/settings.json"),
                r#"{"key": "value"}"#,
            )
            .unwrap();
            fs::write(source.path().join(".claude/commands.md"), "# Commands").unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("test-overlay")),
                &[PathBuf::from(".claude")],
                None,
                false,
                false,
            );
            assert!(result.is_ok(), "create_overlay failed: {:?}", result);

            // Check directory was copied
            let overlay_dir = output.path().join("test-overlay/.claude");
            assert!(overlay_dir.exists(), ".claude directory should exist");
            assert!(overlay_dir.join("settings.json").exists());
            assert!(overlay_dir.join("commands.md").exists());
        }

        #[test]
        fn generates_repoverlay_toml_with_name() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("test-overlay")),
                &[PathBuf::from(".envrc")],
                Some("my-custom-name".to_string()),
                false,
                false,
            );
            assert!(result.is_ok());

            let config_content =
                fs::read_to_string(output.path().join("test-overlay/repoverlay.toml")).unwrap();
            assert!(config_content.contains("my-custom-name"));
        }

        #[test]
        fn dry_run_does_not_create_files() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("test-overlay")),
                &[PathBuf::from(".envrc")],
                None,
                true, // dry_run
                false,
            );
            assert!(result.is_ok());

            // Check no files were created
            assert!(!output.path().join("test-overlay").exists());
        }

        #[test]
        fn fails_when_no_files_specified() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("test-overlay")),
                &[], // empty include
                None,
                false,
                false,
            );
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("No files specified")
            );
        }

        #[test]
        fn fails_on_nonexistent_include_path() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("test-overlay")),
                &[PathBuf::from("nonexistent.txt")],
                None,
                false,
                false,
            );
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("does not exist"));
        }

        #[test]
        fn fails_on_non_git_source() {
            let source = TempDir::new().unwrap(); // Not a git repo
            let output = TempDir::new().unwrap();

            fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("test-overlay")),
                &[PathBuf::from(".envrc")],
                None,
                false,
                false,
            );
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not a git repository")
            );
        }
    }

    // Integration tests for switch command
    mod switch {
        use super::*;

        #[test]
        fn removes_existing_overlays_before_applying() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            // Apply first overlay
            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("first-overlay".to_string()),
                None,
                false,
            )
            .unwrap();

            // Verify first overlay is applied
            assert!(repo.path().join(".envrc").exists());

            // Switch to second overlay
            let result = switch_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("second-overlay".to_string()),
                None,
            );
            assert!(result.is_ok(), "switch_overlay failed: {:?}", result);

            // Verify first overlay is removed
            assert!(
                !repo.path().join(".envrc").exists(),
                ".envrc should be removed"
            );

            // Verify second overlay is applied
            assert!(
                repo.path().join(".env.local").exists(),
                ".env.local should exist"
            );
        }

        #[test]
        fn applies_to_empty_repo() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = switch_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("new-overlay".to_string()),
                None,
            );
            assert!(result.is_ok());

            assert!(repo.path().join(".envrc").exists());
        }

        #[test]
        fn fails_on_non_git_target() {
            let target = TempDir::new().unwrap(); // Not a git repo
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = switch_overlay(
                overlay.path().to_str().unwrap(),
                target.path(),
                false,
                None,
                None,
            );
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not a git repository")
            );
        }

        #[test]
        fn removes_multiple_overlays_before_applying() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);
            let overlay3 = create_test_overlay(&[(".env.prod", "PROD=true")]);

            // Apply first two overlays
            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
                None,
                false,
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
                None,
                false,
            )
            .unwrap();

            // Verify both overlays are applied
            assert!(repo.path().join(".envrc").exists());
            assert!(repo.path().join(".env.local").exists());

            // Switch to third overlay
            switch_overlay(
                overlay3.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("overlay-c".to_string()),
                None,
            )
            .unwrap();

            // Verify old overlays are removed
            assert!(!repo.path().join(".envrc").exists());
            assert!(!repo.path().join(".env.local").exists());

            // Verify new overlay is applied
            assert!(repo.path().join(".env.prod").exists());
        }
    }
}
