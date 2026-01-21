//! repoverlay CLI - Overlay config files into git repositories without committing them.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::io::{self, Write};
use std::path::PathBuf;

use repoverlay::{
    apply_overlay, remove_overlay, show_status,
    cache::CacheManager,
    github::{GitHubSource, GitRef},
    state::{
        list_applied_overlays, load_external_states, load_overlay_state, normalize_overlay_name,
        OverlaySource, STATE_DIR, OVERLAYS_DIR,
    },
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
            handle_remove(&target, name, all)?;
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
        Commands::Cache { command } => {
            handle_cache_command(command)?;
        }
    }

    Ok(())
}

/// Handle the remove command with interactive selection support.
fn handle_remove(target: &PathBuf, name: Option<String>, remove_all: bool) -> Result<()> {
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

    if remove_all || name.is_some() {
        // Non-interactive mode - delegate to library
        return remove_overlay(&target, name, remove_all);
    }

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
            remove_overlay(&target, None, true)?;
        } else if selection >= 1 && selection <= applied_overlays.len() {
            let overlay_name = &applied_overlays[selection - 1];
            remove_overlay(&target, Some(overlay_name.clone()), false)?;
        } else {
            bail!("Invalid selection: {}", selection);
        }
    } else if input.eq_ignore_ascii_case("all") {
        remove_overlay(&target, None, true)?;
    } else {
        bail!("Invalid selection: {}", input);
    }

    Ok(())
}

fn restore_overlays(target: &PathBuf, dry_run: bool) -> Result<()> {
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
        if let Err(e) = apply_overlay(
            &source_str,
            &target,
            false, // Use symlinks by default
            Some(state.meta.name.clone()),
            ref_override,
            true, // Update cache
        ) {
            eprintln!(
                "  {} Failed to restore '{}': {}",
                "Error:".red(),
                state.meta.name,
                e
            );
        }
    }

    Ok(())
}

fn update_overlays(target: &PathBuf, name: Option<String>, dry_run: bool) -> Result<()> {
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
                git_ref: GitRef::from_str(git_ref),
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
            remove_overlay(&target, Some(normalized_name.clone()), false)?;

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
