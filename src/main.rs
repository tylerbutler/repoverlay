//! CLI entry point for repoverlay.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use repoverlay::{
    CONFIG_FILE, CacheManager, OVERLAYS_DIR, STATE_DIR, apply_overlay, canonicalize_path,
    list_applied_overlays, parse_github_owner_repo, remove_overlay,
    remove_single_overlay, restore_overlays, show_status, switch_overlay, update_overlays,
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
        ///   <https://github.com/owner/repo>
        ///   <https://github.com/owner/repo/tree/main/overlays/rust>
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
    ///
    /// Examples:
    ///   repoverlay create my-overlay          # Detects org/repo from git remote
    ///   repoverlay create org/repo/my-overlay # Explicit target
    ///   repoverlay create --local ./output    # Write to local directory only
    Create {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit target
        /// Omit to use interactive mode or --local for local output
        name: Option<String>,

        /// Include specific files or directories (can be specified multiple times)
        #[arg(short, long)]
        include: Vec<PathBuf>,

        /// Write to local directory instead of overlay repo
        #[arg(short, long, conflicts_with = "name")]
        local: Option<PathBuf>,

        /// Source repository to extract files from (defaults to current directory)
        #[arg(short, long)]
        source: Option<PathBuf>,

        /// Show what would be created without creating files
        #[arg(long)]
        dry_run: bool,

        /// Skip interactive prompts, use defaults
        #[arg(short = 'y', long)]
        yes: bool,

        /// Force overwrite if overlay already exists
        #[arg(short, long)]
        force: bool,
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

    /// Initialize overlay repository configuration
    #[command(name = "init-repo")]
    InitRepo {
        /// URL of the overlay repository (e.g., `https://github.com/user/repo-overlays`)
        url: String,

        /// Skip cloning the repository
        #[arg(long)]
        no_clone: bool,
    },

    /// List available overlays from the overlay repository
    List {
        /// Filter by target repository (format: org/repo)
        #[arg(short, long)]
        target: Option<String>,

        /// Update overlay repo before listing
        #[arg(long)]
        update: bool,
    },

    /// Sync changes from an applied overlay back to the overlay repo
    ///
    /// Examples:
    ///   repoverlay sync my-overlay          # Detects org/repo from git remote
    ///   repoverlay sync org/repo/my-overlay # Explicit target
    Sync {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit values
        name: String,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would be synced without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Push all pending commits in the overlay repo to remote
    Push,

    /// Publish an overlay to the overlay repository
    #[command(hide = true)] // Hidden: deprecated, use create instead
    Publish {
        /// Path to the overlay source directory
        source: PathBuf,

        /// Target repository (format: org/repo)
        /// Auto-detected from current git remote if not specified
        #[arg(short, long)]
        target: Option<String>,

        /// Overlay name (defaults from repoverlay.ccl or directory name)
        #[arg(short, long)]
        name: Option<String>,

        /// Commit message
        #[arg(short, long)]
        message: Option<String>,

        /// Skip push to remote (just commit locally)
        #[arg(long)]
        no_push: bool,

        /// Show what would be published without making changes
        #[arg(long)]
        dry_run: bool,
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
        Commands::Create {
            name,
            include,
            local,
            source,
            dry_run,
            yes,
            force,
        } => {
            let source = source.unwrap_or_else(|| PathBuf::from("."));
            create_overlay_command(&source, name, local, &include, dry_run, yes, force)?;
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
        Commands::InitRepo { url, no_clone } => {
            init_repo(&url, no_clone)?;
        }
        Commands::List { target, update } => {
            list_overlays(target.as_deref(), update)?;
        }
        Commands::Sync {
            name,
            target,
            dry_run,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            sync_overlay(&name, &target, dry_run)?;
        }
        Commands::Push => {
            push_overlay_repo()?;
        }
        Commands::Publish {
            source,
            target,
            name,
            message,
            no_push,
            dry_run,
        } => {
            eprintln!(
                "{} 'repoverlay publish' is deprecated and will be removed in a future version.",
                "Warning:".yellow().bold()
            );
            eprintln!("         Use 'repoverlay create <name>' instead to create overlays in the overlay repo.");
            eprintln!();

            publish_overlay(
                &source,
                target.as_deref(),
                name.as_deref(),
                message.as_deref(),
                no_push,
                dry_run,
            )?;
        }
    }

    Ok(())
}

/// Handle remove command with interactive selection support.
fn handle_remove(target: &std::path::Path, name: Option<String>, remove_all: bool) -> Result<()> {
    if remove_all || name.is_some() {
        return remove_overlay(target, name, remove_all);
    }

    // Interactive selection
    let target = canonicalize_path(target, "Target directory")?;
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

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

/// Initialize overlay repository configuration.
fn init_repo(url: &str, no_clone: bool) -> Result<()> {
    use repoverlay::config::{
        OverlayRepoConfig, global_config_path, save_global_config_with_comments,
    };
    use repoverlay::overlay_repo::OverlayRepoManager;

    // Validate URL looks reasonable
    if !url.starts_with("https://") && !url.starts_with("git@") {
        bail!(
            "Invalid repository URL. Use HTTPS (https://github.com/...) or SSH (git@github.com:...) format."
        );
    }

    let config = OverlayRepoConfig {
        url: url.to_string(),
        local_path: None,
    };

    // Save configuration
    save_global_config_with_comments(&config)?;
    println!(
        "{} Configuration saved to: {}",
        "✓".green().bold(),
        global_config_path()?.display()
    );

    if no_clone {
        println!(
            "\n{} Skipped cloning. Run 'repoverlay list' to clone and see available overlays.",
            "Note:".yellow()
        );
        return Ok(());
    }

    // Clone the repository
    println!("{} overlay repository...", "Cloning".blue().bold());
    let manager = OverlayRepoManager::new(config)?;
    manager.ensure_cloned()?;

    // List available overlays
    let overlays = manager.list_overlays()?;
    println!(
        "\n{} Overlay repository initialized with {} overlay(s) available.",
        "✓".green().bold(),
        overlays.len()
    );

    if !overlays.is_empty() {
        println!("\nRun 'repoverlay list' to see available overlays.");
    }

    Ok(())
}

/// List available overlays from the overlay repository.
fn list_overlays(target_filter: Option<&str>, update: bool) -> Result<()> {
    use repoverlay::config::load_config;
    use repoverlay::overlay_repo::OverlayRepoManager;

    let config = load_config(None)?;

    let overlay_config = config.overlay_repo.ok_or_else(|| {
        anyhow::anyhow!(
            "Overlay repository not configured.\n\n\
             Run 'repoverlay init-repo <url>' to set up an overlay repository.\n\
             Example: repoverlay init-repo https://github.com/tylerbutler/repo-overlays"
        )
    })?;

    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    if update {
        println!("{} overlay repository...", "Updating".blue().bold());
        manager.pull()?;
    }

    let overlays = if let Some(filter) = target_filter {
        // Parse org/repo filter
        let parts: Vec<&str> = filter.split('/').collect();
        if parts.len() != 2 {
            bail!("Invalid target filter format. Use: org/repo");
        }
        manager.list_overlays_for_repo(parts[0], parts[1])?
    } else {
        manager.list_overlays()?
    };

    if overlays.is_empty() {
        if let Some(filter) = target_filter {
            println!("{} No overlays found for {}.", "Status:".bold(), filter);
        } else {
            println!("{} No overlays found in repository.", "Status:".bold());
        }
        return Ok(());
    }

    println!("{}\n", "Available overlays:".bold());

    // Group by org/repo
    let mut current_group: Option<(String, String)> = None;
    for overlay in &overlays {
        let group = (overlay.org.clone(), overlay.repo.clone());
        if current_group.as_ref() != Some(&group) {
            if current_group.is_some() {
                println!();
            }
            println!("{}{}{}:", overlay.org.cyan(), "/".dimmed(), overlay.repo);
            current_group = Some(group);
        }
        let config_marker = if overlay.has_config {
            ""
        } else {
            " (no config)"
        };
        println!("  - {}{}", overlay.name, config_marker.dimmed());
    }

    println!(
        "\nTo apply an overlay: repoverlay apply {}",
        "<org>/<repo>/<name>".dimmed()
    );

    Ok(())
}

/// Publish an overlay to the overlay repository.
fn publish_overlay(
    source: &std::path::Path,
    target: Option<&str>,
    name: Option<&str>,
    message: Option<&str>,
    no_push: bool,
    dry_run: bool,
) -> Result<()> {
    use repoverlay::config::load_config;
    use repoverlay::overlay_repo::OverlayRepoManager;
    use repoverlay::state;

    // Validate source exists
    let source = canonicalize_path(source, "Overlay source")?;
    if !source.is_dir() {
        bail!("Source must be a directory: {}", source.display());
    }

    // Load config
    let config = load_config(None)?;
    let overlay_config = config.overlay_repo.ok_or_else(|| {
        anyhow::anyhow!(
            "Overlay repository not configured.\n\n\
             Run 'repoverlay init-repo <url>' to set up an overlay repository."
        )
    })?;

    // Determine target org/repo
    let (org, repo) = if let Some(t) = target {
        let parts: Vec<&str> = t.split('/').collect();
        if parts.len() != 2 {
            bail!("Invalid target format. Use: org/repo");
        }
        (parts[0].to_string(), parts[1].to_string())
    } else {
        // Try to detect from current git remote
        detect_target_repo(&source)?
    };

    // Determine overlay name
    let overlay_name = if let Some(n) = name {
        n.to_string()
    } else {
        // Try to read from repoverlay.ccl
        let config_path = source.join(CONFIG_FILE);

        if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            let cfg: state::OverlayConfig =
                sickle::from_str(&content).with_context(|| "Failed to parse repoverlay.ccl")?;
            cfg.overlay
                .name
                .unwrap_or_else(|| source.file_name().unwrap().to_string_lossy().to_string())
        } else {
            source.file_name().unwrap().to_string_lossy().to_string()
        }
    };

    println!("{} Publishing overlay:", "Publish".blue().bold());
    println!("  Source:  {}", source.display());
    println!("  Target:  {}/{}", org, repo);
    println!("  Name:    {}", overlay_name);

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        println!("\nWould publish to: {}/{}/{}", org, repo, overlay_name);
        return Ok(());
    }

    // Create manager and ensure cloned
    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    // Pull latest
    println!("\n{} latest changes...", "Pulling".blue().bold());
    manager.pull()?;

    // Stage the overlay
    println!("{} overlay files...", "Copying".blue().bold());
    let dest = manager.stage_overlay(&org, &repo, &overlay_name, &source)?;
    println!("  Copied to: {}", dest.display());

    // Check if there are changes
    if !manager.has_staged_changes()? {
        println!("\n{} No changes to publish.", "Note:".yellow());
        return Ok(());
    }

    // Commit
    let commit_msg = message
        .unwrap_or(&format!(
            "Update overlay: {}/{}/{}",
            org, repo, overlay_name
        ))
        .to_string();

    println!("{} changes...", "Committing".blue().bold());
    manager.commit(&commit_msg)?;

    // Push
    if no_push {
        println!(
            "\n{} Changes committed but not pushed (--no-push).",
            "Note:".yellow()
        );
    } else {
        println!("{} to remote...", "Pushing".blue().bold());
        manager.push()?;
        println!(
            "\n{} Overlay published: {}/{}/{}",
            "✓".green().bold(),
            org,
            repo,
            overlay_name
        );
    }

    println!(
        "\nTo apply: repoverlay apply {}/{}/{}",
        org, repo, overlay_name
    );

    Ok(())
}

/// Detect org/repo from git remote origin.
fn detect_target_repo(path: &std::path::Path) -> Result<(String, String)> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(path)
        .output()
        .context("Failed to get git remote")?;

    if !output.status.success() {
        bail!(
            "Could not detect target repository from git remote.\n\
             Please specify explicitly: repoverlay create org/repo/name"
        );
    }

    let url = String::from_utf8(output.stdout)?.trim().to_string();
    parse_github_owner_repo(&url)
}

