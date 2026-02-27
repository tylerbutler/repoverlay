//! CLI implementation for repoverlay.

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::LazyLock;

use crate::{
    CONFIG_FILE, CacheManager, ConflictStrategy, OVERLAYS_DIR, OverlayName, ResolvedSource,
    STATE_DIR, apply_multiple_overlays, apply_overlay, canonicalize_path, config,
    get_cached_repo_commit, list_applied_overlays, list_overlays_from_cached_repo,
    parse_github_owner_repo, remove_overlay, remove_single_overlay, restore_overlays,
    selection::is_interactive, show_status, show_status_json, status_has_overlays, switch_overlay,
    update_overlays, validate_git_repo,
};

/// Build version string with git info for local builds
static VERSION: LazyLock<String> = LazyLock::new(|| {
    let version = env!("CARGO_PKG_VERSION");
    let is_ci = option_env!("REPOVERLAY_CI_BUILD") == Some("true");

    // CI builds just show the version
    if is_ci {
        return version.to_string();
    }

    // Local builds show: {version}-{branch} ({sha}) or {version}-{branch} ({sha}) (dirty)
    let sha = option_env!("VERGEN_GIT_SHA").map(|s| &s[..7.min(s.len())]);
    let branch = option_env!("VERGEN_GIT_BRANCH");
    let dirty = option_env!("VERGEN_GIT_DIRTY") == Some("true");

    match (sha, branch, dirty) {
        (Some(sha), Some(branch), true) => format!("{version}-{branch} ({sha}) (dirty)"),
        (Some(sha), Some(branch), false) => format!("{version}-{branch} ({sha})"),
        (Some(sha), None, true) => format!("{version} ({sha}) (dirty)"),
        (Some(sha), None, false) => format!("{version} ({sha})"),
        (None, _, _) => version.to_string(),
    }
});

fn version_string() -> &'static str {
    &VERSION
}

/// Check for updates and print a notification if a new version is available.
///
/// Uses tiny-update-check to query crates.io with caching (24 hours).
fn check_for_updates() {
    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");

    if let Ok(Some(update)) = tiny_update_check::check(name, version) {
        eprintln!();
        eprintln!(
            "{} A new version of {} is available: {} → {}",
            "Update available:".yellow().bold(),
            name,
            update.current,
            update.latest.green().bold()
        );
        eprintln!(
            "                  {}",
            "https://github.com/tylerbutler/repoverlay/releases".cyan()
        );
    }
}