/// Parse an overlay name argument.
///
/// Returns (org, repo, name) tuple.
/// - If the argument contains 2 slashes, parses as org/repo/name
/// - If no slashes, detects org/repo from git remote
/// - If 1 slash, returns an error (invalid format)
fn parse_overlay_name_arg(
    name_arg: &str,
    source_path: &std::path::Path,
) -> Result<(String, String, String)> {
    let slash_count = name_arg.chars().filter(|c| *c == '/').count();

    match slash_count {
        0 => {
            // Short form: just the overlay name
            let (org, repo) = detect_target_repo(source_path)?;
            Ok((org, repo, name_arg.to_string()))
        }
        2 => {
            // Full form: org/repo/name
            let parts: Vec<&str> = name_arg.split('/').collect();
            if parts.iter().any(|p| p.is_empty()) {
                bail!(
                    "Invalid overlay path format: {}\n\n\
                     Use one of:\n  \
                     - my-overlay (detects org/repo from git remote)\n  \
                     - org/repo/my-overlay (explicit)",
                    name_arg
                );
            }
            Ok((
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
            ))
        }
        _ => {
            bail!(
                "Invalid overlay path format: {}\n\n\
                 Use one of:\n  \
                 - my-overlay (detects org/repo from git remote)\n  \
                 - org/repo/my-overlay (explicit)",
                name_arg
            );
        }
    }
}

/// Handle the create command with the new argument structure.
///
/// This function handles:
/// - `create <name>` - create in overlay repo, auto-detect org/repo
/// - `create org/repo/name` - create in overlay repo at explicit path
/// - `create --local ./output` - create in local directory only
fn create_overlay_command(
    source: &std::path::Path,
    name_arg: Option<String>,
    local: Option<PathBuf>,
    include: &[PathBuf],
    dry_run: bool,
    yes: bool,
    force: bool,
) -> Result<()> {
    use repoverlay::config::load_config;
    use repoverlay::overlay_repo::OverlayRepoManager;

    // Validate source is a git repo
    if !source.join(".git").exists() {
        bail!(
            "Source directory is not a git repository: {}",
            source.display()
        );
    }

    // Handle --local mode (write to local directory)
    if let Some(local_path) = local {
        // Use existing create_overlay function for local mode
        return repoverlay::create_overlay(
            source,
            Some(local_path),
            include,
            None, // name derived from directory
            dry_run,
            yes,
        );
    }

    // For overlay repo mode, we need the name argument
    let name_arg = name_arg.ok_or_else(|| {
        anyhow::anyhow!(
            "Missing overlay name.\n\n\
             Usage:\n  \
             repoverlay create my-overlay          # Detects org/repo from git remote\n  \
             repoverlay create org/repo/my-overlay # Explicit target\n  \
             repoverlay create --local ./output    # Write to local directory"
        )
    })?;

    // Parse the name argument
    let (org, repo, overlay_name) = parse_overlay_name_arg(&name_arg, source)?;

    // Load overlay repo config
    let config = load_config(None)?;
    let overlay_config = config.overlay_repo.ok_or_else(|| {
        anyhow::anyhow!(
            "Overlay repository not configured.\n\n\
             Run 'repoverlay init-repo <url>' to set up an overlay repository.\n\
             Or use --local to write to a local directory."
        )
    })?;

    // Create manager and ensure cloned
    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    // Determine output path in overlay repo
    let output_path = manager.path().join(&org).join(&repo).join(&overlay_name);

    // Check if overlay already exists
    if output_path.exists() && !force {
        bail!(
            "Overlay '{}/{}/{}' already exists.\n\n\
             To update an applied overlay, use: repoverlay sync {}\n\
             To overwrite, use: repoverlay create {} --force",
            org,
            repo,
            overlay_name,
            overlay_name,
            name_arg
        );
    }

    println!(
        "{} Creating overlay: {}/{}/{}",
        "Create".blue().bold(),
        org,
        repo,
        overlay_name
    );

    if dry_run {
        println!("  Source:  {}", source.display());
        println!("  Target:  {}", output_path.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        return Ok(());
    }

    // If includes not specified, use discovery/interactive mode
    if include.is_empty() {
        // Use the existing discovery logic from create_overlay
        return repoverlay::create_overlay(
            source,
            Some(output_path),
            include,
            Some(overlay_name.clone()),
            dry_run,
            yes,
        )
        .and_then(|_| {
            // Auto-commit after creating
            auto_commit_overlay(&manager, &org, &repo, &overlay_name, true)
        });
    }

    // Validate all include paths exist
    for path in include {
        let full_path = source.join(path);
        if !full_path.exists() {
            bail!("Include path does not exist: {}", path.display());
        }
    }

    // If force and exists, remove existing first
    if output_path.exists() && force {
        fs::remove_dir_all(&output_path)?;
    }

    // Copy files and create overlay
    let copied_files =
        repoverlay::copy_files_to_overlay(source, &output_path, include)?;

    // Generate config
    fs::write(
        output_path.join("repoverlay.ccl"),
        repoverlay::generate_overlay_config(&overlay_name),
    )?;

    repoverlay::print_overlay_created(&output_path, &copied_files);

    // Auto-commit
    auto_commit_overlay(&manager, &org, &repo, &overlay_name, true)?;

    Ok(())
}

/// Auto-commit changes to an overlay in the overlay repo.
fn auto_commit_overlay(
    manager: &repoverlay::overlay_repo::OverlayRepoManager,
    org: &str,
    repo: &str,
    name: &str,
    is_new: bool,
) -> Result<()> {
    // Check if there are changes to commit
    if !manager.has_staged_changes()? {
        // Stage all changes
        use std::process::Command;
        let output = Command::new("git")
            .args(["add", "."])
            .current_dir(manager.path())
            .output()
            .context("Failed to stage changes")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to stage changes: {}", stderr.trim());
        }
    }

    // Check again if there are staged changes
    if !manager.has_staged_changes()? {
        println!(
            "{} No changes to commit.",
            "Note:".yellow()
        );
        return Ok(());
    }

    let action = if is_new { "Add" } else { "Update" };
    let commit_msg = format!("{} overlay: {}/{}/{}", action, org, repo, name);

    println!("{} changes...", "Committing".blue().bold());
    manager.commit(&commit_msg)?;

    println!(
        "\n{} Overlay created: {}/{}/{}",
        "✓".green().bold(),
        org,
        repo,
        name
    );
    println!(
        "\nTo push to remote: {}",
        "repoverlay push".cyan()
    );
    println!(
        "To apply: repoverlay apply {}/{}/{}",
        org, repo, name
    );

    Ok(())
}