/// Overlay config files into git repositories without committing them
#[derive(Parser)]
#[command(name = "repoverlay")]
#[command(version = version_string(), about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Print help in markdown format (for documentation generation)
    #[arg(long, hide = true)]
    markdown_help: bool,
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
        #[arg(short, long, value_name = "REF", help_heading = "GitHub Options")]
        r#ref: Option<String>,

        /// Force update the cached repository before applying (GitHub sources only)
        #[arg(long, help_heading = "GitHub Options")]
        update: bool,

        /// Overwrite existing files and re-apply same-name overlays
        #[arg(long, visible_alias = "overwrite", conflicts_with_all = ["skip_conflicts", "interactive"])]
        force: bool,

        /// Skip conflicting files silently, continue with non-conflicting files
        #[arg(long, conflicts_with_all = ["force", "interactive"])]
        skip_conflicts: bool,

        /// Prompt interactively for each conflict (overwrite, skip, diff, or abort)
        #[arg(short, long, conflicts_with_all = ["force", "skip_conflicts"])]
        interactive: bool,

        /// Deep merge conflicting JSON files instead of failing
        #[arg(long, env = "REPOVERLAY_MERGE")]
        merge: bool,

        /// Use a specific overlay source instead of priority order (multi-source configs only)
        #[arg(long = "from", value_name = "SOURCE", help_heading = "GitHub Options")]
        from_source: Option<String>,

        /// Show what would be applied without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove applied overlay(s)
    Remove {
        /// Name of the overlay to remove
        name: Option<String>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Remove all applied overlays
        #[arg(long)]
        all: bool,

        /// Show what would be removed without making changes
        #[arg(long)]
        dry_run: bool,

        /// Interactive selection mode
        #[arg(short, long)]
        interactive: bool,
    },

    /// Show the status of applied overlays
    Status {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show only a specific overlay
        #[arg(short, long)]
        name: Option<String>,

        /// Output as JSON for scripting and CI integration
        #[arg(long, conflicts_with = "quiet")]
        json: bool,

        /// Quiet mode: exit code only (0 = overlays applied, 1 = none)
        #[arg(short, long, conflicts_with = "json")]
        quiet: bool,
    },

    /// Restore overlays after git clean or other removal
    Restore {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would be restored without applying
        #[arg(long)]
        dry_run: bool,

        /// Overwrite existing files during restore
        #[arg(long, visible_alias = "overwrite", conflicts_with_all = ["skip_conflicts", "interactive"])]
        force: bool,

        /// Skip conflicting files silently during restore
        #[arg(long, conflicts_with_all = ["force", "interactive"])]
        skip_conflicts: bool,

        /// Prompt interactively for each conflict during restore
        #[arg(short, long, conflicts_with_all = ["force", "skip_conflicts"])]
        interactive: bool,

        /// Deep merge conflicting JSON files instead of failing
        #[arg(long, env = "REPOVERLAY_MERGE")]
        merge: bool,
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

        /// Overwrite existing files during update
        #[arg(long, visible_alias = "overwrite", conflicts_with_all = ["skip_conflicts", "interactive"])]
        force: bool,

        /// Skip conflicting files silently during update
        #[arg(long, conflicts_with_all = ["force", "interactive"])]
        skip_conflicts: bool,

        /// Prompt interactively for each conflict during update
        #[arg(short, long, conflicts_with_all = ["force", "skip_conflicts"])]
        interactive: bool,

        /// Deep merge conflicting JSON files instead of failing
        #[arg(long, env = "REPOVERLAY_MERGE")]
        merge: bool,
    },

    /// Create a new overlay from files in a repository
    ///
    /// Examples:
    ///   repoverlay create my-overlay              # Detects org/repo from git remote
    ///   repoverlay create org/repo/my-overlay     # Explicit target
    ///   repoverlay create --output ./output        # Write to local directory
    Create {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit target
        /// Omit when using --output for local directory output
        name: Option<String>,

        /// Include specific files or directories (can be specified multiple times)
        #[arg(short, long)]
        include: Vec<PathBuf>,

        /// Source repository to extract files from (defaults to current directory)
        #[arg(short, long)]
        source: Option<PathBuf>,

        /// Output directory for local overlay creation (no overlay repo required)
        #[arg(short, long)]
        output: Option<PathBuf>,

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

    /// Create a new overlay in a local directory
    ///
    /// Examples:
    ///   repoverlay create-local ./output      # Write to local directory
    #[command(name = "create-local", hide = true)]
    CreateLocal {
        /// Output directory for the overlay
        output: PathBuf,

        /// Include specific files or directories (can be specified multiple times)
        #[arg(short, long)]
        include: Vec<PathBuf>,

        /// Source repository to extract files from (defaults to current directory)
        #[arg(short, long)]
        source: Option<PathBuf>,

        /// Show what would be created without creating files
        #[arg(long)]
        dry_run: bool,

        /// Skip interactive prompts, use defaults
        #[arg(short = 'y', long)]
        yes: bool,

        /// Force overwrite if output already exists
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

        /// Overwrite existing repo files when applying the new overlay
        #[arg(long, visible_alias = "overwrite", conflicts_with_all = ["skip_conflicts", "interactive"])]
        force: bool,

        /// Skip conflicting repo files silently when applying the new overlay
        #[arg(long, conflicts_with_all = ["force", "interactive"])]
        skip_conflicts: bool,

        /// Prompt interactively for each conflict when applying the new overlay
        #[arg(short, long, conflicts_with_all = ["force", "skip_conflicts"])]
        interactive: bool,

        /// Deep merge conflicting JSON files instead of failing
        #[arg(long, env = "REPOVERLAY_MERGE")]
        merge: bool,
    },

    /// Manage the overlay cache
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },

    /// Browse available overlays from the overlay repository
    #[command(name = "browse")]
    Browse {
        /// Overlay source (GitHub username, owner/repo, or URL)
        ///
        /// Browse overlays from this source without adding it as a configured source.
        /// If omitted, uses configured sources.
        #[arg(value_name = "SOURCE")]
        source: Option<String>,

        /// Filter by target repository (format: org/repo)
        #[arg(short = 'f', long)]
        filter: Option<String>,

        /// Update overlay repo before listing
        #[arg(long)]
        update: bool,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Disable interactive selection (just list overlays)
        #[arg(long)]
        no_interactive: bool,

        /// Show what would be applied without making changes
        #[arg(long)]
        dry_run: bool,

        /// Show all overlays, including those for other repositories
        #[arg(long)]
        show_all: bool,
    },

    /// List available overlays from the overlay repository
    #[command(name = "list", hide = true)]
    List {
        /// Filter by target repository (format: org/repo)
        #[arg(short = 'f', long, alias = "target")]
        filter: Option<String>,

        /// Update overlay repo before listing
        #[arg(long)]
        update: bool,
    },

    /// Sync changes from an applied overlay back to the overlay repo
    ///
    /// Examples:
    ///   repoverlay sync my-overlay          # Detects org/repo from git remote
    ///   repoverlay sync org/repo/my-overlay # Explicit target
    ///   repoverlay sync --all               # Sync all applied overlays
    Sync {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit values
        name: Option<String>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Sync all applied overlays from the overlay repo
        #[arg(long)]
        all: bool,

        /// Show what would be synced without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Add files to an existing applied overlay
    ///
    /// Deprecated: use `repoverlay edit --add` instead.
    #[command(hide = true)]
    Add {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit values
        name: String,

        /// Files to add (relative paths from target repo)
        files: Vec<PathBuf>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would be added without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Edit an existing applied overlay (add files, remove files, or re-select interactively)
    ///
    /// Examples:
    ///   repoverlay edit my-overlay --add newfile.txt
    ///   repoverlay edit my-overlay --remove oldfile.txt
    ///   repoverlay edit my-overlay --add new.txt --remove old.txt
    ///   repoverlay edit org/repo/my-overlay --interactive
    Edit {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit values
        name: String,

        /// Files to add to the overlay
        #[arg(short, long, value_name = "FILE", num_args = 1..)]
        add: Vec<PathBuf>,

        /// Files to remove from the overlay
        #[arg(short, long, value_name = "FILE", num_args = 1..)]
        remove: Vec<PathBuf>,

        /// Re-run interactive file selection with current files pre-selected
        #[arg(short, long, conflicts_with_all = ["add", "remove"])]
        interactive: bool,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would change without making changes
        #[arg(long)]
        dry_run: bool,
    },

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

    /// Manage overlay sources (for multi-source configurations)
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum SourceCommand {
    /// Add a new overlay source
    Add {
        /// Git URL, GitHub shorthand (owner/repo), or GitHub username
        url: config::SourceUrlInput,

        /// Name for this source (defaults to repo name)
        #[arg(long)]
        name: Option<String>,
    },

    /// List configured overlay sources
    List,

    /// Remove an overlay source
    Remove {
        /// Name of the source to remove
        name: String,
    },
}

#[derive(Subcommand)]
enum CacheCommand {
    /// List cached repositories
    List,

    /// Clear all cached repositories
    #[command(hide = true)]
    Clear {
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Remove cached repositories
    Remove {
        /// Repository to remove (format: owner/repo)
        #[arg(conflicts_with = "all")]
        repo: Option<String>,

        /// Remove all cached repositories
        #[arg(short, long)]
        all: bool,

        /// Skip confirmation prompt (used with --all)
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Show cache location
    Path,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    // Handle markdown help generation (for documentation)
    if cli.markdown_help {
        clap_markdown::print_help_markdown::<Cli>();
        return Ok(());
    }

    // Show help when no command is provided
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(());
    };

    match command {
        Commands::Apply {
            source,
            target,
            copy,
            name,
            r#ref,
            update,
            force,
            skip_conflicts,
            interactive,
            merge,
            from_source,
            dry_run,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            let conflict_strategy = if force {
                ConflictStrategy::Force
            } else if skip_conflicts {
                ConflictStrategy::SkipConflicts
            } else if interactive {
                ConflictStrategy::Interactive
            } else {
                ConflictStrategy::Fail
            };
            apply_overlay(
                &source,
                &target,
                copy,
                name,
                r#ref.as_deref(),
                update,
                conflict_strategy,
                merge,
                from_source.as_deref(),
                dry_run,
            )?;
        }
        Commands::Remove {
            name,
            target,
            all,
            dry_run,
            interactive,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            handle_remove(&target, name, all, dry_run, interactive)?;
        }
        Commands::Status {
            target,
            name,
            json,
            quiet,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            if quiet {
                let has_overlays = status_has_overlays(&target, name.as_deref())?;
                if !has_overlays {
                    std::process::exit(1);
                }
            } else if json {
                show_status_json(&target, name.as_deref())?;
            } else {
                show_status(&target, name)?;
            }
        }
        Commands::Restore {
            target,
            dry_run,
            force,
            skip_conflicts,
            interactive,
            merge,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            let conflict_strategy = if force {
                ConflictStrategy::Force
            } else if skip_conflicts {
                ConflictStrategy::SkipConflicts
            } else if interactive {
                ConflictStrategy::Interactive
            } else {
                ConflictStrategy::Fail
            };
            restore_overlays(&target, dry_run, conflict_strategy, merge)?;
        }
        Commands::Update {
            name,
            target,
            dry_run,
            force,
            skip_conflicts,
            interactive,
            merge,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            let conflict_strategy = if force {
                ConflictStrategy::Force
            } else if skip_conflicts {
                ConflictStrategy::SkipConflicts
            } else if interactive {
                ConflictStrategy::Interactive
            } else {
                ConflictStrategy::Fail
            };
            update_overlays(&target, name, dry_run, conflict_strategy, merge)?;
        }
        Commands::Create {
            name,
            include,
            source,
            output,
            dry_run,
            yes,
            force,
        } => {
            let source = source.unwrap_or_else(|| PathBuf::from("."));
            create_overlay_command(&source, name, output, &include, dry_run, yes, force)?;
        }
        Commands::CreateLocal {
            output,
            include,
            source,
            dry_run,
            yes,
            force: _,
        } => {
            eprintln!(
                "{} 'repoverlay create-local' is deprecated and will be removed in 1.0.",
                "Warning:".yellow().bold()
            );
            eprintln!("         Use 'repoverlay create --output <path>' instead.");
            eprintln!();

            let source = source.unwrap_or_else(|| PathBuf::from("."));
            crate::create_overlay(&source, Some(output), &include, None, dry_run, yes)?;
        }
        Commands::Switch {
            source,
            target,
            copy,
            name,
            r#ref,
            force,
            skip_conflicts,
            interactive,
            merge,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            let conflict_strategy = if force {
                ConflictStrategy::Force
            } else if skip_conflicts {
                ConflictStrategy::SkipConflicts
            } else if interactive {
                ConflictStrategy::Interactive
            } else {
                ConflictStrategy::Fail
            };
            switch_overlay(
                &source,
                &target,
                copy,
                name,
                r#ref.as_deref(),
                conflict_strategy,
                merge,
            )?;
        }
        Commands::Cache { command } => {
            handle_cache_command(command)?;
        }
        Commands::Browse {
            source,
            filter,
            update,
            target,
            no_interactive,
            dry_run,
            show_all,
        } => {
            browse_overlays(
                source.as_deref(),
                filter.as_deref(),
                update,
                target,
                no_interactive,
                dry_run,
                show_all,
            )?;
        }
        Commands::List { filter, update } => {
            eprintln!(
                "{} 'repoverlay list' is deprecated and will be removed in 1.0.",
                "Warning:".yellow().bold()
            );
            eprintln!("         Use 'repoverlay browse' instead.");
            eprintln!();

            browse_overlays(None, filter.as_deref(), update, None, true, false, true)?;
        }
        Commands::Sync {
            name,
            target,
            all,
            dry_run,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            handle_sync(&target, name, all, dry_run)?;
        }
        Commands::Edit {
            name,
            add,
            remove,
            interactive,
            target,
            dry_run,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            edit_overlay(&name, &target, &add, &remove, interactive, dry_run)?;
        }
        Commands::Add {
            name,
            files,
            target,
            dry_run,
        } => {
            eprintln!(
                "{} 'repoverlay add' is deprecated and will be removed in 1.0. Use 'repoverlay edit --add' instead.",
                "Warning:".yellow().bold()
            );
            eprintln!();
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            add_files_to_overlay(&name, &target, &files, dry_run)?;
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
                "{} 'repoverlay publish' is deprecated and will be removed in 1.0.",
                "Warning:".yellow().bold()
            );
            eprintln!(
                "         Use 'repoverlay create <name>' instead to create overlays in the overlay repo."
            );
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
        Commands::Source { command } => {
            handle_source_command(command)?;
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "repoverlay", &mut io::stdout());
            // Skip update check for completions - output goes to shell scripts
            return Ok(());
        }
    }

    // Check for updates after successful command execution
    check_for_updates();

    Ok(())
}

/// Handle source subcommands.
fn handle_source_command(command: SourceCommand) -> Result<()> {
    use colored::Colorize;

    let mut config = config::load_config(None)?;

    match command {
        SourceCommand::Add { url, name } => {
            // URL is already validated and parsed by clap via FromStr
            let validated_url = url.to_url();

            // Extract name from validated URL if not provided
            let source_name = name.unwrap_or_else(|| {
                validated_url
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("source")
                    .trim_end_matches(".git")
                    .to_string()
            });

            // Validate extracted name is not empty
            if source_name.is_empty() {
                anyhow::bail!(
                    "Could not extract source name from URL. Please provide a name with --name"
                );
            }

            // Check if name already exists
            if config.sources.iter().any(|s| s.name == source_name) {
                anyhow::bail!("Source '{source_name}' already exists");
            }

            let new_source = config::Source {
                name: source_name.clone(),
                url: validated_url.clone(),
            };

            // Append to end of sources list
            config.sources.push(new_source);
            config::save_config(&config)?;

            println!(
                "{} source '{}' at position {}",
                "Added".green().bold(),
                source_name,
                config.sources.len()
            );
            println!("       URL: {validated_url}");
        }
        SourceCommand::List => {
            if config.sources.is_empty() {
                println!("No overlay sources configured.");
                println!();
                println!("Add a source with:");
                println!("  repoverlay source add <url>");
                return Ok(());
            }

            println!("{}", "Configured overlay sources (priority order):".bold());
            println!();

            for (i, source) in config.sources.iter().enumerate() {
                println!(
                    "  {}. {} {}",
                    i + 1,
                    source.name.cyan(),
                    "(highest priority)"
                        .dimmed()
                        .to_string()
                        .chars()
                        .take(if i == 0 { 18 } else { 0 })
                        .collect::<String>()
                );
                println!("     URL: {}", source.url);
            }

            // Show legacy config if present
            if let Some(ref legacy) = config.overlay_repo {
                println!();
                println!("{}", "Legacy configuration (deprecated):".yellow());
                println!("  overlay_repo: {}", legacy.url);
            }
        }
        SourceCommand::Remove { name } => {
            let original_len = config.sources.len();
            config.sources.retain(|s| s.name != name);

            if config.sources.len() == original_len {
                anyhow::bail!("Source '{name}' not found");
            }

            config::save_config(&config)?;

            println!("{} source '{}'", "Removed".red().bold(), name);
        }
    }

    Ok(())
}

/// Handle remove command with interactive selection support.
fn handle_remove(
    target: &std::path::Path,
    name: Option<String>,
    remove_all: bool,
    dry_run: bool,
    interactive: bool,
) -> Result<()> {
    use crate::selection::{FlatSelectionConfig, SelectableItem, select_flat};

    // If name or --all is specified, use direct removal
    if remove_all || name.is_some() {
        return remove_overlay(target, name, remove_all, dry_run);
    }

    // If not interactive and no name specified, require explicit action.
    // In an interactive terminal, default to interactive mode automatically.
    if !interactive && !is_interactive() {
        bail!(
            "No overlay name specified.\n\n\
             Usage:\n  \
             repoverlay remove <name>        # Remove specific overlay\n  \
             repoverlay remove --all         # Remove all overlays\n  \
             repoverlay remove --interactive # Interactive selection"
        );
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

    let items: Vec<SelectableItem> = applied_overlays
        .iter()
        .map(|name| SelectableItem {
            id: name.to_string(),
            label: name.to_string(),
            description: None,
            preselected: false,
            disabled: false,
        })
        .collect();

    let result = select_flat(
        &items,
        &FlatSelectionConfig {
            prompt: "Select overlay(s) to remove:".into(),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlays selected for removal");
    }

    let remove_all = result.selected_ids.len() == applied_overlays.len();

    for overlay_name in &result.selected_ids {
        if dry_run {
            println!(
                "{} Dry run - would remove overlay '{overlay_name}'",
                "Note:".yellow()
            );
        } else {
            remove_single_overlay(&target, &overlays_dir, overlay_name)?;
        }
    }

    if !dry_run {
        if remove_all {
            fs::remove_dir_all(target.join(STATE_DIR))?;
            println!("\n{} Removed all overlays", "✓".green().bold());
        } else {
            let remaining = list_applied_overlays(&target)?;
            if remaining.is_empty() {
                fs::remove_dir_all(target.join(STATE_DIR))?;
            }
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
            eprintln!(
                "{} 'repoverlay cache clear' is deprecated and will be removed in 1.0.",
                "Warning:".yellow().bold()
            );
            eprintln!("         Use 'repoverlay cache remove --all' instead.");
            eprintln!();

            clear_cache(&cache, yes)?;
        }

        CacheCommand::Remove { repo, all, yes } => {
            if all {
                clear_cache(&cache, yes)?;
            } else if let Some(repo) = repo {
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
                    println!("{owner}/{repo_name} is not cached.");
                }
            } else {
                bail!(
                    "Specify a repository to remove or use --all.\n\n\
                     Usage:\n  \
                     repoverlay cache remove <owner/repo>  # Remove specific repo\n  \
                     repoverlay cache remove --all          # Remove all cached repos"
                );
            }
        }

        CacheCommand::Path => {
            println!("{}", cache.cache_dir().display());
        }
    }

    Ok(())
}

/// Clear the entire cache with optional confirmation prompt.
fn clear_cache(cache: &CacheManager, skip_confirm: bool) -> Result<()> {
    if !skip_confirm {
        print!("Remove all cached repositories? [y/N] ");
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
        "{} Removed {} cached repository(s).",
        "✓".green().bold(),
        count
    );
    Ok(())
}

/// Filter overlays to those matching the current repository.
///
/// When `skip_filter` is true, returns all overlays unfiltered. Otherwise, detects
/// the current repository from git remotes and returns only matching overlays.
/// Falls back to showing all overlays if detection fails or nothing matches.
///
/// Returns `(overlays, was_filtered)`.
fn auto_filter_overlays(
    overlays: Vec<crate::overlay_repo::AvailableOverlay>,
    skip_filter: bool,
) -> (Vec<crate::overlay_repo::AvailableOverlay>, bool) {
    use crate::upstream::detect_repo_identity;

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
        .filter(|o| identity.matches(&o.org, &o.repo))
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
fn print_overlay_list(overlays: &[crate::overlay_repo::AvailableOverlay], filtered: bool) {
    println!("{}\n", "Available overlays:".bold());

    // Group by org/repo
    let mut current_group: Option<(String, String)> = None;
    for overlay in overlays {
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

    if filtered {
        println!(
            "\n{}",
            "Showing overlays for current repository. Use --show-all to see all.".dimmed()
        );
    }

    println!(
        "\nTo apply an overlay: repoverlay apply {}",
        "<org>/<repo>/<name>".dimmed()
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
fn browse_overlays(
    source: Option<&str>,
    target_filter: Option<&str>,
    update: bool,
    target: Option<PathBuf>,
    no_interactive: bool,
    dry_run: bool,
    show_all: bool,
) -> Result<()> {
    use crate::config::load_config;
    use crate::overlay_repo::OverlayRepoManager;
    use crate::state::OverlaySource;

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

    let config = load_config(None)?;
    let overlay_config = config.get_default_overlay_repo_config()?;

    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    if update {
        println!("{} overlay repository...", "Updating".blue().bold());
        manager.pull()?;
    }

    let overlays = if let Some(filter) = target_filter {
        let parts: Vec<&str> = filter.split('/').collect();
        if parts.len() != 2 {
            bail!("Invalid target filter format. Use: org/repo");
        }
        manager.list_overlays_for_repo(parts[0], parts[1])?
    } else {
        manager.list_overlays()?
    };

    let commit = manager.get_current_commit()?;
    let build_source_info = |o: &crate::overlay_repo::AvailableOverlay| {
        let path = manager.get_overlay_path(&o.org, &o.repo, &o.name)?;
        Ok(ResolvedSource {
            path,
            source_info: OverlaySource::overlay_repo(
                o.org.clone(),
                o.repo.clone(),
                o.name.clone(),
                commit.clone(),
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
    use crate::github::GitHubSource;
    use crate::reference::SourceReference;
    use crate::state::OverlaySource;

    let reference = SourceReference::parse(source_str);

    let (owner, repo) = match reference {
        SourceReference::OnePart { username } => {
            let default_repo = config::default_overlay_repo_name();
            (username, default_repo)
        }
        SourceReference::TwoPart { owner, repo } => (owner, repo),
        SourceReference::GitHubUrl(url) => {
            let github_source = GitHubSource::parse(&url)?;
            (github_source.owner, github_source.repo)
        }
        SourceReference::LocalPath { .. } | SourceReference::ThreePart { .. } => {
            bail!(
                "Invalid source for browse: '{source_str}'\n\n\
                 Use a GitHub username, owner/repo, or GitHub URL."
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

    let build_source_info = |o: &crate::overlay_repo::AvailableOverlay| {
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

/// Shared logic for browse: filter, display, select, and apply overlays.
#[allow(clippy::fn_params_excessive_bools)]
fn browse_and_apply<F>(
    overlays: Vec<crate::overlay_repo::AvailableOverlay>,
    target_filter: Option<&str>,
    target: Option<PathBuf>,
    no_interactive: bool,
    dry_run: bool,
    show_all: bool,
    build_source: F,
) -> Result<()>
where
    F: Fn(&crate::overlay_repo::AvailableOverlay) -> Result<ResolvedSource>,
{
    use crate::selection::{FlatSelectionConfig, SelectableItem, select_flat};
    use crate::state::normalize_overlay_name;

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
            let disabled = normalize_overlay_name(&o.name)
                .is_ok_and(|normalized| applied_overlays.iter().any(|n| n == normalized.as_str()));
            SelectableItem {
                id: o.to_string(),
                label: o.display_bold(),
                description: disabled.then(|| "already applied".into()),
                preselected: false,
                disabled,
            }
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

/// Publish an overlay to the overlay repository.
fn publish_overlay(
    source: &std::path::Path,
    target: Option<&str>,
    name: Option<&str>,
    message: Option<&str>,
    no_push: bool,
    dry_run: bool,
) -> Result<()> {
    use crate::config::load_config;
    use crate::overlay_repo::OverlayRepoManager;
    use crate::state;

    // Validate source exists
    let source = canonicalize_path(source, "Overlay source")?;
    if !source.is_dir() {
        bail!("Source must be a directory: {}", source.display());
    }

    // Load config
    let config = load_config(None)?;
    let overlay_config = config.get_default_overlay_repo_config()?;

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
    println!("  Target:  {org}/{repo}");
    println!("  Name:    {overlay_name}");

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        println!("\nWould publish to: {org}/{repo}/{overlay_name}");
        return Ok(());
    }

    // Create manager and ensure cloned
    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    // Pull latest
    println!("\n{} latest changes...", "Pulling".blue().bold());
    manager.pull()?;

    // Stage the overlay
    let copying = "Copying".blue().bold();
    println!("{copying} overlay files...");
    let dest = manager.stage_overlay(&org, &repo, &overlay_name, &source)?;
    println!("  Copied to: {}", dest.display());

    // Check if there are changes
    if !manager.has_staged_changes()? {
        println!("\n{} No changes to publish.", "Note:".yellow());
        return Ok(());
    }

    // Commit
    let commit_msg = message
        .unwrap_or(&format!("Update overlay: {org}/{repo}/{overlay_name}"))
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
        let check = "✓".green().bold();
        println!("\n{check} Overlay published: {org}/{repo}/{overlay_name}");
    }

    println!("\nTo apply: repoverlay apply {org}/{repo}/{overlay_name}");

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
                    "Invalid overlay path format: {name_arg}\n\n\
                     Use one of:\n  \
                     - my-overlay (detects org/repo from git remote)\n  \
                     - org/repo/my-overlay (explicit)"
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
                "Invalid overlay path format: {name_arg}\n\n\
                 Use one of:\n  \
                 - my-overlay (detects org/repo from git remote)\n  \
                 - org/repo/my-overlay (explicit)"
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
    use crate::config::load_config;
    use crate::overlay_repo::OverlayRepoManager;

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
        return crate::create_overlay(
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
             repoverlay create --output ./output   # Write to local directory"
        )
    })?;

    // Parse the name argument
    let (org, repo, overlay_name) = parse_overlay_name_arg(&name_arg, source)?;

    // Load overlay repo config
    let config = load_config(None)?;
    let overlay_config = config.get_default_overlay_repo_config()?;

    // Create manager and ensure cloned
    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;

    // Determine output path in overlay repo
    let output_path = manager.path().join(&org).join(&repo).join(&overlay_name);

    // Check if overlay already exists
    if output_path.exists() && !force {
        bail!(
            "Overlay '{org}/{repo}/{overlay_name}' already exists.\n\n\
             To update an applied overlay, use: repoverlay sync {overlay_name}\n\
             To overwrite, use: repoverlay create {name_arg} --force"
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
        return crate::create_overlay(
            source,
            Some(output_path),
            include,
            Some(overlay_name.clone()),
            dry_run,
            yes,
        )
        .and_then(|()| {
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
    let copied_files = crate::copy_files_to_overlay(source, &output_path, include)?;

    // Generate config
    fs::write(
        output_path.join("repoverlay.ccl"),
        crate::generate_overlay_config(&overlay_name),
    )?;

    crate::print_overlay_created(&output_path, &copied_files);

    // Auto-commit
    auto_commit_overlay(&manager, &org, &repo, &overlay_name, true)?;

    Ok(())
}

/// Auto-commit changes to an overlay in the overlay repo.
fn auto_commit_overlay(
    manager: &crate::overlay_repo::OverlayRepoManager,
    org: &str,
    repo: &str,
    name: &str,
    is_new: bool,
) -> Result<()> {
    use std::process::Command;

    // Fetch latest from remote before committing to avoid divergence
    println!("{} overlay repo...", "Syncing".blue().bold());
    let fetch_output = Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(manager.path())
        .output()
        .context("Failed to fetch from remote")?;

    if fetch_output.status.success() {
        // Try to pull/rebase to incorporate remote changes
        let pull_output = Command::new("git")
            .args(["pull", "--rebase", "--autostash"])
            .current_dir(manager.path())
            .output()
            .context("Failed to pull from remote")?;

        if !pull_output.status.success() {
            let stderr = String::from_utf8_lossy(&pull_output.stderr);
            // If pull fails due to conflicts, warn but continue
            eprintln!(
                "{} Could not pull latest changes: {}",
                "Warning:".yellow(),
                stderr.trim()
            );
        }
    } else {
        // Fetch failed, but continue - might be offline
        eprintln!(
            "{} Could not fetch from remote (offline?), continuing...",
            "Warning:".yellow()
        );
    }

    // Check if there are changes to commit
    if !manager.has_staged_changes()? {
        // Stage all changes
        let output = Command::new("git")
            .args(["add", "."])
            .current_dir(manager.path())
            .output()
            .context("Failed to stage changes")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr.trim();
            bail!("Failed to stage changes: {msg}");
        }
    }

    // Check again if there are staged changes
    if !manager.has_staged_changes()? {
        println!("{} No changes to commit.", "Note:".yellow());
        return Ok(());
    }

    let action = if is_new { "Add" } else { "Update" };
    let commit_msg = format!("{action} overlay: {org}/{repo}/{name}");

    println!("{} changes...", "Committing".blue().bold());
    manager.commit(&commit_msg)?;

    // Auto-push to remote
    println!("{} to remote...", "Pushing".blue().bold());
    match manager.push() {
        Ok(()) => {
            let check = "✓".green().bold();
            let action_word = if is_new { "created" } else { "updated" };
            println!("\n{check} Overlay {action_word}: {org}/{repo}/{name}");
        }
        Err(e) => {
            let warn = "Warning:".yellow();
            eprintln!("\n{warn} Committed locally but failed to push: {e}");
            eprintln!("Run 'repoverlay push' to push manually when online.");
        }
    }

    println!("To apply: repoverlay apply {org}/{repo}/{name}");

    Ok(())
}

/// Handle the sync command, dispatching to single or all-overlay sync.
fn handle_sync(
    target: &std::path::Path,
    name: Option<String>,
    sync_all: bool,
    dry_run: bool,
) -> Result<()> {
    use crate::config::load_config;
    use crate::overlay_repo::OverlayRepoManager;
    use crate::state::{OverlaySource, SourceResolver};
    use crate::{load_overlay_state, normalize_overlay_name};

    // Validate target is a git repo
    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        let target_display = target.display();
        bail!("Target directory is not a git repository: {target_display}");
    }

    if sync_all {
        let applied_overlays = list_applied_overlays(&target)?;
        if applied_overlays.is_empty() {
            println!("{} No overlays are currently applied.", "Note:".yellow());
            return Ok(());
        }

        // Load overlay repo config and manager once for all overlays
        let config = load_config(None)?;
        let overlay_config = config.get_default_overlay_repo_config()?;
        let manager = OverlayRepoManager::new(overlay_config)?;
        manager.ensure_cloned()?;

        let mut synced = 0u32;
        let mut skipped = 0u32;

        for overlay_name in &applied_overlays {
            let state = load_overlay_state(&target, overlay_name.as_str())?;

            // Use SourceResolver to check syncability (#146, #149)
            if !state.source.is_syncable() {
                // Special case: GitHub sources that are actually overlay repos
                // (applied via two-part browse mode) can still be synced.
                let mut handled = false;
                if let OverlaySource::GitHub {
                    owner,
                    repo: gh_repo,
                    subpath,
                    ..
                } = &state.source
                    && let Some(overlay_ref) =
                        subpath.as_deref().and_then(parse_github_overlay_subpath)
                {
                    let is_overlay_repo = config
                        .sources
                        .iter()
                        .any(|s| is_overlay_repo_url(&s.url, owner, gh_repo));

                    if is_overlay_repo {
                        let (ref org, ref repo, ref name) = overlay_ref;
                        sync_single_overlay(&target, org, name, repo, &state, &manager, dry_run)?;
                        if !dry_run {
                            auto_commit_overlay(&manager, org, repo, name, false)?;
                        }
                        synced += 1;
                        handled = true;
                    }
                }

                if !handled {
                    let label = state.source.source_type_label();
                    println!(
                        "{} Skipping '{}' ({label} source, not syncable to overlay repo)",
                        "Warning:".yellow(),
                        overlay_name
                    );
                    skipped += 1;
                }
                continue;
            }

            // OverlayRepo source — sync directly
            match &state.source {
                OverlaySource::OverlayRepo {
                    org, repo, name, ..
                } => {
                    sync_single_overlay(&target, org, name, repo, &state, &manager, dry_run)?;
                    if !dry_run {
                        auto_commit_overlay(&manager, org, repo, name, false)?;
                    }
                    synced += 1;
                }
                // Other source types are already handled by the is_syncable check above
                _ => unreachable!("is_syncable() returned true for non-OverlayRepo source"),
            }
        }

        println!();
        let check = "✓".green().bold();
        println!("{check} Synced {synced} overlay(s), skipped {skipped}");
    } else if let Some(name_arg) = name {
        // Parse the name argument to get org/repo/name
        let (org, repo, overlay_name) = parse_overlay_name_arg(&name_arg, &target)?;

        // Verify the overlay is currently applied
        let normalized_name = normalize_overlay_name(&overlay_name)?;
        let applied_overlays = list_applied_overlays(&target)?;

        if !applied_overlays
            .iter()
            .any(|n| n == normalized_name.as_str())
        {
            bail!(
                "Overlay '{overlay_name}' is not currently applied.\n\n\
                 To apply it first: repoverlay apply {org}/{repo}/{overlay_name}"
            );
        }

        // Load overlay state to get file mappings
        let state = load_overlay_state(&target, &normalized_name)?;

        // Check source syncability upfront (#146, #149)
        {
            use crate::state::SourceResolver;
            if !state.source.is_syncable() {
                let label = state.source.source_type_label();
                bail!(
                    "Cannot sync overlay '{overlay_name}' ({label} source).\n\n\
                     Only overlay repo sources can be synced."
                );
            }
        }

        // Load overlay repo config (respects source_name for multi-source configs, #147)
        let config = load_config(None)?;
        let source_name = match &state.source {
            OverlaySource::OverlayRepo { source_name, .. } => source_name.as_deref(),
            _ => None,
        };
        let overlay_config = config.get_overlay_repo_config_by_name(source_name)?;

        // Create manager and ensure cloned
        let manager = OverlayRepoManager::new(overlay_config)?;
        manager.ensure_cloned()?;

        sync_single_overlay(
            &target,
            &org,
            &overlay_name,
            &repo,
            &state,
            &manager,
            dry_run,
        )?;

        // Auto-commit
        auto_commit_overlay(&manager, &org, &repo, &overlay_name, false)?;
    } else {
        bail!(
            "Must specify an overlay name or use --all.\n\n\
             Usage:\n  \
             repoverlay sync my-overlay\n  \
             repoverlay sync --all"
        );
    }

    Ok(())
}

/// Parse a GitHub overlay subpath like "org/repo/name" into its components.
///
/// Returns `Some((org, repo, name))` if the subpath has exactly three parts.
fn parse_github_overlay_subpath(subpath: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = subpath.split('/').collect();
    if parts.len() == 3 && parts.iter().all(|p| !p.is_empty()) {
        Some((
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
        ))
    } else {
        None
    }
}

/// Check if a configured source URL matches a GitHub owner/repo.
///
/// Handles both full URLs (`https://github.com/owner/repo`) and
/// shorthand that was expanded during config deserialization.
fn is_overlay_repo_url(source_url: &str, gh_owner: &str, gh_repo: &str) -> bool {
    let expected_suffix = format!("/{gh_owner}/{gh_repo}");
    let url_trimmed = source_url.trim_end_matches(".git");
    url_trimmed.ends_with(&expected_suffix)
}

/// Sync a single overlay's files from the target repo back to the overlay repo.
fn sync_single_overlay(
    target: &std::path::Path,
    org: &str,
    overlay_name: &str,
    repo: &str,
    state: &crate::state::OverlayState,
    manager: &crate::overlay_repo::OverlayRepoManager,
    dry_run: bool,
) -> Result<()> {
    let overlay_repo_path = manager.path().join(org).join(repo).join(overlay_name);

    if !overlay_repo_path.exists() {
        bail!(
            "Overlay '{org}/{repo}/{overlay_name}' does not exist in overlay repo.\n\n\
             Did you mean to use 'repoverlay create {org}/{repo}/{overlay_name}' instead?"
        );
    }

    let syncing = "Syncing".blue().bold();
    println!("{syncing} overlay: {org}/{repo}/{overlay_name}");

    if dry_run {
        println!("  Target: {}", target.display());
        println!("  Repo:   {}", overlay_repo_path.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());

        // Show what would be synced
        println!("\nFiles that would be synced:");
        for entry in state.file_entries() {
            let target_file = target.join(&entry.target);

            if target_file.exists() {
                println!(
                    "  {} {} -> {}",
                    "→".cyan(),
                    entry.target.display(),
                    entry.source.display()
                );
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
    }

    Ok(())
}

/// Edit an existing applied overlay (add files, remove files, or re-select interactively).
fn edit_overlay(
    name_arg: &str,
    target: &std::path::Path,
    add_files: &[PathBuf],
    remove_files: &[PathBuf],
    interactive: bool,
    dry_run: bool,
) -> Result<()> {
    // Validate at least one operation.
    // In an interactive terminal, default to interactive mode automatically.
    if add_files.is_empty() && remove_files.is_empty() && !interactive && !is_interactive() {
        bail!(
            "No operation specified. Please specify at least one of:\n  \
             --add <file>      Add files to the overlay\n  \
             --remove <file>   Remove files from the overlay\n  \
             --interactive     Re-select files interactively"
        );
    }

    // Handle add
    if !add_files.is_empty() {
        add_files_to_overlay(name_arg, target, add_files, dry_run)?;
    }

    // Handle remove (stub for now)
    if !remove_files.is_empty() {
        remove_files_from_overlay(name_arg, target, remove_files, dry_run)?;
    }

    // Interactive mode (explicit flag or auto-detected interactive terminal with no other ops)
    if interactive || (add_files.is_empty() && remove_files.is_empty() && is_interactive()) {
        interactive_edit_overlay(name_arg, target, dry_run)?;
    }

    Ok(())
}

/// Resolve an overlay's source to a local filesystem path.
///
/// Uses the `SourceResolver` trait to handle all source types uniformly:
/// - Local: returns the stored path directly
/// - `OverlayRepo`: reconstructs path from the overlay repo (respects `source_name`)
/// - GitHub: returns the cached download path
fn resolve_overlay_source_path(state: &crate::state::OverlayState) -> Result<PathBuf> {
    use crate::state::SourceResolver;

    state.source.resolve_local_path()
}

/// Interactively re-select which files from an overlay source should be applied.
///
/// Shows the selection UI with all files from the overlay source directory,
/// pre-selecting the currently applied files. Computes the diff between the
/// old and new selections and applies adds/removes accordingly.
fn interactive_edit_overlay(name_arg: &str, target: &std::path::Path, dry_run: bool) -> Result<()> {
    use crate::detection::{DetectedFile, FileCategory};
    use crate::selection::{SelectionConfig, select_files};
    use crate::{list_applied_overlays, load_overlay_state, normalize_overlay_name};
    use std::collections::HashSet;
    use walkdir::WalkDir;

    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        bail!(
            "Target directory is not a git repository: {}",
            target.display()
        );
    }

    // Parse overlay name and verify it's applied
    let overlay_name = if name_arg.contains('/') {
        let parts: Vec<&str> = name_arg.split('/').collect();
        if parts.len() == 3 {
            parts[2].to_string()
        } else {
            bail!("Invalid overlay path: {name_arg}");
        }
    } else {
        name_arg.to_string()
    };

    let normalized_name = normalize_overlay_name(&overlay_name)?;
    let applied_overlays = list_applied_overlays(&target)?;
    if !applied_overlays
        .iter()
        .any(|n| n == normalized_name.as_str())
    {
        bail!("Overlay '{overlay_name}' is not currently applied.");
    }

    let state = load_overlay_state(&target, &normalized_name)?;

    // Check mutability upfront before any changes (#142, #148, #149)
    {
        use crate::state::SourceResolver;
        if !state.source.is_mutable() {
            let label = state.source.source_type_label();
            bail!(
                "Interactive edit is not supported for {label} overlays.\n\n\
                 {label} overlays are read-only. Use --add and --remove flags instead."
            );
        }
    }

    // Resolve overlay source to a local directory
    let source_path = resolve_overlay_source_path(&state)?;

    if !source_path.exists() {
        bail!(
            "Overlay source directory not found: {}\n\n\
             Interactive edit requires access to the overlay source files.",
            source_path.display()
        );
    }

    // Collect current overlay file paths for pre-selection
    let current_files: HashSet<PathBuf> = state
        .file_entries()
        .iter()
        .map(|e| e.target.clone())
        .collect();

    // Walk the overlay source directory to find all available files.
    // Only skip .git directory - hidden directories like .claude/, .vscode/, etc.
    // are valid overlay content and must be included.
    let mut detected_files: Vec<DetectedFile> = Vec::new();
    for entry in WalkDir::new(&source_path)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            // Only skip .git directory
            !(e.file_type().is_dir() && e.file_name() == ".git")
        })
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            let relative = entry
                .path()
                .strip_prefix(&source_path)
                .unwrap_or_else(|_| entry.path())
                .to_path_buf();

            let rel_str = relative.to_string_lossy();

            // Skip overlay config file and cache metadata (not user-facing overlay content)
            if relative == std::path::Path::new(crate::state::CONFIG_FILE)
                || rel_str == ".repoverlay-cache-meta.ccl"
            {
                continue;
            }

            let is_currently_applied = current_files.contains(&relative);

            detected_files.push(DetectedFile {
                path: relative,
                category: FileCategory::Untracked, // Generic category for overlay files
                preselected: is_currently_applied,
                depth: 0,
                parent_dir: None,
            });
        }
    }

    if detected_files.is_empty() {
        bail!(
            "No files found in overlay source: {}",
            source_path.display()
        );
    }

    // Sort by path for consistent display
    detected_files.sort_by(|a, b| a.path.cmp(&b.path));

    // Configure selection UI - don't hide any categories
    let config = SelectionConfig {
        prompt: format!("Edit overlay '{normalized_name}' \u{2014} select files to include"),
        default_hidden_categories: HashSet::new(),
    };

    let result = select_files(&detected_files, &config)?;

    if result.cancelled {
        println!("{} Selection cancelled, no changes made.", "Note:".yellow());
        return Ok(());
    }

    // Compute diff: what to add and what to remove
    let new_selection: HashSet<PathBuf> = result.selected_files.into_iter().collect();

    let to_add: Vec<PathBuf> = new_selection.difference(&current_files).cloned().collect();
    let to_remove: Vec<PathBuf> = current_files.difference(&new_selection).cloned().collect();

    if to_add.is_empty() && to_remove.is_empty() {
        println!("{} No changes to apply.", "Note:".yellow());
        return Ok(());
    }

    // Print summary
    println!("{} overlay: {}", "Editing".blue().bold(), normalized_name);
    if !to_add.is_empty() {
        println!("\nFiles to add:");
        for f in &to_add {
            println!("  {} {}", "+".green(), f.display());
        }
    }
    if !to_remove.is_empty() {
        println!("\nFiles to remove:");
        for f in &to_remove {
            println!("  {} {}", "-".red(), f.display());
        }
    }

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        return Ok(());
    }

    // Apply removals first, then additions
    if !to_remove.is_empty() {
        remove_files_from_overlay(name_arg, &target, &to_remove, false)?;
    }
    if !to_add.is_empty() {
        // Copy files from overlay source to target, then add to overlay.
        // The files exist in the overlay source but not in the target repo.
        // add_files_to_overlay expects them to exist in the target first.
        for file in &to_add {
            let source_file = source_path.join(file);
            let target_file = target.join(file);
            if let Some(parent) = target_file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_file, &target_file).with_context(|| {
                format!("Failed to copy {} from overlay source", file.display())
            })?;
        }
        add_files_to_overlay(name_arg, &target, &to_add, false)?;
    }

    Ok(())
}

fn remove_files_from_overlay(
    name_arg: &str,
    target: &std::path::Path,
    files: &[PathBuf],
    dry_run: bool,
) -> Result<()> {
    use crate::state::EntryType;
    use crate::{
        load_overlay_state, normalize_overlay_name, save_external_state, save_overlay_state,
        update_git_exclude,
    };

    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        bail!(
            "Target directory is not a git repository: {}",
            target.display()
        );
    }

    // Extract overlay name from the argument (handles both short and full forms)
    let overlay_name = if name_arg.contains('/') {
        let parts: Vec<&str> = name_arg.split('/').collect();
        if parts.len() == 3 {
            parts[2].to_string()
        } else {
            bail!(
                "Invalid overlay path format: {name_arg}\n\n\
                 Use one of:\n  \
                 - my-overlay (detects org/repo from git remote)\n  \
                 - org/repo/my-overlay (explicit)"
            );
        }
    } else {
        name_arg.to_string()
    };

    let normalized_name = normalize_overlay_name(&overlay_name)?;
    let applied_overlays = list_applied_overlays(&target)?;

    if !applied_overlays
        .iter()
        .any(|n| n == normalized_name.as_str())
    {
        let names: Vec<&str> = applied_overlays.iter().map(OverlayName::as_str).collect();
        bail!(
            "Overlay '{}' is not currently applied.\n\n\
             Applied overlays: {}",
            overlay_name,
            if applied_overlays.is_empty() {
                "(none)".to_string()
            } else {
                names.join(", ")
            }
        );
    }

    let mut state = load_overlay_state(&target, &normalized_name)?;

    // Validate all files are managed by this overlay
    for file in files {
        let file_normalized = file.to_string_lossy().replace('\\', "/");
        if !state
            .file_entries()
            .iter()
            .any(|e| e.target.to_string_lossy().replace('\\', "/") == file_normalized)
        {
            let managed: Vec<String> = state
                .file_entries()
                .iter()
                .map(|e| e.target.to_string_lossy().into_owned())
                .collect();
            bail!(
                "File '{}' is not managed by overlay '{}'.\n\n\
                 Files in this overlay: {}",
                file.display(),
                normalized_name,
                managed.join(", ")
            );
        }
    }

    println!(
        "{} files from overlay: {}",
        "Removing".red().bold(),
        normalized_name
    );

    if dry_run {
        println!("  Target: {}", target.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        println!("\nFiles that would be removed:");
        for file in files {
            println!("  {} {}", "-".red(), file.display());
        }
        return Ok(());
    }

    let mut removed_count = 0;

    for file in files {
        let file_path = target.join(file);

        if file_path.exists() || file_path.is_symlink() {
            // Find entry type from state before removing
            let is_directory = state
                .file_entries()
                .iter()
                .any(|e| e.target == *file && e.entry_type == EntryType::Directory);

            if is_directory {
                if file_path.is_symlink() {
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
                    fs::remove_dir_all(&file_path).with_context(|| {
                        format!("Failed to remove directory: {}", file_path.display())
                    })?;
                }
                println!("  {} {}/", "-".red(), file.display());
            } else {
                fs::remove_file(&file_path)
                    .with_context(|| format!("Failed to remove: {}", file_path.display()))?;
                println!("  {} {}", "-".red(), file.display());
            }

            // Clean up empty parent directories
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

        // Remove from state
        state.remove_file(file);
        removed_count += 1;
    }

    // Rebuild exclude entries from remaining files
    let remaining_entries: Vec<String> = state
        .file_entries()
        .iter()
        .map(|e| {
            let path = e.target.to_string_lossy().replace('\\', "/");
            match e.entry_type {
                EntryType::Directory => format!("{path}/"),
                EntryType::File => path,
            }
        })
        .collect();

    // Rewrite git exclude with remaining entries
    update_git_exclude(&target, &normalized_name, &remaining_entries, true)?;

    // Save updated state
    save_overlay_state(&target, &state)?;

    // Save external backup
    if let Err(e) = save_external_state(&target, &normalized_name, &state) {
        eprintln!(
            "  {} Could not save external backup: {}",
            "Warning:".yellow(),
            e
        );
    }

    println!(
        "\n{} Removed {} file(s) from overlay '{}'",
        "done".green().bold(),
        removed_count,
        normalized_name
    );

    Ok(())
}

/// Add files to an existing applied overlay.
///
/// This adds new files to an overlay that is already applied to the target repository.
/// The files are linked to the overlay repo and the overlay state is updated.
fn add_files_to_overlay(
    name_arg: &str,
    target: &std::path::Path,
    files: &[PathBuf],
    dry_run: bool,
) -> Result<()> {
    use crate::state::{EntryType, FileEntry, LinkType};
    use crate::{
        load_all_overlay_targets, load_overlay_state, normalize_overlay_name, save_external_state,
        save_overlay_state, update_git_exclude,
    };

    // Validate target is a git repo
    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        let target_display = target.display();
        bail!("Target directory is not a git repository: {target_display}");
    }

    // Check that files were provided
    if files.is_empty() {
        bail!(
            "No files specified.\n\n\
             Usage: repoverlay add <overlay-name> <file> [<file>...]"
        );
    }

    // Parse the name argument to get org/repo/name
    let (org, repo, overlay_name) = parse_overlay_name_arg(name_arg, &target)?;

    // Verify the overlay is currently applied
    let normalized_name = normalize_overlay_name(&overlay_name)?;
    let applied_overlays = list_applied_overlays(&target)?;

    if !applied_overlays
        .iter()
        .any(|n| n == normalized_name.as_str())
    {
        bail!(
            "Overlay '{overlay_name}' is not currently applied.\n\n\
             To apply it first: repoverlay apply {org}/{repo}/{overlay_name}"
        );
    }

    // Load existing overlay state
    let mut state = load_overlay_state(&target, &normalized_name)?;

    // Check source mutability upfront before any filesystem changes (#148)
    {
        use crate::state::SourceResolver;
        if !state.source.is_mutable() {
            let label = state.source.source_type_label();
            bail!(
                "Cannot add files to a {label} overlay (read-only source).\n\n\
                 {label} overlays are cached read-only. Use a local or overlay repo source instead."
            );
        }
    }

    // Validate all files exist
    for file in files {
        let full_path = target.join(file);
        if !full_path.exists() {
            bail!(
                "File does not exist: {}\n\n\
                 Create the file first, then add it to the overlay.",
                file.display()
            );
        }
    }

    // Load all existing overlay targets to check for conflicts
    let existing_targets = load_all_overlay_targets(&target)?;

    // Check that files aren't already managed by an overlay
    for file in files {
        let file_str = file.to_string_lossy().replace('\\', "/");
        if let Some(other_overlay) = existing_targets.get(&file_str) {
            bail!(
                "File '{}' is already managed by overlay '{}'.\n\
                 Remove it from that overlay first.",
                file.display(),
                other_overlay
            );
        }
    }

    println!(
        "{} files to overlay: {}",
        "Adding".blue().bold(),
        overlay_name
    );

    if dry_run {
        println!("  Target: {}", target.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        println!("\nFiles that would be added:");
        for file in files {
            println!("  {} {}", "+".green(), file.display());
        }
        return Ok(());
    }

    // Resolve the overlay source to a local path using the SourceResolver trait (#149).
    // This correctly handles Local, OverlayRepo (with source_name), and GitHub sources.
    let overlay_repo_path = {
        use crate::state::SourceResolver;
        state.source.resolve_local_path().with_context(|| {
            format!("Failed to resolve source path for overlay '{overlay_name}'")
        })?
    };

    if !overlay_repo_path.exists() {
        bail!(
            "Overlay source directory not found: {}\n\n\
             Did you mean to use 'repoverlay create {name_arg}' instead?",
            overlay_repo_path.display()
        );
    }

    // Determine link type (symlink unless on Windows)
    let link_type = if cfg!(windows) {
        LinkType::Copy
    } else {
        LinkType::Symlink
    };

    let mut exclude_entries: Vec<String> = Vec::new();
    let mut added_count = 0;

    for file in files {
        let target_file = target.join(file);
        let overlay_file = overlay_repo_path.join(file);

        // Copy file to overlay repo
        if let Some(parent) = overlay_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&target_file, &overlay_file)
            .with_context(|| format!("Failed to copy {} to overlay repo", target_file.display()))?;

        // Remove original file (we'll replace it with symlink)
        fs::remove_file(&target_file)
            .with_context(|| format!("Failed to remove {} for linking", target_file.display()))?;

        // Create symlink/copy from overlay repo to target
        match link_type {
            LinkType::Symlink => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&overlay_file, &target_file).with_context(|| {
                    format!("Failed to create symlink: {}", target_file.display())
                })?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(&overlay_file, &target_file).with_context(
                    || format!("Failed to create symlink: {}", target_file.display()),
                )?;
            }
            LinkType::Copy | LinkType::Merged => {
                fs::copy(&overlay_file, &target_file)
                    .with_context(|| format!("Failed to copy file: {}", target_file.display()))?;
            }
        }

        // Add to state
        state.add_file(FileEntry {
            source: file.clone(),
            target: file.clone(),
            link_type,
            entry_type: EntryType::File,
        });

        // Add to exclude list
        let exclude_path = file.to_string_lossy().replace('\\', "/");
        exclude_entries.push(exclude_path);

        println!("  {} {}", "+".green(), file.display());
        added_count += 1;
    }

    // Update git exclude with new entries
    update_git_exclude(&target, &normalized_name, &exclude_entries, true)?;

    // Save updated overlay state
    save_overlay_state(&target, &state)?;

    // Save external backup
    if let Err(e) = save_external_state(&target, &normalized_name, &state) {
        eprintln!(
            "  {} Could not save external backup: {}",
            "Warning:".yellow(),
            e
        );
    }

    println!(
        "\n{} Added {} file(s) to overlay '{}'",
        "✓".green().bold(),
        added_count,
        overlay_name
    );

    // Auto-commit to overlay repo (only for OverlayRepo sources)
    if let crate::state::OverlaySource::OverlayRepo { source_name, .. } = &state.source {
        use crate::config::load_config;
        use crate::overlay_repo::OverlayRepoManager;

        let config = load_config(None)?;
        let overlay_config = config.get_overlay_repo_config_by_name(source_name.as_deref())?;
        let manager = OverlayRepoManager::new(overlay_config)?;
        auto_commit_overlay(&manager, &org, &repo, &overlay_name, false)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_overlay;
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
        use crate::remove_overlay_section;

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
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "apply_overlay failed: {result:?}");

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
                ConflictStrategy::default(),
                false,
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
                true, // copy mode
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                    r"mappings =
  .envrc = .env
",
                ),
            ]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
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
                    r"overlay =
  name = my-custom-overlay
",
                ),
            ]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Conflict") || err.contains("already managed"));
        }

        #[test]
        fn force_overwrites_existing_file() {
            let repo = create_test_repo();
            // Create existing file
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let overlay = create_test_overlay(&[(".envrc", "overlay content")]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::Force,
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "force should allow overwriting: {result:?}");

            // File should now be a symlink to overlay
            let target_file = repo.path().join(".envrc");
            assert!(target_file.is_symlink(), ".envrc should be a symlink");
        }

        #[test]
        fn force_reapplies_same_name_overlay() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "overlay content")]);

            // Apply first time
            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Apply again with same name using force
            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                false,
                ConflictStrategy::Force,
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "force should allow re-applying: {result:?}");
        }

        #[test]
        fn skip_conflicts_skips_existing_file() {
            let repo = create_test_repo();
            // Create existing file
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let overlay = create_test_overlay(&[
                (".envrc", "overlay content"),
                ("other.txt", "other content"),
            ]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::SkipConflicts,
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "skip_conflicts should succeed: {result:?}");

            // .envrc should NOT be a symlink (kept existing)
            let envrc = repo.path().join(".envrc");
            assert!(!envrc.is_symlink(), ".envrc should NOT be a symlink");
            assert_eq!(
                fs::read_to_string(&envrc).unwrap(),
                "existing content",
                ".envrc should have original content"
            );

            // other.txt should be applied
            let other = repo.path().join("other.txt");
            assert!(other.exists(), "other.txt should exist");
            assert!(other.is_symlink(), "other.txt should be a symlink");
        }

        #[test]
        fn skip_conflicts_fails_on_same_name_overlay() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "content")]);

            // Apply first time
            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // skip_conflicts should NOT allow re-applying same name
            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                false,
                ConflictStrategy::SkipConflicts,
                false,
                None,
                false,
            );
            assert!(result.is_err(), "skip_conflicts should fail on same-name");
            assert!(result.unwrap_err().to_string().contains("already applied"));
        }

        #[test]
        fn force_overwrites_existing_directory() {
            let repo = create_test_repo();
            // Create existing directory that will conflict
            fs::create_dir_all(repo.path().join("scratch")).unwrap();
            fs::write(repo.path().join("scratch/existing.txt"), "existing").unwrap();

            let overlay = TempDir::new().unwrap();
            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::Force,
                false,
                None,
                false,
            );
            assert!(
                result.is_ok(),
                "force should allow overwriting directory: {result:?}"
            );

            // scratch dir should now be a symlink to overlay
            let scratch_dir = repo.path().join("scratch");
            assert!(scratch_dir.is_symlink(), "scratch should be a symlink");
        }

        #[test]
        fn skip_conflicts_skips_existing_directory() {
            let repo = create_test_repo();
            // Create existing directory that will conflict
            fs::create_dir_all(repo.path().join("scratch")).unwrap();
            fs::write(repo.path().join("scratch/existing.txt"), "existing").unwrap();

            let overlay = TempDir::new().unwrap();
            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();
            // Also add a non-conflicting file
            fs::write(overlay.path().join("other.txt"), "other content").unwrap();
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::SkipConflicts,
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "skip_conflicts should succeed: {result:?}");

            // scratch should NOT be a symlink (kept existing)
            let scratch_dir = repo.path().join("scratch");
            assert!(!scratch_dir.is_symlink(), "scratch should NOT be a symlink");
            assert_eq!(
                fs::read_to_string(scratch_dir.join("existing.txt")).unwrap(),
                "existing",
                "existing file should be preserved"
            );

            // other.txt should be applied
            let other = repo.path().join("other.txt");
            assert!(other.exists(), "other.txt should exist");
        }

        #[test]
        fn force_fails_on_cross_overlay_file_conflict() {
            let repo = create_test_repo();

            // Apply first overlay with .envrc
            let overlay1 = create_test_overlay(&[(".envrc", "first")]);
            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("first".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Try to apply second overlay with same file using Force
            let overlay2 = create_test_overlay(&[(".envrc", "second")]);
            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("second".to_string()),
                None,
                false,
                ConflictStrategy::Force,
                false,
                None,
                false,
            );
            assert!(
                result.is_err(),
                "force should still fail on cross-overlay conflict"
            );
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("already managed by overlay"),
                "error should mention cross-overlay conflict: {err}"
            );
        }

        #[test]
        fn skip_conflicts_skips_cross_overlay_file_conflict() {
            let repo = create_test_repo();

            // Apply first overlay with .envrc
            let overlay1 = create_test_overlay(&[(".envrc", "first")]);
            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("first".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Apply second overlay with overlapping file + unique file using SkipConflicts
            let overlay2 =
                create_test_overlay(&[(".envrc", "second"), ("unique.txt", "unique content")]);
            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("second".to_string()),
                None,
                false,
                ConflictStrategy::SkipConflicts,
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "skip_conflicts should succeed: {result:?}");

            // unique.txt should be applied
            assert!(
                repo.path().join("unique.txt").exists(),
                "unique.txt should be applied"
            );
        }

        #[test]
        fn force_fails_on_cross_overlay_directory_conflict() {
            let repo = create_test_repo();

            // Apply first overlay with scratch directory
            let overlay1 = TempDir::new().unwrap();
            fs::create_dir_all(overlay1.path().join("scratch")).unwrap();
            fs::write(overlay1.path().join("scratch/notes.txt"), "notes").unwrap();
            fs::write(
                overlay1.path().join("repoverlay.ccl"),
                "overlay =\n  name = overlay-a\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Try to apply second overlay with same directory using Force
            let overlay2 = TempDir::new().unwrap();
            fs::create_dir_all(overlay2.path().join("scratch")).unwrap();
            fs::write(overlay2.path().join("scratch/other.txt"), "other").unwrap();
            fs::write(
                overlay2.path().join("repoverlay.ccl"),
                "overlay =\n  name = overlay-b\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::Force,
                false,
                None,
                false,
            );
            assert!(
                result.is_err(),
                "force should still fail on cross-overlay directory conflict"
            );
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("already managed by overlay"),
                "error should mention cross-overlay conflict: {err}"
            );
        }

        #[test]
        fn skip_conflicts_skips_cross_overlay_directory_conflict() {
            let repo = create_test_repo();

            // Apply first overlay with scratch directory
            let overlay1 = TempDir::new().unwrap();
            fs::create_dir_all(overlay1.path().join("scratch")).unwrap();
            fs::write(overlay1.path().join("scratch/notes.txt"), "notes").unwrap();
            fs::write(
                overlay1.path().join("repoverlay.ccl"),
                "overlay =\n  name = overlay-a\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Apply second overlay with same dir + unique file using SkipConflicts
            let overlay2 = TempDir::new().unwrap();
            fs::create_dir_all(overlay2.path().join("scratch")).unwrap();
            fs::write(overlay2.path().join("scratch/other.txt"), "other").unwrap();
            fs::write(overlay2.path().join("unique.txt"), "content").unwrap();
            fs::write(
                overlay2.path().join("repoverlay.ccl"),
                "overlay =\n  name = overlay-b\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::SkipConflicts,
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "skip_conflicts should succeed: {result:?}");

            // unique.txt should be applied
            assert!(
                repo.path().join("unique.txt").exists(),
                "unique.txt should be applied"
            );
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No files found"));
        }

        #[test]
        fn fails_on_nonexistent_source() {
            let repo = create_test_repo();
            let result = apply_overlay(
                "/nonexistent/path",
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_err());
        }

        #[test]
        fn applies_directory_symlink() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create a directory with files inside
            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();
            fs::write(overlay.path().join("scratch/todo.md"), "# TODO").unwrap();

            // Create config with directories list
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "apply_overlay failed: {result:?}");

            // Check directory symlink was created
            let target_dir = repo.path().join("scratch");
            assert!(target_dir.exists(), "scratch should exist");
            assert!(
                target_dir.is_symlink(),
                "scratch should be a symlink to directory"
            );

            // Check files inside are accessible
            assert!(target_dir.join("notes.txt").exists());
            assert!(target_dir.join("todo.md").exists());

            // Check content is correct
            let content = fs::read_to_string(target_dir.join("notes.txt")).unwrap();
            assert_eq!(content, "notes");
        }

        #[test]
        fn applies_directory_with_copy_mode() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create a directory with files inside
            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();

            // Create config with directories list
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true, // copy mode
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "apply_overlay failed: {result:?}");

            // Check directory was copied (not symlinked)
            let target_dir = repo.path().join("scratch");
            assert!(target_dir.exists(), "scratch should exist");
            assert!(
                !target_dir.is_symlink(),
                "scratch should NOT be a symlink in copy mode"
            );
            assert!(target_dir.is_dir(), "scratch should be a directory");

            // Check files inside are accessible
            assert!(target_dir.join("notes.txt").exists());
        }

        #[test]
        fn directory_symlink_skips_files_inside() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create a directory with files and a standalone file
            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();
            fs::write(overlay.path().join(".envrc"), "export FOO=bar").unwrap();

            // Create config with directories list
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "apply_overlay failed: {result:?}");

            // Check directory symlink was created
            let target_dir = repo.path().join("scratch");
            assert!(target_dir.is_symlink(), "scratch should be a symlink");

            // Check standalone file was also symlinked
            let envrc = repo.path().join(".envrc");
            assert!(envrc.is_symlink(), ".envrc should be a symlink");
        }

        #[test]
        fn directory_symlink_warns_on_nonexistent() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create a regular file so overlay isn't empty
            fs::write(overlay.path().join(".envrc"), "export FOO=bar").unwrap();

            // Create config with directories list including non-existent directory
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = nonexistent\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            // Should succeed (just warns about missing directory)
            assert!(result.is_ok(), "apply_overlay failed: {result:?}");

            // Check regular file was still symlinked
            assert!(repo.path().join(".envrc").is_symlink());

            // Non-existent directory should not be created
            assert!(!repo.path().join("nonexistent").exists());
        }

        #[test]
        fn directory_conflict_with_existing_path() {
            let repo = create_test_repo();

            // Create a directory in the repo that conflicts
            fs::create_dir_all(repo.path().join("scratch")).unwrap();
            fs::write(repo.path().join("scratch/existing.txt"), "existing").unwrap();

            let overlay = TempDir::new().unwrap();
            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("already exists"));
        }

        #[test]
        fn directory_conflict_with_existing_overlay() {
            let repo = create_test_repo();

            // Apply first overlay with a directory
            let overlay1 = TempDir::new().unwrap();
            fs::create_dir_all(overlay1.path().join("scratch")).unwrap();
            fs::write(overlay1.path().join("scratch/notes.txt"), "notes").unwrap();
            fs::write(
                overlay1.path().join("repoverlay.ccl"),
                "overlay =\n  name = overlay-a\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Try to apply second overlay with same directory
            let overlay2 = TempDir::new().unwrap();
            fs::create_dir_all(overlay2.path().join("scratch")).unwrap();
            fs::write(overlay2.path().join("scratch/other.txt"), "other").unwrap();
            fs::write(
                overlay2.path().join("repoverlay.ccl"),
                "overlay =\n  name = overlay-b\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("already managed"));
        }

        #[test]
        fn directory_symlink_updates_git_exclude_with_trailing_slash() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            // Directory should have trailing slash in exclude
            assert!(content.contains("scratch/"));
        }

        #[test]
        fn directory_path_not_a_directory_warning() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create a file (not directory) that matches the directory name
            fs::write(overlay.path().join("scratch"), "this is a file").unwrap();
            fs::write(overlay.path().join(".envrc"), "export FOO=bar").unwrap();
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            // Should succeed (just warns about scratch not being a directory)
            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_ok());

            // Regular file should still be symlinked
            assert!(repo.path().join(".envrc").is_symlink());
            // scratch as a directory symlink should not exist (it was a file in overlay)
            assert!(!repo.path().join("scratch").is_symlink());
        }

        #[test]
        #[ignore = "tylerbutler/santa#71: forward slashes in map keys cause parsing errors in sickle"]
        fn mapping_supports_nested_paths_in_key_and_value() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create nested source file
            fs::create_dir_all(overlay.path().join("config")).unwrap();
            fs::write(overlay.path().join("config/settings.json"), "{}").unwrap();

            // Map nested source path to nested destination path (forward slashes in both)
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                r"mappings =
  config/settings.json = .vscode/settings.json