/// Sync changes from an applied overlay back to the overlay repo.
///
/// This copies changed files from the target repository back to the overlay repo
/// and auto-commits the changes.
fn sync_overlay(name_arg: &str, target: &std::path::Path, dry_run: bool) -> Result<()> {
    use repoverlay::config::load_config;
    use repoverlay::overlay_repo::OverlayRepoManager;
    use repoverlay::{load_overlay_state, normalize_overlay_name};

    // Validate target is a git repo
    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        bail!(
            "Target directory is not a git repository: {}",
            target.display()
        );
    }

    // Parse the name argument to get org/repo/name
    let (org, repo, overlay_name) = parse_overlay_name_arg(name_arg, &target)?;

    // Verify the overlay is currently applied
    let normalized_name = normalize_overlay_name(&overlay_name)?;
    let applied_overlays = list_applied_overlays(&target)?;

    if !applied_overlays.contains(&normalized_name) {
        bail!(
            "Overlay '{}' is not currently applied.\n\n\
             To apply it first: repoverlay apply {}/{}/{}",
            overlay_name,
            org,
            repo,
            overlay_name
        );
    }

    // Load overlay state to get file mappings
    let state = load_overlay_state(&target, &normalized_name)?;

    // Load overlay repo config
    let config = load_config(None)?;
    let overlay_config = config.overlay_repo.ok_or_else(|| {
        anyhow::anyhow!(
            "Overlay repository not configured.\n\n\
             Run 'repoverlay init-repo <url>' to set up an overlay repository."
        )
    })?;

    // Create manager and ensure cloned
    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    // Get the overlay path in the overlay repo
    let overlay_repo_path = manager.path().join(&org).join(&repo).join(&overlay_name);

    if !overlay_repo_path.exists() {
        bail!(
            "Overlay '{}/{}/{}' does not exist in overlay repo.\n\n\
             Did you mean to use 'repoverlay create {}' instead?",
            org,
            repo,
            overlay_name,
            name_arg
        );
    }

    println!(
        "{} overlay: {}/{}/{}",
        "Syncing".blue().bold(),
        org,
        repo,
        overlay_name
    );

    if dry_run {
        println!("  Target: {}", target.display());
        println!("  Repo:   {}", overlay_repo_path.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());

        // Show what would be synced
        println!("\nFiles that would be synced:");
        for entry in state.file_entries() {
            let target_file = target.join(&entry.target);

            if target_file.exists() {
                println!("  {} {} -> {}", "→".cyan(), entry.target.display(), entry.source.display());
            }
        }

        return Ok(());
    }

    // Copy files from target back to overlay repo
    let mut synced_count = 0;
    for entry in state.file_entries() {
        let target_file = target.join(&entry.target);
        let overlay_file = overlay_repo_path.join(&entry.source);

        if target_file.exists() {
            // Ensure parent directory exists
            if let Some(parent) = overlay_file.parent() {
                fs::create_dir_all(parent)?;
            }

            // Copy file
            fs::copy(&target_file, &overlay_file).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    target_file.display(),
                    overlay_file.display()
                )
            })?;

            println!("  {} {}", "→".green(), entry.source.display());
            synced_count += 1;
        }
    }

    if synced_count == 0 {
        println!("{} No files to sync.", "Note:".yellow());
        return Ok(());
    }

    // Auto-commit
    auto_commit_overlay(&manager, &org, &repo, &overlay_name, false)?;

    Ok(())
}