",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "apply_overlay failed: {result:?}");

            // Check that mapping was applied (file at nested destination)
            assert!(
                repo.path().join(".vscode/settings.json").exists(),
                "mapped target should exist at nested path"
            );
            assert!(
                !repo.path().join("config/settings.json").exists(),
                "original path should not exist when mapped"
            );
        }

        #[test]
        fn nested_directory_symlinks_use_forward_slashes() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create nested directory structure
            fs::create_dir_all(overlay.path().join("config/editors")).unwrap();
            fs::write(
                overlay.path().join("config/editors/vscode.json"),
                r#"{"editor": "vscode"}"#,
            )
            .unwrap();

            // Use forward slashes in directories list (portable across platforms)
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                r"overlay =
  name = test-overlay

directories =
  = config/editors
",
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );
            assert!(result.is_ok(), "apply_overlay failed: {result:?}");

            // Check directory symlink was created
            let target_dir = repo.path().join("config/editors");
            assert!(target_dir.exists(), "config/editors should exist");
            assert!(
                target_dir.is_symlink(),
                "config/editors should be a symlink"
            );

            // Check files inside are accessible
            assert!(target_dir.join("vscode.json").exists());
        }

        #[test]
        fn dry_run_does_not_apply_overlay() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                true, // dry_run
            );
            assert!(result.is_ok(), "apply_overlay dry_run failed: {result:?}");

            // Check no files were created
            assert!(
                !repo.path().join(".envrc").exists(),
                ".envrc should not exist in dry run"
            );
            // Check no state was saved
            assert!(
                !repo.path().join(".repoverlay").exists(),
                ".repoverlay dir should not exist in dry run"
            );
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();
            remove_overlay(repo.path(), Some("test-overlay".to_string()), false, false).unwrap();

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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            assert!(repo.path().join(".envrc").exists());
            assert!(repo.path().join(".env.local").exists());

            remove_overlay(repo.path(), None, true, false).unwrap();

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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            remove_overlay(repo.path(), Some("overlay-a".to_string()), false, false).unwrap();

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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();
            assert!(repo.path().join(".vscode").exists());

            remove_overlay(repo.path(), Some("test".to_string()), false, false).unwrap();
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();
            remove_overlay(repo.path(), Some("test".to_string()), false, false).unwrap();

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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();
            remove_overlay(repo.path(), Some("test".to_string()), false, false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(!content.contains("# repoverlay:test start"));
            assert!(!content.contains(".envrc"));
            assert!(!content.contains("# repoverlay:managed"));
        }

        #[test]
        fn fails_when_no_overlay_applied() {
            let repo = create_test_repo();

            let result = remove_overlay(repo.path(), Some("nonexistent".to_string()), false, false);
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let result =
                remove_overlay(repo.path(), Some("fake-overlay".to_string()), false, false);
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Manually delete the file
            fs::remove_file(repo.path().join(".envrc")).unwrap();

            // Remove should still succeed
            let result = remove_overlay(repo.path(), Some("test".to_string()), false, false);
            assert!(result.is_ok());
        }

        #[test]
        fn removes_directory_symlink() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create a directory with files inside
            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();

            // Create config with directories list
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Verify directory symlink exists
            assert!(repo.path().join("scratch").is_symlink());

            // Remove overlay
            remove_overlay(repo.path(), Some("test-overlay".to_string()), false, false).unwrap();

            // Verify directory symlink was removed
            assert!(!repo.path().join("scratch").exists());
            assert!(!repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn removes_copied_directory() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            // Create a directory with files inside
            fs::create_dir_all(overlay.path().join("scratch")).unwrap();
            fs::write(overlay.path().join("scratch/notes.txt"), "notes").unwrap();

            // Create config with directories list
            fs::write(
                overlay.path().join("repoverlay.ccl"),
                "overlay =\n  name = test-overlay\n\ndirectories =\n  = scratch\n",
            )
            .unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true, // copy mode
                None,
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Verify directory was copied (not symlink)
            let target_dir = repo.path().join("scratch");
            assert!(!target_dir.is_symlink());
            assert!(target_dir.is_dir());

            // Remove overlay
            remove_overlay(repo.path(), Some("test-overlay".to_string()), false, false).unwrap();

            // Verify directory was removed
            assert!(!repo.path().join("scratch").exists());
            assert!(!repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn dry_run_does_not_remove_overlay() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Dry run removal
            let result = remove_overlay(repo.path(), Some("test-overlay".to_string()), false, true);
            assert!(result.is_ok(), "dry_run remove failed: {result:?}");

            // Verify files are still present
            assert!(
                repo.path().join(".envrc").exists(),
                ".envrc should still exist after dry run"
            );
            assert!(
                repo.path().join(".repoverlay").exists(),
                ".repoverlay should still exist after dry run"
            );
        }

        #[test]
        fn dry_run_all_does_not_remove_overlays() {
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Dry run removal of all
            let result = remove_overlay(repo.path(), None, true, true);
            assert!(result.is_ok(), "dry_run remove --all failed: {result:?}");

            // Verify all files are still present
            assert!(
                repo.path().join(".envrc").exists(),
                ".envrc should still exist after dry run"
            );
            assert!(
                repo.path().join(".env.local").exists(),
                ".env.local should still exist after dry run"
            );
        }

        #[test]
        fn handle_remove_requires_name_or_interactive_flag() {
            let repo = create_test_repo();

            // Calling handle_remove without name, --all, or --interactive should fail
            let result = handle_remove(repo.path(), None, false, false, false);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("No overlay name specified"),
                "Expected usage error, got: {err}"
            );
        }

        #[test]
        fn handle_remove_with_name_succeeds() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Calling handle_remove with a name should succeed
            let result = handle_remove(
                repo.path(),
                Some("test-overlay".to_string()),
                false,
                false,
                false,
            );
            assert!(result.is_ok(), "handle_remove with name failed: {result:?}");
            assert!(!repo.path().join(".envrc").exists());
        }

        #[test]
        fn handle_remove_with_all_flag_succeeds() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("test-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Calling handle_remove with --all should succeed
            let result = handle_remove(repo.path(), None, true, false, false);
            assert!(
                result.is_ok(),
                "handle_remove with --all failed: {result:?}"
            );
            assert!(!repo.path().join(".envrc").exists());
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
            assert!(result.is_ok(), "create_overlay failed: {result:?}");

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
            assert!(result.is_ok(), "create_overlay failed: {result:?}");

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
                "Expected error about no files, got: {err_msg}"
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

        #[test]
        fn create_local_creates_overlay_in_output_directory() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();
            fs::create_dir_all(source.path().join(".vscode")).unwrap();
            fs::write(
                source.path().join(".vscode/settings.json"),
                r#"{"key": "value"}"#,
            )
            .unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("my-local-overlay")),
                &[PathBuf::from(".envrc"), PathBuf::from(".vscode")],
                None,
                false,
                false,
            );
            assert!(result.is_ok(), "create_overlay failed: {result:?}");

            // Check files were created in output directory
            let overlay_dir = output.path().join("my-local-overlay");
            assert!(overlay_dir.exists(), "Overlay directory should exist");
            assert!(overlay_dir.join(".envrc").exists(), ".envrc should exist");
            assert!(
                overlay_dir.join(".vscode/settings.json").exists(),
                ".vscode/settings.json should exist"
            );

            // Verify content
            let content = fs::read_to_string(overlay_dir.join(".envrc")).unwrap();
            assert_eq!(content, "export FOO=bar");
        }

        #[test]
        fn create_local_dry_run_does_not_create_files() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

            let result = create_overlay(
                source.path(),
                Some(output.path().join("my-local-overlay")),
                &[PathBuf::from(".envrc")],
                None,
                true, // dry_run
                false,
            );
            assert!(result.is_ok());

            // Check no files were created
            assert!(
                !output.path().join("my-local-overlay").exists(),
                "Overlay directory should not exist in dry run"
            );
        }
    }

    // Unit tests for parse_overlay_name_arg
    mod parse_overlay_name_arg_tests {
        use super::*;

        #[test]
        fn parses_full_form_org_repo_name() {
            let source = create_test_repo();

            // Add a git remote so we have a valid git repo
            Command::new("git")
                .args(["config", "user.email", "test@test.com"])
                .current_dir(source.path())
                .output()
                .unwrap();
            Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(source.path())
                .output()
                .unwrap();

            let result = parse_overlay_name_arg("myorg/myrepo/my-overlay", source.path());
            assert!(result.is_ok());
            let (org, repo, name) = result.unwrap();
            assert_eq!(org, "myorg");
            assert_eq!(repo, "myrepo");
            assert_eq!(name, "my-overlay");
        }

        #[test]
        fn fails_on_invalid_single_slash() {
            let source = create_test_repo();
            let result = parse_overlay_name_arg("org/name", source.path());
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Invalid"));
        }

        #[test]
        fn fails_on_too_many_slashes() {
            let source = create_test_repo();
            let result = parse_overlay_name_arg("a/b/c/d", source.path());
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Invalid"));
        }

        #[test]
        fn fails_on_empty_parts_in_full_form() {
            let source = create_test_repo();

            // Empty org
            let result = parse_overlay_name_arg("/repo/name", source.path());
            assert!(result.is_err());

            // Empty repo
            let result = parse_overlay_name_arg("org//name", source.path());
            assert!(result.is_err());

            // Empty name
            let result = parse_overlay_name_arg("org/repo/", source.path());
            assert!(result.is_err());
        }

        #[test]
        fn short_form_requires_git_remote() {
            let source = create_test_repo();
            // No remote configured, should fail
            let result = parse_overlay_name_arg("my-overlay", source.path());
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Could not detect"));
        }

        #[test]
        fn short_form_works_with_github_remote() {
            let source = create_test_repo();

            // Configure git
            Command::new("git")
                .args(["config", "user.email", "test@test.com"])
                .current_dir(source.path())
                .output()
                .unwrap();
            Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(source.path())
                .output()
                .unwrap();

            // Add a GitHub remote
            Command::new("git")
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/testorg/testrepo.git",
                ])
                .current_dir(source.path())
                .output()
                .unwrap();

            let result = parse_overlay_name_arg("my-overlay", source.path());
            assert!(result.is_ok());
            let (org, repo, name) = result.unwrap();
            assert_eq!(org, "testorg");
            assert_eq!(repo, "testrepo");
            assert_eq!(name, "my-overlay");
        }
    }

    // Unit tests for detect_target_repo
    mod detect_target_repo_tests {
        use super::*;

        #[test]
        fn fails_without_remote() {
            let repo = create_test_repo();
            let result = detect_target_repo(repo.path());
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Could not detect"));
        }

        #[test]
        fn detects_https_github_remote() {
            let repo = create_test_repo();

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

            let result = detect_target_repo(repo.path());
            assert!(result.is_ok());
            let (org, name) = result.unwrap();
            assert_eq!(org, "owner");
            assert_eq!(name, "repo");
        }

        #[test]
        fn detects_ssh_github_remote() {
            let repo = create_test_repo();

            Command::new("git")
                .args(["remote", "add", "origin", "git@github.com:owner/repo.git"])
                .current_dir(repo.path())
                .output()
                .unwrap();

            let result = detect_target_repo(repo.path());
            assert!(result.is_ok());
            let (org, name) = result.unwrap();
            assert_eq!(org, "owner");
            assert_eq!(name, "repo");
        }

        #[test]
        fn fails_for_non_github_remote() {
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

            let result = detect_target_repo(repo.path());
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Non-GitHub"));
        }
    }

    // Unit tests for handle_cache_command edge cases
    mod cache_command_tests {
        use super::*;

        #[test]
        fn cache_remove_fails_on_invalid_format() {
            // Invalid format (no slash)
            let result = handle_cache_command(CacheCommand::Remove {
                repo: Some("invalid".to_string()),
                all: false,
                yes: false,
            });
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Invalid repository format")
            );
        }

        #[test]
        fn cache_remove_fails_on_too_many_slashes() {
            let result = handle_cache_command(CacheCommand::Remove {
                repo: Some("a/b/c".to_string()),
                all: false,
                yes: false,
            });
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Invalid repository format")
            );
        }

        #[test]
        fn cache_remove_fails_when_no_repo_or_all() {
            let result = handle_cache_command(CacheCommand::Remove {
                repo: None,
                all: false,
                yes: false,
            });
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Specify a repository")
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_ok(), "switch_overlay failed: {result:?}");

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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
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
                ConflictStrategy::default(),
                false,
            )
            .unwrap();

            // Verify old overlays are removed
            assert!(!repo.path().join(".envrc").exists());
            assert!(!repo.path().join(".env.local").exists());

            // Verify new overlay is applied
            assert!(repo.path().join(".env.prod").exists());
        }

        #[test]
        fn force_overwrites_existing_repo_file() {
            let repo = create_test_repo();
            // Create a file that will conflict with the new overlay
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let overlay = create_test_overlay(&[(".envrc", "overlay content")]);

            let result = switch_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                ConflictStrategy::Force,
                false,
            );
            assert!(
                result.is_ok(),
                "switch with force should overwrite: {result:?}"
            );

            // File should now be a symlink to overlay
            assert!(
                repo.path().join(".envrc").is_symlink(),
                ".envrc should be a symlink"
            );
        }

        #[test]
        fn skip_conflicts_skips_existing_repo_file() {
            let repo = create_test_repo();
            // Create a file that will conflict with the new overlay
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let overlay = create_test_overlay(&[
                (".envrc", "overlay content"),
                ("other.txt", "other content"),
            ]);

            let result = switch_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                ConflictStrategy::SkipConflicts,
                false,
            );
            assert!(
                result.is_ok(),
                "switch with skip_conflicts should succeed: {result:?}"
            );

            // .envrc should NOT be a symlink (kept existing)
            let envrc = repo.path().join(".envrc");
            assert!(!envrc.is_symlink(), ".envrc should NOT be a symlink");
            assert_eq!(
                fs::read_to_string(&envrc).unwrap(),
                "existing content",
                ".envrc should have original content"
            );

            // other.txt should be applied
            assert!(
                repo.path().join("other.txt").exists(),
                "other.txt should exist"
            );
        }

        #[test]
        fn default_fails_on_existing_repo_file() {
            let repo = create_test_repo();
            // Create a file that will conflict with the new overlay
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let overlay = create_test_overlay(&[(".envrc", "overlay content")]);

            let result = switch_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
                None,
                ConflictStrategy::default(),
                false,
            );
            assert!(
                result.is_err(),
                "switch without flags should fail on conflict"
            );
        }
    }

    // Tests for sync and sync --all functionality
    mod sync {
        use super::*;
        use crate::config::OverlayRepoConfig;
        use crate::overlay_repo::OverlayRepoManager;
        use crate::state::{
            EntryType, FileEntry, LinkType, OverlaySource, OverlayState, save_overlay_state,
        };

        /// Create a mock overlay repo directory with overlay files.
        /// Returns a `TempDir` that acts as the overlay repo root.
        #[allow(clippy::type_complexity)]
        fn create_mock_overlay_repo(overlays: &[(&str, &str, &str, &[(&str, &str)])]) -> TempDir {
            let dir = TempDir::new().unwrap();

            // Initialize as git repo (needed for OverlayRepoManager)
            Command::new("git")
                .args(["init"])
                .current_dir(dir.path())
                .output()
                .expect("Failed to init overlay repo");

            // Configure git user for commits
            Command::new("git")
                .args(["config", "user.email", "test@test.com"])
                .current_dir(dir.path())
                .output()
                .unwrap();
            Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(dir.path())
                .output()
                .unwrap();

            for (org, repo, overlay_name, files) in overlays {
                let overlay_path = dir.path().join(org).join(repo).join(overlay_name);
                fs::create_dir_all(&overlay_path).unwrap();

                for (file_path, content) in *files {
                    let full_path = overlay_path.join(file_path);
                    if let Some(parent) = full_path.parent() {
                        fs::create_dir_all(parent).unwrap();
                    }
                    fs::write(full_path, content).unwrap();
                }
            }

            // Initial commit so manager operations work
            Command::new("git")
                .args(["add", "."])
                .current_dir(dir.path())
                .output()
                .unwrap();
            Command::new("git")
                .args(["commit", "-m", "init", "--allow-empty"])
                .current_dir(dir.path())
                .output()
                .unwrap();

            dir
        }

        /// Create an `OverlayRepoManager` pointing at a local directory.
        fn create_test_manager(local_path: &std::path::Path) -> OverlayRepoManager {
            let config = OverlayRepoConfig {
                url: "https://example.com/test-repo.git".to_string(),
                local_path: Some(local_path.to_path_buf()),
            };
            OverlayRepoManager::new(config).unwrap()
        }

        /// Save a test overlay state with the given source to the target repo.
        fn save_test_state(
            target: &std::path::Path,
            name: &str,
            source: OverlaySource,
            files: Vec<(&str, &str)>,
        ) {
            let mut state = OverlayState::new(name.to_string(), source);
            for (src, tgt) in files {
                state.add_file(FileEntry {
                    source: PathBuf::from(src),
                    target: PathBuf::from(tgt),
                    link_type: LinkType::Copy,
                    entry_type: EntryType::File,
                });
            }
            save_overlay_state(target, &state).unwrap();
        }

        #[test]
        fn handle_sync_requires_name_or_all() {
            let repo = create_test_repo();
            let result = handle_sync(repo.path(), None, false, false);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Must specify an overlay name or use --all")
            );
        }

        #[test]
        fn handle_sync_fails_on_non_git_target() {
            let target = TempDir::new().unwrap();
            let result = handle_sync(target.path(), None, true, false);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not a git repository")
            );
        }

        #[test]
        fn handle_sync_all_with_no_overlays() {
            let repo = create_test_repo();
            // No overlays applied, should succeed with no-op
            let result = handle_sync(repo.path(), None, true, false);
            assert!(result.is_ok());
        }

        #[test]
        fn sync_single_overlay_copies_files_back() {
            let repo = create_test_repo();
            let overlay_repo = create_mock_overlay_repo(&[(
                "myorg",
                "myrepo",
                "my-overlay",
                &[(".envrc", "original content")],
            )]);

            // Create a file in the target repo (simulating an applied overlay)
            fs::write(repo.path().join(".envrc"), "modified content").unwrap();

            // Create overlay state
            let mut state = OverlayState::new(
                "my-overlay".to_string(),
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "my-overlay".to_string(),
                    "abc123".to_string(),
                ),
            );
            state.add_file(FileEntry {
                source: PathBuf::from(".envrc"),
                target: PathBuf::from(".envrc"),
                link_type: LinkType::Copy,
                entry_type: EntryType::File,
            });

            let manager = create_test_manager(overlay_repo.path());

            let result = sync_single_overlay(
                repo.path(),
                "myorg",
                "my-overlay",
                "myrepo",
                &state,
                &manager,
                false,
            );
            assert!(result.is_ok());

            // Verify the file was copied back to the overlay repo
            let overlay_file = overlay_repo
                .path()
                .join("myorg")
                .join("myrepo")
                .join("my-overlay")
                .join(".envrc");
            assert_eq!(
                fs::read_to_string(overlay_file).unwrap(),
                "modified content"
            );
        }

        #[test]
        fn sync_single_overlay_dry_run_does_not_modify() {
            let repo = create_test_repo();
            let overlay_repo = create_mock_overlay_repo(&[(
                "myorg",
                "myrepo",
                "my-overlay",
                &[(".envrc", "original content")],
            )]);

            fs::write(repo.path().join(".envrc"), "modified content").unwrap();

            let mut state = OverlayState::new(
                "my-overlay".to_string(),
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "my-overlay".to_string(),
                    "abc123".to_string(),
                ),
            );
            state.add_file(FileEntry {
                source: PathBuf::from(".envrc"),
                target: PathBuf::from(".envrc"),
                link_type: LinkType::Copy,
                entry_type: EntryType::File,
            });

            let manager = create_test_manager(overlay_repo.path());

            let result = sync_single_overlay(
                repo.path(),
                "myorg",
                "my-overlay",
                "myrepo",
                &state,
                &manager,
                true, // dry_run
            );
            assert!(result.is_ok());

            // Verify overlay repo file was NOT modified
            let overlay_file = overlay_repo
                .path()
                .join("myorg")
                .join("myrepo")
                .join("my-overlay")
                .join(".envrc");
            assert_eq!(
                fs::read_to_string(overlay_file).unwrap(),
                "original content"
            );
        }

        #[test]
        fn sync_single_overlay_fails_if_overlay_not_in_repo() {
            let repo = create_test_repo();
            let overlay_repo = create_mock_overlay_repo(&[]);

            let state = OverlayState::new(
                "missing-overlay".to_string(),
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "missing-overlay".to_string(),
                    "abc123".to_string(),
                ),
            );

            let manager = create_test_manager(overlay_repo.path());

            let result = sync_single_overlay(
                repo.path(),
                "myorg",
                "missing-overlay",
                "myrepo",
                &state,
                &manager,
                false,
            );
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("does not exist in overlay repo")
            );
        }

        #[test]
        fn sync_single_overlay_skips_missing_target_files() {
            let repo = create_test_repo();
            let overlay_repo = create_mock_overlay_repo(&[(
                "myorg",
                "myrepo",
                "my-overlay",
                &[(".envrc", "original")],
            )]);

            // Don't create the target file - simulates a deleted file

            let mut state = OverlayState::new(
                "my-overlay".to_string(),
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "my-overlay".to_string(),
                    "abc123".to_string(),
                ),
            );
            state.add_file(FileEntry {
                source: PathBuf::from(".envrc"),
                target: PathBuf::from(".envrc"),
                link_type: LinkType::Copy,
                entry_type: EntryType::File,
            });

            let manager = create_test_manager(overlay_repo.path());

            let result = sync_single_overlay(
                repo.path(),
                "myorg",
                "my-overlay",
                "myrepo",
                &state,
                &manager,
                false,
            );
            assert!(result.is_ok());

            // Overlay repo file should still have original content
            let overlay_file = overlay_repo
                .path()
                .join("myorg")
                .join("myrepo")
                .join("my-overlay")
                .join(".envrc");
            assert_eq!(fs::read_to_string(overlay_file).unwrap(), "original");
        }

        #[test]
        fn sync_single_overlay_copies_multiple_files() {
            let repo = create_test_repo();
            let overlay_repo = create_mock_overlay_repo(&[(
                "myorg",
                "myrepo",
                "my-overlay",
                &[(".envrc", "old1"), ("config.json", "old2")],
            )]);

            fs::write(repo.path().join(".envrc"), "new1").unwrap();
            fs::write(repo.path().join("config.json"), "new2").unwrap();

            let mut state = OverlayState::new(
                "my-overlay".to_string(),
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "my-overlay".to_string(),
                    "abc123".to_string(),
                ),
            );
            state.add_file(FileEntry {
                source: PathBuf::from(".envrc"),
                target: PathBuf::from(".envrc"),
                link_type: LinkType::Copy,
                entry_type: EntryType::File,
            });
            state.add_file(FileEntry {
                source: PathBuf::from("config.json"),
                target: PathBuf::from("config.json"),
                link_type: LinkType::Copy,
                entry_type: EntryType::File,
            });

            let manager = create_test_manager(overlay_repo.path());

            sync_single_overlay(
                repo.path(),
                "myorg",
                "my-overlay",
                "myrepo",
                &state,
                &manager,
                false,
            )
            .unwrap();

            let base = overlay_repo
                .path()
                .join("myorg")
                .join("myrepo")
                .join("my-overlay");
            assert_eq!(fs::read_to_string(base.join(".envrc")).unwrap(), "new1");
            assert_eq!(
                fs::read_to_string(base.join("config.json")).unwrap(),
                "new2"
            );
        }

        #[test]
        fn sync_single_overlay_creates_parent_directories() {
            let repo = create_test_repo();
            let overlay_repo = create_mock_overlay_repo(&[(
                "myorg",
                "myrepo",
                "my-overlay",
                &[], // No files initially
            )]);

            // Create a nested file in the target repo
            let nested_dir = repo.path().join(".claude");
            fs::create_dir_all(&nested_dir).unwrap();
            fs::write(nested_dir.join("CLAUDE.md"), "# Config").unwrap();

            let mut state = OverlayState::new(
                "my-overlay".to_string(),
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "my-overlay".to_string(),
                    "abc123".to_string(),
                ),
            );
            state.add_file(FileEntry {
                source: PathBuf::from(".claude/CLAUDE.md"),
                target: PathBuf::from(".claude/CLAUDE.md"),
                link_type: LinkType::Copy,
                entry_type: EntryType::File,
            });

            let manager = create_test_manager(overlay_repo.path());

            sync_single_overlay(
                repo.path(),
                "myorg",
                "my-overlay",
                "myrepo",
                &state,
                &manager,
                false,
            )
            .unwrap();

            let overlay_file = overlay_repo
                .path()
                .join("myorg")
                .join("myrepo")
                .join("my-overlay")
                .join(".claude")
                .join("CLAUDE.md");
            assert!(overlay_file.exists());
            assert_eq!(fs::read_to_string(overlay_file).unwrap(), "# Config");
        }

        #[test]
        fn handle_sync_all_skips_local_sources() {
            let repo = create_test_repo();

            // Create overlay state files with a local source
            save_test_state(
                repo.path(),
                "local-overlay",
                OverlaySource::local(PathBuf::from("/some/path")),
                vec![(".envrc", ".envrc")],
            );

            // handle_sync --all should succeed (returns Ok) but needs config
            // for the manager. Since there are no OverlayRepo sources to sync,
            // we test that the state files with non-OverlayRepo sources are
            // correctly identified by listing applied overlays.
            let applied = crate::list_applied_overlays(repo.path()).unwrap();
            assert_eq!(applied.len(), 1);
            assert_eq!(applied[0], "local-overlay");

            // Verify the source is Local (which would be skipped by --all)
            let state = crate::load_overlay_state(repo.path(), "local-overlay").unwrap();
            assert!(!state.source.is_overlay_repo());
        }

        #[test]
        fn handle_sync_all_skips_github_sources() {
            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "github-overlay",
                OverlaySource::github(
                    "https://github.com/org/repo".to_string(),
                    "org".to_string(),
                    "repo".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    None,
                ),
                vec![(".envrc", ".envrc")],
            );

            let state = crate::load_overlay_state(repo.path(), "github-overlay").unwrap();
            assert!(state.source.is_github());
            assert!(!state.source.is_overlay_repo());
        }

        #[test]
        fn handle_sync_all_identifies_overlay_repo_sources() {
            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "repo-overlay",
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "repo-overlay".to_string(),
                    "abc123".to_string(),
                ),
                vec![(".envrc", ".envrc")],
            );

            let state = crate::load_overlay_state(repo.path(), "repo-overlay").unwrap();
            assert!(state.source.is_overlay_repo());
        }

        #[test]
        fn handle_sync_all_with_mixed_sources_identifies_correctly() {
            let repo = create_test_repo();

            // Create overlays with different source types
            save_test_state(
                repo.path(),
                "local-one",
                OverlaySource::local(PathBuf::from("/path")),
                vec![("a.txt", "a.txt")],
            );
            save_test_state(
                repo.path(),
                "github-one",
                OverlaySource::github(
                    "https://github.com/o/r".to_string(),
                    "o".to_string(),
                    "r".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    None,
                ),
                vec![("b.txt", "b.txt")],
            );
            save_test_state(
                repo.path(),
                "repo-one",
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "repo-one".to_string(),
                    "abc123".to_string(),
                ),
                vec![("c.txt", "c.txt")],
            );

            let applied = crate::list_applied_overlays(repo.path()).unwrap();
            assert_eq!(applied.len(), 3);

            // Verify source type classification for each
            let mut overlay_repo_count = 0;
            let mut skipped_count = 0;
            for name in &applied {
                let state = crate::load_overlay_state(repo.path(), name.as_str()).unwrap();
                if state.source.is_overlay_repo() {
                    overlay_repo_count += 1;
                } else {
                    skipped_count += 1;
                }
            }
            assert_eq!(overlay_repo_count, 1);
            assert_eq!(skipped_count, 2);
        }

        #[test]
        fn parse_github_overlay_subpath_valid() {
            let result = parse_github_overlay_subpath("microsoft/FluidFramework/claude-config");
            assert!(result.is_some());
            let (org, repo, name) = result.unwrap();
            assert_eq!(org, "microsoft");
            assert_eq!(repo, "FluidFramework");
            assert_eq!(name, "claude-config");
        }

        #[test]
        fn parse_github_overlay_subpath_invalid() {
            // Two parts
            assert!(parse_github_overlay_subpath("org/repo").is_none());
            // One part
            assert!(parse_github_overlay_subpath("overlay").is_none());
            // Four parts
            assert!(parse_github_overlay_subpath("a/b/c/d").is_none());
            // Empty parts
            assert!(parse_github_overlay_subpath("org//name").is_none());
            assert!(parse_github_overlay_subpath("/repo/name").is_none());
            assert!(parse_github_overlay_subpath("org/repo/").is_none());
        }

        #[test]
        fn is_overlay_repo_url_matches_https() {
            assert!(is_overlay_repo_url(
                "https://github.com/tylerbutler/repo-overlays",
                "tylerbutler",
                "repo-overlays"
            ));
        }

        #[test]
        fn is_overlay_repo_url_matches_with_git_suffix() {
            assert!(is_overlay_repo_url(
                "https://github.com/tylerbutler/repo-overlays.git",
                "tylerbutler",
                "repo-overlays"
            ));
        }

        #[test]
        fn is_overlay_repo_url_no_match_different_owner() {
            assert!(!is_overlay_repo_url(
                "https://github.com/other/repo-overlays",
                "tylerbutler",
                "repo-overlays"
            ));
        }

        #[test]
        fn is_overlay_repo_url_no_match_different_repo() {
            assert!(!is_overlay_repo_url(
                "https://github.com/tylerbutler/other-repo",
                "tylerbutler",
                "repo-overlays"
            ));
        }

        #[test]
        fn github_source_from_overlay_repo_is_syncable() {
            // When an overlay is applied via two-part browse mode from the overlay repo,
            // it should be recognized as syncable (not skipped) by sync --all.
            let repo = create_test_repo();

            // This is what happens when a user applies via `repoverlay apply tylerbutler/repo-overlays`
            // The source is stored as GitHub with the overlay identity in the subpath.
            save_test_state(
                repo.path(),
                "build-troubleshooter",
                OverlaySource::github(
                    "https://github.com/tylerbutler/repo-overlays".to_string(),
                    "tylerbutler".to_string(),
                    "repo-overlays".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    Some("microsoft/FluidFramework/build-troubleshooter".to_string()),
                ),
                vec![(".envrc", ".envrc")],
            );

            let state = crate::load_overlay_state(repo.path(), "build-troubleshooter").unwrap();
            // Source is GitHub, not OverlayRepo
            assert!(state.source.is_github());
            assert!(!state.source.is_overlay_repo());

            // But subpath should be parseable as an overlay reference
            if let OverlaySource::GitHub { subpath, .. } = &state.source {
                let parsed = subpath.as_deref().and_then(parse_github_overlay_subpath);
                assert!(parsed.is_some());
                let (org, repo, name) = parsed.unwrap();
                assert_eq!(org, "microsoft");
                assert_eq!(repo, "FluidFramework");
                assert_eq!(name, "build-troubleshooter");
            } else {
                panic!("Expected GitHub source");
            }
        }

        #[test]
        fn github_source_without_subpath_is_not_syncable() {
            // A GitHub source without a subpath (direct repo apply) should still be skipped
            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "direct-github",
                OverlaySource::github(
                    "https://github.com/someone/some-overlay".to_string(),
                    "someone".to_string(),
                    "some-overlay".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    None,
                ),
                vec![(".envrc", ".envrc")],
            );

            let state = crate::load_overlay_state(repo.path(), "direct-github").unwrap();
            if let OverlaySource::GitHub { subpath, .. } = &state.source {
                let parsed = subpath.as_deref().and_then(parse_github_overlay_subpath);
                assert!(parsed.is_none());
            } else {
                panic!("Expected GitHub source");
            }
        }
    }

    // CLI structure and parsing tests using clap's try_parse_from()
    // These tests validate CLI behavior without running the binary.
    mod cli_parsing {
        use super::*;
        use clap::CommandFactory;

        #[test]
        fn cli_structure_is_valid() {
            // Validate CLI structure - will panic if structure is invalid
            Cli::command().debug_assert();
        }

        #[test]
        fn apply_parses_source_argument() {
            let cli = Cli::try_parse_from(["repoverlay", "apply", "./my-overlay"]).unwrap();

            match cli.command {
                Some(Commands::Apply { source, .. }) => {
                    assert_eq!(source, "./my-overlay");
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_parses_all_options() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "apply",
                "./overlay",
                "--target",
                "/path/to/repo",
                "--copy",
                "--name",
                "my-name",
                "--ref",
                "main",
                "--update",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Apply {
                    source,
                    target,
                    copy,
                    name,
                    r#ref,
                    update,
                    force,
                    skip_conflicts,
                    interactive,
                    merge: _,
                    from_source,
                    dry_run,
                }) => {
                    assert_eq!(source, "./overlay");
                    assert_eq!(target, Some(PathBuf::from("/path/to/repo")));
                    assert!(copy);
                    assert_eq!(name, Some("my-name".to_string()));
                    assert_eq!(r#ref, Some("main".to_string()));
                    assert!(update);
                    assert!(!force);
                    assert!(!skip_conflicts);
                    assert!(!interactive);
                    assert!(from_source.is_none());
                    assert!(!dry_run);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_parses_dry_run() {
            let cli =
                Cli::try_parse_from(["repoverlay", "apply", "./overlay", "--dry-run"]).unwrap();

            match cli.command {
                Some(Commands::Apply { dry_run, .. }) => {
                    assert!(dry_run);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_parses_force_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "apply", "./overlay", "--force"]).unwrap();

            match cli.command {
                Some(Commands::Apply {
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(force);
                    assert!(!skip_conflicts);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_parses_skip_conflicts_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "apply", "./overlay", "--skip-conflicts"])
                .unwrap();

            match cli.command {
                Some(Commands::Apply {
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(!force);
                    assert!(skip_conflicts);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_rejects_force_and_skip_conflicts_together() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "apply",
                "./overlay",
                "--force",
                "--skip-conflicts",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn apply_parses_interactive_flag() {
            let cli =
                Cli::try_parse_from(["repoverlay", "apply", "./overlay", "--interactive"]).unwrap();
            match cli.command {
                Some(Commands::Apply {
                    force,
                    skip_conflicts,
                    interactive,
                    ..
                }) => {
                    assert!(!force);
                    assert!(!skip_conflicts);
                    assert!(interactive);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_parses_interactive_short_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "apply", "./overlay", "-i"]).unwrap();
            match cli.command {
                Some(Commands::Apply { interactive, .. }) => {
                    assert!(interactive);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_rejects_force_and_interactive_together() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "apply",
                "./overlay",
                "--force",
                "--interactive",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn apply_rejects_skip_conflicts_and_interactive_together() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "apply",
                "./overlay",
                "--skip-conflicts",
                "--interactive",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn apply_parses_merge_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "apply", "./overlay", "--merge"]).unwrap();
            match cli.command {
                Some(Commands::Apply { merge, .. }) => {
                    assert!(merge);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_merge_combinable_with_force() {
            let cli =
                Cli::try_parse_from(["repoverlay", "apply", "./overlay", "--merge", "--force"])
                    .unwrap();
            match cli.command {
                Some(Commands::Apply { merge, force, .. }) => {
                    assert!(merge);
                    assert!(force);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_merge_combinable_with_skip_conflicts() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "apply",
                "./overlay",
                "--merge",
                "--skip-conflicts",
            ])
            .unwrap();
            match cli.command {
                Some(Commands::Apply {
                    merge,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(merge);
                    assert!(skip_conflicts);
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn apply_requires_source() {
            let result = Cli::try_parse_from(["repoverlay", "apply"]);
            assert!(result.is_err());
        }

        #[test]
        fn remove_parses_name_argument() {
            let cli = Cli::try_parse_from(["repoverlay", "remove", "my-overlay"]).unwrap();

            match cli.command {
                Some(Commands::Remove { name, all, .. }) => {
                    assert_eq!(name, Some("my-overlay".to_string()));
                    assert!(!all);
                }
                _ => panic!("Expected Remove command"),
            }
        }

        #[test]
        fn remove_parses_all_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "remove", "--all"]).unwrap();

            match cli.command {
                Some(Commands::Remove { name, all, .. }) => {
                    assert!(name.is_none());
                    assert!(all);
                }
                _ => panic!("Expected Remove command"),
            }
        }

        #[test]
        fn remove_parses_dry_run() {
            let cli =
                Cli::try_parse_from(["repoverlay", "remove", "my-overlay", "--dry-run"]).unwrap();

            match cli.command {
                Some(Commands::Remove { dry_run, .. }) => {
                    assert!(dry_run);
                }
                _ => panic!("Expected Remove command"),
            }
        }

        #[test]
        fn remove_parses_interactive() {
            let cli = Cli::try_parse_from(["repoverlay", "remove", "--interactive"]).unwrap();

            match cli.command {
                Some(Commands::Remove { interactive, .. }) => {
                    assert!(interactive);
                }
                _ => panic!("Expected Remove command"),
            }
        }

        #[test]
        fn remove_parses_short_interactive() {
            let cli = Cli::try_parse_from(["repoverlay", "remove", "-i"]).unwrap();

            match cli.command {
                Some(Commands::Remove { interactive, .. }) => {
                    assert!(interactive);
                }
                _ => panic!("Expected Remove command"),
            }
        }

        #[test]
        fn status_parses_without_arguments() {
            let cli = Cli::try_parse_from(["repoverlay", "status"]).unwrap();

            match cli.command {
                Some(Commands::Status {
                    target,
                    name,
                    json,
                    quiet,
                }) => {
                    assert!(target.is_none());
                    assert!(name.is_none());
                    assert!(!json);
                    assert!(!quiet);
                }
                _ => panic!("Expected Status command"),
            }
        }

        #[test]
        fn status_parses_name_filter() {
            let cli =
                Cli::try_parse_from(["repoverlay", "status", "--name", "my-overlay"]).unwrap();

            match cli.command {
                Some(Commands::Status { name, .. }) => {
                    assert_eq!(name, Some("my-overlay".to_string()));
                }
                _ => panic!("Expected Status command"),
            }
        }

        #[test]
        fn status_parses_json_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "status", "--json"]).unwrap();

            match cli.command {
                Some(Commands::Status { json, .. }) => {
                    assert!(json);
                }
                _ => panic!("Expected Status command"),
            }
        }

        #[test]
        fn status_parses_quiet_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "status", "--quiet"]).unwrap();

            match cli.command {
                Some(Commands::Status { quiet, .. }) => {
                    assert!(quiet);
                }
                _ => panic!("Expected Status command"),
            }
        }

        #[test]
        fn status_parses_short_quiet() {
            let cli = Cli::try_parse_from(["repoverlay", "status", "-q"]).unwrap();

            match cli.command {
                Some(Commands::Status { quiet, .. }) => {
                    assert!(quiet);
                }
                _ => panic!("Expected Status command"),
            }
        }

        #[test]
        fn status_json_with_name_filter() {
            let cli =
                Cli::try_parse_from(["repoverlay", "status", "--json", "--name", "my-overlay"])
                    .unwrap();

            match cli.command {
                Some(Commands::Status { json, name, .. }) => {
                    assert!(json);
                    assert_eq!(name, Some("my-overlay".to_string()));
                }
                _ => panic!("Expected Status command"),
            }
        }

        #[test]
        fn restore_parses_dry_run() {
            let cli = Cli::try_parse_from(["repoverlay", "restore", "--dry-run"]).unwrap();

            match cli.command {
                Some(Commands::Restore { dry_run, .. }) => {
                    assert!(dry_run);
                }
                _ => panic!("Expected Restore command"),
            }
        }

        #[test]
        fn restore_parses_force_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "restore", "--force"]).unwrap();

            match cli.command {
                Some(Commands::Restore {
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(force);
                    assert!(!skip_conflicts);
                }
                _ => panic!("Expected Restore command"),
            }
        }

        #[test]
        fn restore_parses_skip_conflicts_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "restore", "--skip-conflicts"]).unwrap();

            match cli.command {
                Some(Commands::Restore {
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(!force);
                    assert!(skip_conflicts);
                }
                _ => panic!("Expected Restore command"),
            }
        }

        #[test]
        fn restore_rejects_force_and_skip_conflicts_together() {
            let result =
                Cli::try_parse_from(["repoverlay", "restore", "--force", "--skip-conflicts"]);
            assert!(result.is_err());
        }

        #[test]
        fn restore_parses_interactive_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "restore", "--interactive"]).unwrap();
            match cli.command {
                Some(Commands::Restore {
                    interactive,
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(interactive);
                    assert!(!force);
                    assert!(!skip_conflicts);
                }
                _ => panic!("Expected Restore command"),
            }
        }

        #[test]
        fn restore_rejects_force_and_interactive_together() {
            let result = Cli::try_parse_from(["repoverlay", "restore", "--force", "--interactive"]);
            assert!(result.is_err());
        }

        #[test]
        fn update_parses_overlay_name() {
            let cli = Cli::try_parse_from(["repoverlay", "update", "my-overlay"]).unwrap();

            match cli.command {
                Some(Commands::Update { name, dry_run, .. }) => {
                    assert_eq!(name, Some("my-overlay".to_string()));
                    assert!(!dry_run);
                }
                _ => panic!("Expected Update command"),
            }
        }

        #[test]
        fn update_parses_force_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "update", "--force"]).unwrap();

            match cli.command {
                Some(Commands::Update {
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(force);
                    assert!(!skip_conflicts);
                }
                _ => panic!("Expected Update command"),
            }
        }

        #[test]
        fn update_parses_skip_conflicts_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "update", "--skip-conflicts"]).unwrap();

            match cli.command {
                Some(Commands::Update {
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(!force);
                    assert!(skip_conflicts);
                }
                _ => panic!("Expected Update command"),
            }
        }

        #[test]
        fn update_rejects_force_and_skip_conflicts_together() {
            let result =
                Cli::try_parse_from(["repoverlay", "update", "--force", "--skip-conflicts"]);
            assert!(result.is_err());
        }

        #[test]
        fn update_parses_interactive_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "update", "--interactive"]).unwrap();
            match cli.command {
                Some(Commands::Update {
                    interactive,
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(interactive);
                    assert!(!force);
                    assert!(!skip_conflicts);
                }
                _ => panic!("Expected Update command"),
            }
        }

        #[test]
        fn update_rejects_force_and_interactive_together() {
            let result = Cli::try_parse_from(["repoverlay", "update", "--force", "--interactive"]);
            assert!(result.is_err());
        }

        #[test]
        fn create_parses_options() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "create",
                "my-overlay",
                "--include",
                ".envrc",
                "--include",
                ".vscode",
                "--force",
                "--yes",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Create {
                    name,
                    include,
                    force,
                    yes,
                    ..
                }) => {
                    assert_eq!(name, Some("my-overlay".to_string()));
                    assert_eq!(include.len(), 2);
                    assert!(force);
                    assert!(yes);
                }
                _ => panic!("Expected Create command"),
            }
        }

        #[test]
        fn create_with_output_flag() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "create",
                "--output",
                "./output",
                "--include",
                ".envrc",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Create { name, output, .. }) => {
                    assert!(name.is_none());
                    assert_eq!(output, Some(PathBuf::from("./output")));
                }
                _ => panic!("Expected Create command"),
            }
        }

        #[test]
        fn create_accepts_no_args() {
            // create without name or --output is valid at parse time (error at runtime)
            let cli = Cli::try_parse_from(["repoverlay", "create"]);
            assert!(cli.is_ok());
        }

        #[test]
        fn create_local_parses_options() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "create-local",
                "./output",
                "--include",
                ".envrc",
                "--yes",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::CreateLocal {
                    output,
                    include,
                    yes,
                    ..
                }) => {
                    assert_eq!(output, PathBuf::from("./output"));
                    assert_eq!(include.len(), 1);
                    assert!(yes);
                }
                _ => panic!("Expected CreateLocal command"),
            }
        }

        #[test]
        fn switch_parses_source() {
            let cli = Cli::try_parse_from(["repoverlay", "switch", "./new-overlay"]).unwrap();

            match cli.command {
                Some(Commands::Switch { source, .. }) => {
                    assert_eq!(source, "./new-overlay");
                }
                _ => panic!("Expected Switch command"),
            }
        }

        #[test]
        fn switch_parses_force_flag() {
            let cli =
                Cli::try_parse_from(["repoverlay", "switch", "./overlay", "--force"]).unwrap();

            match cli.command {
                Some(Commands::Switch {
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(force);
                    assert!(!skip_conflicts);
                }
                _ => panic!("Expected Switch command"),
            }
        }

        #[test]
        fn switch_parses_skip_conflicts_flag() {
            let cli =
                Cli::try_parse_from(["repoverlay", "switch", "./overlay", "--skip-conflicts"])
                    .unwrap();

            match cli.command {
                Some(Commands::Switch {
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(!force);
                    assert!(skip_conflicts);
                }
                _ => panic!("Expected Switch command"),
            }
        }

        #[test]
        fn switch_rejects_force_and_skip_conflicts_together() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "switch",
                "./overlay",
                "--force",
                "--skip-conflicts",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn switch_parses_interactive_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "switch", "./overlay", "--interactive"])
                .unwrap();
            match cli.command {
                Some(Commands::Switch {
                    interactive,
                    force,
                    skip_conflicts,
                    ..
                }) => {
                    assert!(interactive);
                    assert!(!force);
                    assert!(!skip_conflicts);
                }
                _ => panic!("Expected Switch command"),
            }
        }

        #[test]
        fn switch_rejects_force_and_interactive_together() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "switch",
                "./overlay",
                "--force",
                "--interactive",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn list_parses_filter() {
            let cli = Cli::try_parse_from(["repoverlay", "list", "--filter", "org/repo"]).unwrap();

            match cli.command {
                Some(Commands::List { filter, .. }) => {
                    assert_eq!(filter, Some("org/repo".to_string()));
                }
                _ => panic!("Expected List command"),
            }
        }

        #[test]
        fn list_parses_target_alias() {
            let cli = Cli::try_parse_from(["repoverlay", "list", "--target", "org/repo"]).unwrap();

            match cli.command {
                Some(Commands::List { filter, .. }) => {
                    assert_eq!(filter, Some("org/repo".to_string()));
                }
                _ => panic!("Expected List command"),
            }
        }

        #[test]
        fn browse_parses_filter() {
            let cli =
                Cli::try_parse_from(["repoverlay", "browse", "--filter", "org/repo"]).unwrap();

            match cli.command {
                Some(Commands::Browse { filter, .. }) => {
                    assert_eq!(filter, Some("org/repo".to_string()));
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn browse_parses_show_all() {
            let cli = Cli::try_parse_from(["repoverlay", "browse", "--show-all"]).unwrap();

            match cli.command {
                Some(Commands::Browse { show_all, .. }) => {
                    assert!(show_all);
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn browse_parses_target() {
            let cli =
                Cli::try_parse_from(["repoverlay", "browse", "--target", "/some/path"]).unwrap();

            match cli.command {
                Some(Commands::Browse { target, .. }) => {
                    assert_eq!(target, Some(PathBuf::from("/some/path")));
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn browse_parses_no_interactive() {
            let cli = Cli::try_parse_from(["repoverlay", "browse", "--no-interactive"]).unwrap();

            match cli.command {
                Some(Commands::Browse { no_interactive, .. }) => {
                    assert!(no_interactive);
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn browse_parses_dry_run() {
            let cli = Cli::try_parse_from(["repoverlay", "browse", "--dry-run"]).unwrap();

            match cli.command {
                Some(Commands::Browse { dry_run, .. }) => {
                    assert!(dry_run);
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn browse_defaults_show_all_false() {
            let cli = Cli::try_parse_from(["repoverlay", "browse"]).unwrap();

            match cli.command {
                Some(Commands::Browse { show_all, .. }) => {
                    assert!(!show_all);
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn browse_parses_source_argument() {
            let cli = Cli::try_parse_from(["repoverlay", "browse", "tylerbutler"]).unwrap();
            match cli.command {
                Some(Commands::Browse { source, .. }) => {
                    assert_eq!(source, Some("tylerbutler".to_string()));
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn browse_source_is_optional() {
            let cli = Cli::try_parse_from(["repoverlay", "browse"]).unwrap();
            match cli.command {
                Some(Commands::Browse { source, .. }) => {
                    assert!(source.is_none());
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn browse_source_with_other_flags() {
            let cli =
                Cli::try_parse_from(["repoverlay", "browse", "tylerbutler", "--show-all"]).unwrap();
            match cli.command {
                Some(Commands::Browse {
                    source, show_all, ..
                }) => {
                    assert_eq!(source, Some("tylerbutler".to_string()));
                    assert!(show_all);
                }
                _ => panic!("Expected Browse command"),
            }
        }

        #[test]
        fn cache_list_subcommand() {
            let cli = Cli::try_parse_from(["repoverlay", "cache", "list"]).unwrap();

            match cli.command {
                Some(Commands::Cache { command }) => match command {
                    CacheCommand::List => {}
                    _ => panic!("Expected Cache List subcommand"),
                },
                _ => panic!("Expected Cache command"),
            }
        }

        #[test]
        fn cache_clear_subcommand() {
            let cli = Cli::try_parse_from(["repoverlay", "cache", "clear"]).unwrap();

            match cli.command {
                Some(Commands::Cache { command }) => match command {
                    CacheCommand::Clear { yes } => {
                        assert!(!yes, "default yes should be false");
                    }
                    _ => panic!("Expected Cache Clear subcommand"),
                },
                _ => panic!("Expected Cache command"),
            }
        }

        #[test]
        fn cache_clear_with_yes_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "cache", "clear", "--yes"]).unwrap();

            match cli.command {
                Some(Commands::Cache { command }) => match command {
                    CacheCommand::Clear { yes } => {
                        assert!(yes, "yes flag should be true");
                    }
                    _ => panic!("Expected Cache Clear subcommand"),
                },
                _ => panic!("Expected Cache command"),
            }
        }

        #[test]
        fn cache_remove_no_args_parses_ok() {
            // No args is valid at parse time (error at runtime)
            let cli = Cli::try_parse_from(["repoverlay", "cache", "remove"]);
            assert!(cli.is_ok());
        }

        #[test]
        fn cache_remove_parses_repo() {
            let cli = Cli::try_parse_from(["repoverlay", "cache", "remove", "owner/repo"]).unwrap();

            match cli.command {
                Some(Commands::Cache { command }) => match command {
                    CacheCommand::Remove { repo, all, .. } => {
                        assert_eq!(repo, Some("owner/repo".to_string()));
                        assert!(!all);
                    }
                    _ => panic!("Expected Cache Remove subcommand"),
                },
                _ => panic!("Expected Cache command"),
            }
        }

        #[test]
        fn cache_remove_all_flag() {
            let cli =
                Cli::try_parse_from(["repoverlay", "cache", "remove", "--all", "--yes"]).unwrap();

            match cli.command {
                Some(Commands::Cache { command }) => match command {
                    CacheCommand::Remove { repo, all, yes } => {
                        assert!(repo.is_none());
                        assert!(all);
                        assert!(yes);
                    }
                    _ => panic!("Expected Cache Remove subcommand"),
                },
                _ => panic!("Expected Cache command"),
            }
        }

        #[test]
        fn cache_path_subcommand() {
            let cli = Cli::try_parse_from(["repoverlay", "cache", "path"]).unwrap();

            match cli.command {
                Some(Commands::Cache { command }) => match command {
                    CacheCommand::Path => {}
                    _ => panic!("Expected Cache Path subcommand"),
                },
                _ => panic!("Expected Cache command"),
            }
        }

        #[test]
        fn invalid_command_rejected() {
            let result = Cli::try_parse_from(["repoverlay", "nonexistent"]);
            assert!(result.is_err());
        }

        #[test]
        fn unknown_flag_rejected() {
            let result = Cli::try_parse_from(["repoverlay", "apply", "--unknown-flag", "source"]);
            assert!(result.is_err());
        }

        #[test]
        fn short_flags_work() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "apply",
                "./overlay",
                "-t",
                "/repo",
                "-n",
                "name",
                "-r",
                "main",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Apply {
                    target,
                    name,
                    r#ref,
                    ..
                }) => {
                    assert_eq!(target, Some(PathBuf::from("/repo")));
                    assert_eq!(name, Some("name".to_string()));
                    assert_eq!(r#ref, Some("main".to_string()));
                }
                _ => panic!("Expected Apply command"),
            }
        }

        #[test]
        fn add_requires_name() {
            let result = Cli::try_parse_from(["repoverlay", "add"]);
            assert!(result.is_err());
        }

        #[test]
        fn add_parses_name_and_files() {
            let cli =
                Cli::try_parse_from(["repoverlay", "add", "my-overlay", "file1.txt", "file2.txt"])
                    .unwrap();

            match cli.command {
                Some(Commands::Add {
                    name,
                    files,
                    target,
                    dry_run,
                }) => {
                    assert_eq!(name, "my-overlay");
                    assert_eq!(files.len(), 2);
                    assert_eq!(files[0], PathBuf::from("file1.txt"));
                    assert_eq!(files[1], PathBuf::from("file2.txt"));
                    assert!(target.is_none());
                    assert!(!dry_run);
                }
                _ => panic!("Expected Add command"),
            }
        }

        #[test]
        fn add_parses_all_options() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "add",
                "org/repo/my-overlay",
                "newfile.txt",
                "--target",
                "/repo",
                "--dry-run",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Add {
                    name,
                    files,
                    target,
                    dry_run,
                }) => {
                    assert_eq!(name, "org/repo/my-overlay");
                    assert_eq!(files, vec![PathBuf::from("newfile.txt")]);
                    assert_eq!(target, Some(PathBuf::from("/repo")));
                    assert!(dry_run);
                }
                _ => panic!("Expected Add command"),
            }
        }

        #[test]
        fn add_accepts_short_target_flag() {
            let cli =
                Cli::try_parse_from(["repoverlay", "add", "my-overlay", "file.txt", "-t", "/repo"])
                    .unwrap();

            match cli.command {
                Some(Commands::Add { target, .. }) => {
                    assert_eq!(target, Some(PathBuf::from("/repo")));
                }
                _ => panic!("Expected Add command"),
            }
        }

        #[test]
        fn add_accepts_multiple_files() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "add",
                "my-overlay",
                "file1.txt",
                "file2.txt",
                "dir/file3.txt",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Add { files, .. }) => {
                    assert_eq!(files.len(), 3);
                    assert_eq!(files[0], PathBuf::from("file1.txt"));
                    assert_eq!(files[1], PathBuf::from("file2.txt"));
                    assert_eq!(files[2], PathBuf::from("dir/file3.txt"));
                }
                _ => panic!("Expected Add command"),
            }
        }

        #[test]
        fn add_accepts_files_with_special_characters() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "add",
                "my-overlay",
                "file with spaces.txt",
                ".hidden-file",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Add { files, .. }) => {
                    assert_eq!(files.len(), 2);
                    assert_eq!(files[0], PathBuf::from("file with spaces.txt"));
                    assert_eq!(files[1], PathBuf::from(".hidden-file"));
                }
                _ => panic!("Expected Add command"),
            }
        }

        #[test]
        fn add_dry_run_defaults_to_false() {
            let cli = Cli::try_parse_from(["repoverlay", "add", "my-overlay", "file.txt"]).unwrap();

            match cli.command {
                Some(Commands::Add { dry_run, .. }) => {
                    assert!(!dry_run);
                }
                _ => panic!("Expected Add command"),
            }
        }

        #[test]
        fn add_target_defaults_to_none() {
            let cli = Cli::try_parse_from(["repoverlay", "add", "my-overlay", "file.txt"]).unwrap();

            match cli.command {
                Some(Commands::Add { target, .. }) => {
                    assert!(target.is_none());
                }
                _ => panic!("Expected Add command"),
            }
        }

        #[test]
        fn edit_requires_name() {
            let result = Cli::try_parse_from(["repoverlay", "edit"]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_parses_add_flag() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--add",
                "file1.txt",
                "file2.txt",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit {
                    name,
                    add,
                    remove,
                    interactive,
                    dry_run,
                    ..
                }) => {
                    assert_eq!(name, "my-overlay");
                    assert_eq!(
                        add,
                        vec![PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]
                    );
                    assert!(remove.is_empty());
                    assert!(!interactive);
                    assert!(!dry_run);
                }
                _ => panic!("Expected Edit command"),
            }
        }

        #[test]
        fn edit_parses_remove_flag() {
            let cli =
                Cli::try_parse_from(["repoverlay", "edit", "my-overlay", "--remove", "file1.txt"])
                    .unwrap();

            match cli.command {
                Some(Commands::Edit { remove, .. }) => {
                    assert_eq!(remove, vec![PathBuf::from("file1.txt")]);
                }
                _ => panic!("Expected Edit command"),
            }
        }

        #[test]
        fn edit_parses_combined_add_remove() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--add",
                "new.txt",
                "--remove",
                "old.txt",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit { add, remove, .. }) => {
                    assert_eq!(add, vec![PathBuf::from("new.txt")]);
                    assert_eq!(remove, vec![PathBuf::from("old.txt")]);
                }
                _ => panic!("Expected Edit command"),
            }
        }

        #[test]
        fn edit_interactive_conflicts_with_add() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--interactive",
                "--add",
                "file.txt",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_interactive_conflicts_with_remove() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--interactive",
                "--remove",
                "file.txt",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_parses_dry_run() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--add",
                "f.txt",
                "--dry-run",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit { dry_run, .. }) => {
                    assert!(dry_run);
                }
                _ => panic!("Expected Edit command"),
            }
        }

        #[test]
        fn edit_parses_target_flag() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--add",
                "f.txt",
                "-t",
                "/repo",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit { target, .. }) => {
                    assert_eq!(target, Some(PathBuf::from("/repo")));
                }
                _ => panic!("Expected Edit command"),
            }
        }

        #[test]
        fn sync_parses_name() {
            let cli = Cli::try_parse_from(["repoverlay", "sync", "my-overlay"]).unwrap();

            match cli.command {
                Some(Commands::Sync { name, all, .. }) => {
                    assert_eq!(name, Some("my-overlay".to_string()));
                    assert!(!all);
                }
                _ => panic!("Expected Sync command"),
            }
        }

        #[test]
        fn sync_parses_all_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "sync", "--all"]).unwrap();

            match cli.command {
                Some(Commands::Sync { name, all, .. }) => {
                    assert!(name.is_none());
                    assert!(all);
                }
                _ => panic!("Expected Sync command"),
            }
        }

        #[test]
        fn sync_parses_dry_run() {
            let cli =
                Cli::try_parse_from(["repoverlay", "sync", "my-overlay", "--dry-run"]).unwrap();

            match cli.command {
                Some(Commands::Sync { dry_run, .. }) => {
                    assert!(dry_run);
                }
                _ => panic!("Expected Sync command"),
            }
        }

        #[test]
        fn sync_parses_all_with_dry_run() {
            let cli = Cli::try_parse_from(["repoverlay", "sync", "--all", "--dry-run"]).unwrap();

            match cli.command {
                Some(Commands::Sync {
                    name, all, dry_run, ..
                }) => {
                    assert!(name.is_none());
                    assert!(all);
                    assert!(dry_run);
                }
                _ => panic!("Expected Sync command"),
            }
        }

        #[test]
        fn sync_parses_no_args() {
            let cli = Cli::try_parse_from(["repoverlay", "sync"]).unwrap();

            match cli.command {
                Some(Commands::Sync { name, all, .. }) => {
                    assert!(name.is_none());
                    assert!(!all);
                }
                _ => panic!("Expected Sync command"),
            }
        }

        #[test]
        fn sync_parses_target_flag() {
            let cli = Cli::try_parse_from(["repoverlay", "sync", "--all", "-t", "/repo"]).unwrap();

            match cli.command {
                Some(Commands::Sync { target, all, .. }) => {
                    assert_eq!(target, Some(PathBuf::from("/repo")));
                    assert!(all);
                }
                _ => panic!("Expected Sync command"),
            }
        }
    }

    // Additional parse_overlay_name_arg edge cases
    mod parse_overlay_name_arg_additional_tests {
        use super::*;
        use std::path::Path;

        #[test]
        fn two_slash_path_with_hyphens_and_underscores() {
            let result =
                parse_overlay_name_arg("my-org/my_repo/my-overlay_v2", Path::new("/tmp")).unwrap();
            assert_eq!(
                result,
                (
                    "my-org".to_string(),
                    "my_repo".to_string(),
                    "my-overlay_v2".to_string()
                )
            );
        }

        #[test]
        fn five_slash_path_errors() {
            let source = create_test_repo();
            let result = parse_overlay_name_arg("a/b/c/d/e", source.path());
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Invalid overlay path format")
            );
        }

        #[test]
        fn empty_name_in_full_form() {
            let result = parse_overlay_name_arg("org/repo/", Path::new("/tmp"));
            assert!(result.is_err());
        }
    }

    /// Regression tests for source-type dispatch bugs.
    ///
    /// These tests reproduce the bugs described in issues #142, #143, #145–#148.
    /// They should fail on the `main` branch (before the `SourceResolver` fix)
    /// and pass once the fix from #149 is applied.
    mod source_resolver_bugs {
        use super::*;
        use crate::save_overlay_state;
        use crate::state::{EntryType, FileEntry, LinkType, OverlaySource, OverlayState};

        fn save_test_state(
            target: &std::path::Path,
            name: &str,
            source: OverlaySource,
            files: Vec<(&str, &str)>,
        ) {
            let mut state = OverlayState::new(name.to_string(), source);
            for (src, tgt) in files {
                state.add_file(FileEntry {
                    source: PathBuf::from(src),
                    target: PathBuf::from(tgt),
                    link_type: LinkType::Copy,
                    entry_type: EntryType::File,
                });
            }
            save_overlay_state(target, &state).unwrap();
        }

        // ==================== #142: resolve_overlay_source_path bails on GitHub ====================

        /// Issue #142: `resolve_overlay_source_path` returns an error for GitHub sources
        /// instead of resolving to the cached path. The function should succeed for
        /// all source types.
        #[test]
        fn issue_142_resolve_source_path_github_should_not_bail() {
            // Create a GitHub-sourced overlay state (no actual GitHub access needed)
            let state = OverlayState::new(
                "test-overlay".to_string(),
                OverlaySource::github(
                    "https://github.com/owner/repo".to_string(),
                    "owner".to_string(),
                    "repo".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    None,
                ),
            );

            // BUG: On main, this returns Err("Interactive edit is not supported for GitHub overlays")
            // EXPECTED: Should return Ok with a path (the cached path), not bail
            let result = resolve_overlay_source_path(&state);
            assert!(
                !result
                    .as_ref()
                    .is_err_and(|e| e.to_string().contains("not supported for GitHub")),
                "Bug #142: resolve_overlay_source_path should not unconditionally \
                 reject GitHub sources. Error: {:?}",
                result.unwrap_err()
            );
        }

        // ==================== #143: add_files_to_overlay assumes overlay repo ====================

        /// Issue #143: `add_files_to_overlay` calls `get_default_overlay_repo_config()`
        /// and uses `OverlayRepoManager` even for Local sources. It should detect
        /// the source type and handle Local sources directly (or give a clear error).
        ///
        /// The bug is that `add_files_to_overlay` never checks `state.source` —
        /// it always goes through the overlay repo code path.
        #[test]
        #[ignore = "fixed by #149"]
        fn issue_143_add_files_should_check_source_type_for_local() {
            let repo = create_test_repo();

            // Create a local overlay pointing to a real directory
            let overlay_source = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            // Create overlay state with Local source
            save_test_state(
                repo.path(),
                "local-overlay",
                OverlaySource::local(overlay_source.path().to_path_buf()),
                vec![(".envrc", ".envrc")],
            );

            // Create the target file and a new file to add
            fs::write(repo.path().join(".envrc"), "export FOO=bar").unwrap();
            fs::write(repo.path().join("new-file.txt"), "new content").unwrap();

            // BUG: On main, add_files_to_overlay loads overlay state but then
            // ignores state.source entirely. It calls get_default_overlay_repo_config()
            // and OverlayRepoManager to resolve the path, which is wrong for Local sources.
            // The overlay repo path (manager.path()/org/repo/name) won't match the
            // actual local source path, causing "does not exist in overlay repo" errors
            // or copying files to the wrong location.
            //
            // EXPECTED: Should use state.source to determine how to resolve the path.
            let result = add_files_to_overlay(
                "org/repo/local-overlay",
                repo.path(),
                &[PathBuf::from("new-file.txt")],
                true, // dry_run to avoid side effects
            );

            // BUG: On main, add_files_to_overlay goes through the overlay repo code
            // path for ALL sources (it never checks state.source). For local sources,
            // it tries to find org/repo/local-overlay in the overlay repo directory.
            // If a global overlay repo is configured, it silently uses the wrong path;
            // if not, it errors about overlay repo config.
            //
            // Either way, the function should NOT try to use the overlay repo for
            // a local source. We verify by checking:
            // 1. If it errors, the error mentions the source type (not overlay repo)
            // 2. If it "succeeds", it went to the overlay repo path (which is wrong)
            if let Err(e) = &result {
                let msg = e.to_string();
                assert!(
                    !msg.contains("does not exist in overlay repo")
                        && !msg.contains("Overlay repository not configured"),
                    "Bug #143: add_files_to_overlay should not use overlay repo \
                     for local sources. Got: {msg}"
                );
            } else {
                // "Success" actually means it found an overlay repo and tried to
                // use it — which is wrong for a local source. The local source's
                // path should be used, not the overlay repo path.
                panic!(
                    "Bug #143: add_files_to_overlay should not use overlay repo \
                     code path for local sources. It 'succeeded' by going through \
                     OverlayRepoManager instead of using the local source path."
                );
            }
        }

        /// Issue #143: `add_files_to_overlay` should give a clear error for GitHub (read-only)
        /// sources instead of trying to use `OverlayRepoManager`.
        #[test]
        fn issue_143_add_files_should_reject_github_clearly() {
            let repo = create_test_repo();

            // Manually create state with GitHub source
            save_test_state(
                repo.path(),
                "gh-overlay",
                OverlaySource::github(
                    "https://github.com/owner/repo".to_string(),
                    "owner".to_string(),
                    "repo".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    None,
                ),
                vec![(".envrc", ".envrc")],
            );

            // Create the file that would be added
            fs::write(repo.path().join(".envrc"), "existing").unwrap();
            fs::write(repo.path().join("new-file.txt"), "content").unwrap();

            // BUG: On main, this fails with "Overlay repository not configured"
            // EXPECTED: Should fail with a clear message about GitHub being read-only
            let result = add_files_to_overlay(
                "org/repo/gh-overlay",
                repo.path(),
                &[PathBuf::from("new-file.txt")],
                false,
            );

            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("read-only") || msg.contains("GitHub") || msg.contains("mutable"),
                "Bug #143: Error for GitHub source should mention read-only/GitHub, got: {msg}"
            );
        }

        // ==================== #145: update shows wrong message for OverlayRepo ====================

        /// Issue #145: `update_overlays` treats `OverlayRepo` the same as Local in
        /// the else branch, printing "local overlay (not updatable)" for both.
        ///
        /// The bug is in the code structure:
        /// ```ignore
        /// if let OverlaySource::GitHub { .. } = &state.source {
        ///     // GitHub check
        /// } else {
        ///     println!("is a local overlay (not updatable)");  // catches BOTH Local and OverlayRepo!
        /// }
        /// ```
        ///
        /// We verify the bug by checking that the code produces the same result
        /// for `OverlayRepo` as it does for Local (which is incorrect).
        #[test]
        #[ignore = "fixed by #149"]
        fn issue_145_update_code_should_handle_overlay_repo_separately() {
            let repo = create_test_repo();

            // Create two overlays: one Local, one OverlayRepo
            save_test_state(
                repo.path(),
                "local-overlay",
                OverlaySource::local(PathBuf::from("/some/path")),
                vec![("a.txt", "a.txt")],
            );
            save_test_state(
                repo.path(),
                "repo-overlay",
                OverlaySource::overlay_repo(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "repo-overlay".to_string(),
                    "abc123def456".to_string(),
                ),
                vec![("b.txt", "b.txt")],
            );

            // Both should go through update_overlays successfully (dry run)
            // The bug is that both produce the same "local overlay (not updatable)" message
            // because OverlayRepo falls into the `else` branch alongside Local.

            // We can verify the bug structurally: OverlaySource has no method to
            // distinguish "updatable" from "not updatable" except is_github().
            // Both Local and OverlayRepo return false for is_github(), so they
            // are treated identically in update_overlays.
            let local_source = OverlaySource::local(PathBuf::from("/path"));
            let repo_source = OverlaySource::overlay_repo(
                "org".to_string(),
                "repo".to_string(),
                "name".to_string(),
                "abc".to_string(),
            );

            // BUG: The only source-type check in update_overlays is `is_github()`.
            // Both Local and OverlayRepo return false, so they are indistinguishable.
            // After the fix (#149), OverlaySource has is_updatable() and
            // source_type_label() methods that distinguish them.
            //
            // Verify that OverlaySource has no method on main to distinguish
            // "updatable" from "not updatable" for non-GitHub sources.
            // After #149, this test should be updated to use is_updatable().

            // The fix adds is_updatable() -> bool:
            //   Local: false, OverlayRepo: true, GitHub: true
            // On main, we must check that no such distinction exists:
            assert!(
                !local_source.is_github() && !repo_source.is_github(),
                "Both sources are non-GitHub"
            );
            // The only way to distinguish them is is_overlay_repo(), but
            // update_overlays doesn't use it. Verify the bug by asserting
            // what SHOULD be true after the fix:
            assert_ne!(
                local_source.is_overlay_repo(),
                repo_source.is_overlay_repo(),
                "Local and OverlayRepo ARE distinguishable via is_overlay_repo(), \
                 but update_overlays doesn't use it — it only uses is_github()"
            );

            // BUG assertion: update_overlays should treat OverlayRepo differently
            // from Local. We verify that the existing API has the ability to
            // distinguish them (is_overlay_repo) but update_overlays doesn't use it.
            // The test fails when the SourceResolver trait is added because
            // source_type_label() returns different values:
            //   local_source.source_type_label() != repo_source.source_type_label()
            // But on main, those methods don't exist, so we simulate the check:
            let local_label = if local_source.is_github() {
                "GitHub"
            } else {
                "local overlay" // This is what update_overlays prints for BOTH
            };
            let repo_label = if repo_source.is_github() {
                "GitHub"
            } else {
                "local overlay" // BUG: Same label for OverlayRepo!
            };
            assert_ne!(
                local_label, repo_label,
                "Bug #145: Local and OverlayRepo get the same label in update_overlays. \
                 Both are labeled '{local_label}' because the code only checks is_github()."
            );
        }

        // ==================== #146: sync single-name doesn't check source type ====================

        /// Issue #146: `handle_sync` single-name path doesn't check `state.source`
        /// before proceeding. It should reject non-OverlayRepo sources with a
        /// clear message about syncability.
        #[test]
        fn issue_146_sync_single_name_should_check_source_type() {
            let repo = create_test_repo();

            // Set up a git remote so parse_overlay_name_arg can detect org/repo
            std::process::Command::new("git")
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/testorg/testrepo.git",
                ])
                .current_dir(repo.path())
                .output()
                .unwrap();

            // Create a local-sourced overlay
            save_test_state(
                repo.path(),
                "local-overlay",
                OverlaySource::local(PathBuf::from("/some/local/path")),
                vec![(".envrc", ".envrc")],
            );

            // BUG: On main, handle_sync for a single name with a local source
            // doesn't check state.source at all. It proceeds to load overlay repo
            // config and create an OverlayRepoManager, eventually failing with
            // an error that doesn't mention the actual problem (wrong source type).
            //
            // EXPECTED: Should fail early with a clear message like
            // "Cannot sync overlay 'local-overlay' (local source)."
            let result = handle_sync(
                repo.path(),
                Some("local-overlay".to_string()),
                false, // not --all
                false,
            );

            // The result should be an error (can't sync a local source)
            assert!(result.is_err(), "sync should fail for local sources");
            let msg = result.unwrap_err().to_string();

            // The error should explicitly mention the source type or syncability.
            // BUG: On main, the error is about overlay repo internals (e.g.,
            // "Overlay does not exist in overlay repo") not about source type.
            assert!(
                msg.contains("local") && (msg.contains("sync") || msg.contains("syncable")),
                "Bug #146: sync error for local source should mention the source type \
                 and syncability. Got: {msg}"
            );
        }

        // ==================== #147: resolve ignores source_name ====================

        /// Issue #147: `resolve_overlay_source_path` always calls
        /// `get_default_overlay_repo_config()` for `OverlayRepo` sources, ignoring
        /// the `source_name` field. With multiple sources configured, this resolves
        /// to the wrong overlay repo.
        ///
        /// We verify this by inspecting the code: the `OverlayRepo` match arm uses
        /// `..` to discard `source_name`, then calls `get_default_overlay_repo_config()`.
        #[test]
        fn issue_147_resolve_should_use_source_name() {
            // Create an OverlayRepo source with a specific source_name
            let state = OverlayState::new(
                "test-overlay".to_string(),
                OverlaySource::overlay_repo_full(
                    "myorg".to_string(),
                    "myrepo".to_string(),
                    "test-overlay".to_string(),
                    "abc123".to_string(),
                    crate::state::ResolvedVia::Direct,
                    "secondary-source".to_string(),
                ),
            );

            // Verify the source_name is stored correctly
            let source_name = match &state.source {
                OverlaySource::OverlayRepo { source_name, .. } => source_name.clone(),
                _ => panic!("Test setup: expected OverlayRepo source"),
            };
            assert_eq!(
                source_name.as_deref(),
                Some("secondary-source"),
                "Test setup: source_name should be set"
            );

            // BUG: On main, resolve_overlay_source_path destructures with `..`
            // which discards source_name, then calls get_default_overlay_repo_config().
            // The result is that it always resolves to the default source even when
            // source_name specifies a different source.
            //
            // We verify this by calling resolve_overlay_source_path — it will call
            // get_default_overlay_repo_config() (ignoring source_name) and fail
            // with "Overlay repository not configured" rather than a source-name-aware
            // error like "Source 'secondary-source' not found in configuration".
            let result = resolve_overlay_source_path(&state);
            assert!(
                result.is_err(),
                "Expected error (no config available in test env)"
            );
            let msg = result.unwrap_err().to_string();

            // BUG: The error says "Overlay repository not configured" because it
            // called get_default_overlay_repo_config() instead of looking up
            // "secondary-source" specifically.
            // AFTER FIX: The error should mention the source name "secondary-source".
            assert!(
                msg.contains("secondary-source"),
                "Bug #147: resolve_overlay_source_path ignores source_name \
                 'secondary-source' and calls get_default_overlay_repo_config() \
                 instead. Error should mention the source name. Got: {msg}"
            );
        }

        // ==================== #148: no early mutability check ====================

        /// Issue #148: `add_files_to_overlay` doesn't check source mutability
        /// before making filesystem changes. For GitHub (read-only) sources,
        /// the function proceeds past validation, attempts to copy files to the
        /// overlay repo, and fails late — potentially leaving partial state.
        ///
        /// The fix is to check mutability (or equivalent) before any mutations.
        /// On main, there is no such upfront check — the function never inspects
        /// `state.source` at all.
        #[test]
        fn issue_148_add_should_check_mutability_before_filesystem_changes() {
            let repo = create_test_repo();

            // Create overlay state with GitHub source (immutable)
            save_test_state(
                repo.path(),
                "gh-overlay",
                OverlaySource::github(
                    "https://github.com/owner/repo".to_string(),
                    "owner".to_string(),
                    "repo".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    None,
                ),
                vec![(".envrc", ".envrc")],
            );

            // Create files
            fs::write(repo.path().join(".envrc"), "existing").unwrap();
            fs::write(repo.path().join("extra.txt"), "content").unwrap();

            // Call add_files_to_overlay with org/repo/name format
            let result = add_files_to_overlay(
                "org/repo/gh-overlay",
                repo.path(),
                &[PathBuf::from("extra.txt")],
                false,
            );

            // It should fail — that's expected for a read-only source.
            assert!(result.is_err());

            // BUG: On main, the error comes from deep in the overlay repo code
            // (e.g., "does not exist in overlay repo") rather than from an early
            // mutability check. The function never inspects state.source to see
            // that it's a GitHub (read-only) source.
            //
            // EXPECTED: Error should mention "read-only", "GitHub", or "mutable"
            // indicating that the source type was checked.
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("read-only")
                    || msg.contains("GitHub")
                    || msg.contains("mutable")
                    || msg.contains("not supported"),
                "Bug #148: Error should come from an early mutability check mentioning \
                 the source type, not from late overlay repo operations. Got: {msg}"
            );
        }
    }
}