/// Push all pending commits in the overlay repo to remote.
fn push_overlay_repo() -> Result<()> {
    use repoverlay::config::load_config;
    use repoverlay::overlay_repo::OverlayRepoManager;

    // Load overlay repo config
    let config = load_config(None)?;
    let overlay_config = config.overlay_repo.ok_or_else(|| {
        anyhow::anyhow!(
            "Overlay repository not configured.\n\n\
             Run 'repoverlay init-repo <url>' to set up an overlay repository."
        )
    })?;

    // Create manager and ensure cloned
    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    println!("{} to remote...", "Pushing".blue().bold());
    manager.push()?;

    println!("{} Pushed successfully.", "✓".green().bold());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use repoverlay::create_overlay;
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
        use repoverlay::remove_overlay_section;

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
                    "repoverlay.ccl",
                    r#"mappings =
  .envrc = .env
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
                    "repoverlay.ccl",
                    r#"overlay =
  name = my-custom-overlay
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
                .join(".repoverlay/overlays/my-custom-overlay.ccl");
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

            let state_file = repo.path().join(".repoverlay/overlays/custom-name.ccl");
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
        use assert_cmd::Command;
        use predicates::prelude::*;

        fn repoverlay_cmd() -> Command {
            // Using deprecated cargo_bin because tests are in src/main.rs (not tests/ dir).
            // The cargo_bin! macro requires CARGO_BIN_EXE_* which isn't set during clippy.
            #[allow(deprecated)]
            Command::cargo_bin("repoverlay").expect("Failed to find repoverlay binary")
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

            // Check repoverlay.ccl was generated
            let config_file = output.path().join("test-overlay/repoverlay.ccl");
            assert!(config_file.exists(), "repoverlay.ccl should exist");
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
        fn generates_repoverlay_ccl_with_name() {
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
                fs::read_to_string(output.path().join("test-overlay/repoverlay.ccl")).unwrap();
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
        fn fails_when_no_files_specified_and_none_discovered() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            // Empty repo with no discoverable files
            let result = create_overlay(
                source.path(),
                Some(output.path().join("test-overlay")),
                &[], // empty include
                None,
                false,
                false,
            );
            assert!(result.is_err());
            // Error message now mentions discovery
            let err_msg = result.unwrap_err().to_string();
            assert!(
                err_msg.contains("No files") || err_msg.contains("--include"),
                "Expected error about no files, got: {}",
                err_msg
            );
        }

        #[test]
        fn dry_run_shows_discovered_files() {
            let source = create_test_repo();

            // Create some AI config files to be discovered
            fs::create_dir_all(source.path().join(".claude")).unwrap();
            fs::write(source.path().join(".claude/settings.json"), "{}").unwrap();
            fs::write(source.path().join("CLAUDE.md"), "# Claude").unwrap();

            // Dry run without --include should show discovered files
            let result = create_overlay(
                source.path(),
                None,
                &[], // no explicit includes
                None,
                true, // dry_run
                false,
            );
            // Should succeed (just prints discovery info)
            assert!(result.is_ok());
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
