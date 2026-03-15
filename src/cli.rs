//! CLI implementation for repoverlay.
//!
//! Defines the command structure using clap and dispatches to `lib.rs` functions.
//! The `run()` function is the internal entry point called from `lib::run()`.

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::{
    CacheManager, ConflictStrategy, OVERLAYS_DIR, OverlayName, ResolvedSource, STATE_DIR,
    apply_multiple_overlays, apply_overlay, canonicalize_path, config, get_cached_repo_commit,
    list_applied_overlays, list_overlays_from_cached_repo, parse_github_owner_repo, remove_overlay,
    remove_single_overlay, restore_overlays, selection::is_interactive, show_status,
    show_status_json, status_has_overlays, switch_overlay, update_overlays, validate_git_repo,
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
/// Fetches an update message from the website when an update is available.
fn check_for_updates() {
    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");

    let checker = tiny_update_check::UpdateChecker::new(name, version)
        .message_url("https://repoverlay.tylerbutler.com/update-message.txt");

    if let Ok(Some(update)) = checker.check_detailed() {
        eprintln!();
        eprintln!(
            "{} A new version of {} is available: {} → {}",
            "Update available:".yellow().bold(),
            name,
            update.current,
            update.latest.green().bold()
        );
        if let Some(msg) = &update.message {
            eprintln!();
            eprintln!("{msg}");
        } else {
            eprintln!(
                "                  {}",
                "https://github.com/tylerbutler/repoverlay/releases".cyan()
            );
        }
    }
}

/// Overlay config files into git repositories without committing them
///
/// Get started: run `repoverlay browse` to interactively discover and apply overlays.
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
    ///
    /// For interactive use, consider `repoverlay browse` instead.
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

        /// Skip updating cached/overlay repositories before applying
        #[arg(long)]
        no_update: bool,

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

        /// Include files/directories or glob patterns (can be specified multiple times)
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

        /// Skip updating cached/overlay repositories before switching
        #[arg(long)]
        no_update: bool,

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

        /// Show what would be switched without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Manage the overlay cache
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },

    /// Browse and apply overlays interactively (recommended)
    ///
    /// Lists available overlays from configured sources and lets you select which to
    /// apply. This is the easiest way to discover and apply overlays. To add sources,
    /// run `repoverlay source add <path-or-url>`.
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

        /// Skip updating overlay repo before listing
        #[arg(long)]
        no_update: bool,

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

    /// Edit an existing applied overlay
    ///
    /// With no subcommand, launches interactive file selection. If no overlay
    /// name is given, prompts to select from applied overlays.
    ///
    /// Examples:
    ///   repoverlay edit                                     # Pick overlay, then edit
    ///   repoverlay edit my-overlay                          # Interactive file selection
    ///   repoverlay edit add my-overlay newfile.txt          # Add files
    ///   repoverlay edit add my-overlay file1.txt file2.txt  # Add multiple files
    ///   repoverlay edit remove my-overlay oldfile.txt       # Remove files
    #[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
    Edit {
        #[command(subcommand)]
        command: Option<EditCommand>,

        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit values
        name: Option<String>,

        /// Files to add to the overlay
        #[arg(short, long, value_name = "FILE", num_args = 1.., hide = true)]
        add: Vec<PathBuf>,

        /// Files to remove from the overlay
        #[arg(short, long, value_name = "FILE", num_args = 1.., hide = true)]
        remove: Vec<PathBuf>,

        /// Re-run interactive file selection with current files pre-selected
        #[arg(short, long, conflicts_with_all = ["add", "remove"], hide = true)]
        interactive: bool,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would change without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Deprecated: use `create --output` instead
    #[command(hide = true)]
    CreateLocal {
        /// Include specific files or directories
        #[arg(short, long)]
        include: Vec<PathBuf>,

        /// Source repository to extract files from (defaults to current directory)
        #[arg(short, long)]
        source: Option<PathBuf>,

        /// Output directory for local overlay creation
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Overlay name
        name: Option<String>,

        /// Show what would be created without creating files
        #[arg(long)]
        dry_run: bool,

        /// Skip interactive prompts
        #[arg(short = 'y', long)]
        yes: bool,

        /// Force overwrite if overlay already exists
        #[arg(short, long)]
        force: bool,
    },

    /// Deprecated: use `browse` instead
    #[command(hide = true, name = "list")]
    List {
        /// Overlay source
        #[arg(value_name = "SOURCE")]
        source: Option<String>,

        /// Filter by target repository
        #[arg(short = 'f', long)]
        filter: Option<String>,

        /// Skip updating overlay repo before listing
        #[arg(long)]
        no_update: bool,

        /// Target repository directory
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Disable interactive selection
        #[arg(long)]
        no_interactive: bool,

        /// Show what would be applied without making changes
        #[arg(long)]
        dry_run: bool,

        /// Show all overlays
        #[arg(long)]
        show_all: bool,
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
        /// Source: Git URL, GitHub shorthand (owner/repo), GitHub username, or local path (./path)
        source: config::SourceUrlInput,

        /// Name for this source (defaults to repo/directory name)
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

#[derive(Subcommand)]
enum EditCommand {
    /// Add files to an applied overlay
    ///
    /// Examples:
    ///   repoverlay edit add my-overlay newfile.txt
    ///   repoverlay edit add my-overlay file1.txt file2.txt
    ///   repoverlay edit add org/repo/my-overlay newfile.txt
    Add {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit values
        name: String,

        /// Files to add to the overlay
        files: Vec<PathBuf>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would change without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove files from an applied overlay
    ///
    /// Examples:
    ///   repoverlay edit remove my-overlay oldfile.txt
    ///   repoverlay edit remove my-overlay file1.txt file2.txt
    ///   repoverlay edit remove org/repo/my-overlay oldfile.txt
    Remove {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit values
        name: String,

        /// Files to remove from the overlay
        files: Vec<PathBuf>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would change without making changes
        #[arg(long)]
        dry_run: bool,
    },
}

pub(crate) fn run() -> Result<()> {
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
            no_update,
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
                !no_update,
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
        Commands::Switch {
            source,
            target,
            copy,
            name,
            r#ref,
            no_update,
            force,
            skip_conflicts,
            interactive,
            merge,
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
            switch_overlay(
                &source,
                &target,
                copy,
                name,
                r#ref.as_deref(),
                !no_update, // default: sync before switching
                conflict_strategy,
                merge,
                dry_run,
            )?;
        }
        Commands::Cache { command } => {
            handle_cache_command(command)?;
        }
        Commands::Browse {
            source,
            filter,
            no_update,
            target,
            no_interactive,
            dry_run,
            show_all,
        } => {
            browse_overlays(
                source.as_deref(),
                filter.as_deref(),
                !no_update,
                target,
                no_interactive,
                dry_run,
                show_all,
            )?;
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
            command,
            name,
            add,
            remove,
            interactive,
            target,
            dry_run,
        } => match command {
            Some(EditCommand::Add {
                name,
                files,
                target,
                dry_run,
            }) => {
                let target = target.unwrap_or_else(|| PathBuf::from("."));
                add_files_to_overlay(&name, &target, &files, dry_run)?;
            }
            Some(EditCommand::Remove {
                name,
                files,
                target,
                dry_run,
            }) => {
                let target = target.unwrap_or_else(|| PathBuf::from("."));
                remove_files_from_overlay(&name, &target, &files, dry_run)?;
            }
            None => {
                let target = target.unwrap_or_else(|| PathBuf::from("."));
                if !add.is_empty() || !remove.is_empty() {
                    eprintln!(
                        "{}: --add/--remove flags are deprecated, use `edit add` or `edit remove` subcommands instead",
                        "Warning".yellow().bold()
                    );
                }
                // When no name is given, interactively select an overlay
                // and go straight to interactive file selection.
                let (name, interactive) = match name {
                    Some(n) => (n, interactive),
                    None => (select_overlay_interactive(&target)?, true),
                };
                edit_overlay(&name, &target, &add, &remove, interactive, dry_run)?;
            }
        },
        Commands::CreateLocal {
            include,
            source,
            output,
            name,
            dry_run,
            yes,
            force,
        } => {
            eprintln!("Warning: `create-local` is deprecated, use `create --output` instead");
            let source = source.unwrap_or_else(|| PathBuf::from("."));
            create_overlay_command(&source, name, output, &include, dry_run, yes, force)?;
        }
        Commands::List {
            source,
            filter,
            no_update,
            target,
            no_interactive,
            dry_run,
            show_all,
        } => {
            eprintln!("Warning: `list` is deprecated, use `browse` instead");
            browse_overlays(
                source.as_deref(),
                filter.as_deref(),
                !no_update,
                target,
                no_interactive,
                dry_run,
                show_all,
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
        SourceCommand::Add { source, name } => {
            if source.is_local() {
                // Local path source — save to per-repo config
                let raw_path = source.local_path().to_path_buf();
                let repo_root = find_repo_root()?;

                // Canonicalize and validate path exists
                let canonical = raw_path
                    .canonicalize()
                    .with_context(|| format!("Path does not exist: {}", raw_path.display()))?;

                // Validate within repo
                let repo_root_canonical = repo_root.canonicalize()?;
                if !canonical.starts_with(&repo_root_canonical) {
                    bail!(
                        "Path must be within the repository: {}",
                        canonical.display()
                    );
                }

                // Convert to repo-relative
                let relative_path = canonical
                    .strip_prefix(&repo_root_canonical)
                    .context("Failed to compute repo-relative path")?;

                // Extract name
                let source_name = name.unwrap_or_else(|| {
                    relative_path
                        .file_name()
                        .map_or_else(|| "local".to_string(), |n| n.to_string_lossy().to_string())
                });

                if source_name.is_empty() {
                    bail!(
                        "Could not extract source name from path. Please provide a name with --name"
                    );
                }

                // Check name conflicts in both global and repo configs
                let global_config = config::load_global_config()?;
                let repo_config = config::load_repo_config(&repo_root)?;

                if global_config.sources.iter().any(|s| s.name == source_name) {
                    bail!("Source '{source_name}' already exists");
                }
                if let Some(ref rc) = repo_config
                    && rc.sources.iter().any(|s| s.name == source_name)
                {
                    bail!("Source '{source_name}' already exists");
                }

                let new_source = config::Source {
                    name: source_name.clone(),
                    url: None,
                    path: Some(PathBuf::from(relative_path)),
                };

                let mut updated_repo_config = repo_config.unwrap_or_default();
                updated_repo_config.sources.push(new_source);
                config::save_repo_config(&repo_root, &updated_repo_config)?;

                println!(
                    "{} local source '{}' at position {}",
                    "Added".green().bold(),
                    source_name,
                    updated_repo_config.sources.len()
                );
                println!("       Path: {}", relative_path.display());
                println!(
                    "       Config: {}",
                    config::repo_config_path(&repo_root).display()
                );
            } else {
                // Git URL source — save to global config
                let validated_url = source.to_url();
                let source_name = name.unwrap_or_else(|| {
                    validated_url
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("source")
                        .trim_end_matches(".git")
                        .to_string()
                });

                if source_name.is_empty() {
                    bail!(
                        "Could not extract source name from URL. Please provide a name with --name"
                    );
                }

                // Check if name already exists
                if config.sources.iter().any(|s| s.name == source_name) {
                    bail!("Source '{source_name}' already exists");
                }

                let new_source = config::Source {
                    name: source_name.clone(),
                    url: Some(validated_url.clone()),
                    path: None,
                };
                config.sources.push(new_source);
                config::save_config(&config)?;

                println!(
                    "{} source '{}' at position {}",
                    "Added".green().bold(),
                    source_name,
                    config.sources.len()
                );
                println!("       {validated_url}");
            }
        }
        SourceCommand::List => {
            let repo_root = find_repo_root().ok();

            let global_config = config::load_config(None)?;
            let repo_config = repo_root
                .as_ref()
                .map(|r| config::load_repo_config(r))
                .transpose()?
                .flatten()
                .unwrap_or_default();

            let has_sources = !global_config.sources.is_empty() || !repo_config.sources.is_empty();

            if !has_sources {
                println!("No overlay sources configured.");
                println!("Add one with: repoverlay source add <url-or-path>");
                return Ok(());
            }

            // Show repo sources first (higher priority)
            if !repo_config.sources.is_empty() {
                println!(
                    "{} ({})",
                    "Repository sources".bold(),
                    repo_root
                        .as_ref()
                        .map(|r| config::repo_config_path(r).display().to_string())
                        .unwrap_or_default()
                );
                for (i, source) in repo_config.sources.iter().enumerate() {
                    print!("  {}. {}", i + 1, source.name.bold());
                    if let Some(path) = &source.path {
                        println!(" (path: {})", path.display());
                    } else if let Some(url) = &source.url {
                        println!(" (url: {url})");
                    } else {
                        println!();
                    }
                }
            }

            if !global_config.sources.is_empty() {
                if !repo_config.sources.is_empty() {
                    println!();
                }
                println!(
                    "{} (~/.config/repoverlay/config.ccl)",
                    "Global sources".bold()
                );
                for (i, source) in global_config.sources.iter().enumerate() {
                    let position = repo_config.sources.len() + i + 1;
                    print!("  {}. {}", position, source.name.bold());
                    if let Some(url) = &source.url {
                        println!(" (url: {url})");
                    } else if let Some(path) = &source.path {
                        println!(" (path: {})", path.display());
                    } else {
                        println!();
                    }
                }
            }

            println!("\nSources are checked in priority order (lowest number = highest priority).");
        }
        SourceCommand::Remove { name } => {
            let repo_root = find_repo_root().ok();

            // Check repo config first
            if let Some(ref root) = repo_root
                && let Some(mut repo_config) = config::load_repo_config(root)?
                && let Some(pos) = repo_config.sources.iter().position(|s| s.name == name)
            {
                repo_config.sources.remove(pos);
                config::save_repo_config(root, &repo_config)?;
                println!(
                    "{} source '{}' from repository config",
                    "Removed".red().bold(),
                    name
                );
                return Ok(());
            }

            // Fall back to global config
            if let Some(pos) = config.sources.iter().position(|s| s.name == name) {
                config.sources.remove(pos);
                config::save_config(&config)?;
                println!(
                    "{} source '{}' from global config",
                    "Removed".red().bold(),
                    name
                );
            } else {
                bail!("Source '{name}' not found in any config");
            }
        }
    }

    Ok(())
}

/// Find the root of the current git repository.
fn find_repo_root() -> Result<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("Failed to find git repository root")?;
    if !output.status.success() {
        bail!("Not inside a git repository");
    }
    let root = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(PathBuf::from(root))
}

/// Handle remove command with interactive selection support.
fn handle_remove(
    target: &std::path::Path,
    name: Option<String>,
    remove_all: bool,
    dry_run: bool,
    interactive: bool,
) -> Result<()> {
    use crate::selection::{FlatSelectionConfig, SelectableItem, ToSelectableItem, select_flat};

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
        .map(|name| name.to_selectable_item(&target))
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
            if overlay.flat {
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
    use crate::sources::SourceManager;
    use crate::state::{OverlaySource, ResolvedVia};

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

    if config.sources.is_empty() {
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

    // Use SourceManager for multi-source browsing (handles both git and local)
    let manager = SourceManager::new(config.sources, Some(&target_canonical))?;
    manager.ensure_all_cloned()?;

    if update {
        println!("{} overlay sources...", "Updating".blue().bold());
        manager.pull_all()?;
    }

    // Get all overlays with their source info
    let all_with_sources = manager.list_all_overlays()?;

    // Build a lookup map: overlay key -> Source
    let source_map: std::collections::HashMap<String, config::Source> = all_with_sources
        .iter()
        .map(|(src, overlay)| (overlay.to_string(), src.clone()))
        .collect();

    // Extract just the overlays for browse_and_apply
    let overlays: Vec<_> = if let Some(filter) = target_filter {
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

    let build_source_info = |o: &crate::overlay_repo::AvailableOverlay| {
        let overlay_key = o.to_string();
        let source = source_map.get(&overlay_key).ok_or_else(|| {
            anyhow::anyhow!("Could not determine source for overlay: {overlay_key}")
        })?;
        let base_path = manager
            .get_source_base_path(&source.name)
            .ok_or_else(|| anyhow::anyhow!("Source base path not found: {}", source.name))?;
        let overlay_path = base_path.join(&o.org).join(&o.repo).join(&o.name);
        let commit = manager.get_source_commit(&source.name)?;

        Ok(ResolvedSource {
            path: overlay_path,
            source_info: OverlaySource::overlay_repo_full(
                o.org.clone(),
                o.repo.clone(),
                o.name.clone(),
                commit,
                ResolvedVia::Direct,
                source.name.clone(),
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

    // Handle local paths directly
    if let SourceReference::LocalPath { path, .. } = &reference {
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
        SourceReference::ThreePart { .. } => {
            bail!(
                "Invalid source for browse: '{source_str}'\n\n\
                 Use a GitHub username, owner/repo, GitHub URL, or local path."
            );
        }
        SourceReference::LocalPath { .. } => unreachable!(),
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
    use crate::sources::list_overlays_in_dir;
    use crate::state::OverlaySource;

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
    let build_source_info = move |o: &crate::overlay_repo::AvailableOverlay| {
        let overlay_path = local_base.join(o.relative_path());
        if !overlay_path.exists() {
            bail!("Overlay directory not found: {}", overlay_path.display());
        }
        Ok(ResolvedSource {
            path: overlay_path,
            source_info: OverlaySource::local(local_base.clone()),
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
    use crate::overlay_repo::BrowseOverlayItem;
    use crate::selection::{FlatSelectionConfig, SelectableItem, ToSelectableItem, select_flat};

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
///
/// After creating the overlay, it is automatically applied to the source
/// repository (symlinks replace originals, state saved, git exclude updated).
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
        // When a name is provided, create a subdirectory: output/<name>/
        let (output_path, overlay_name) = if let Some(ref name) = name_arg {
            (local_path.join(name), Some(name.clone()))
        } else {
            (local_path, None)
        };

        // Use existing create_overlay function for local mode
        crate::create_overlay(
            source,
            Some(output_path.clone()),
            include,
            overlay_name,
            dry_run,
            yes,
        )?;

        // Auto-apply the newly created overlay back to the source repo
        if !dry_run {
            let overlay_source = output_path.to_string_lossy().to_string();
            crate::apply_overlay(
                &overlay_source,
                source,
                false,
                None,
                None,
                false,
                crate::ConflictStrategy::Force,
                false,
                None,
                false,
            )?;
        }

        return Ok(());
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

    // Create manager, ensure cloned, and pull latest
    let manager = OverlayRepoManager::new(overlay_config)?;
    manager.ensure_cloned()?;
    manager.pull()?;

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
        crate::create_overlay(
            source,
            Some(output_path.clone()),
            include,
            Some(overlay_name.clone()),
            dry_run,
            yes,
        )?;

        // Auto-commit after creating
        auto_commit_overlay(&manager, &org, &repo, &overlay_name, true)?;

        // Auto-apply the overlay back to the source repo
        let overlay_source = output_path.to_string_lossy().to_string();
        crate::apply_overlay(
            &overlay_source,
            source,
            false,
            Some(overlay_name),
            None,
            false,
            crate::ConflictStrategy::Force,
            false,
            None,
            false,
        )?;

        return Ok(());
    }

    // Expand globs and validate include paths
    let expanded = crate::expand_include_globs(source, include)?;

    // If force and exists, remove existing first
    if output_path.exists() && force {
        fs::remove_dir_all(&output_path)?;
    }

    // Copy files and create overlay
    let copied_files = crate::copy_files_to_overlay(source, &output_path, &expanded)?;

    // Generate config
    fs::write(
        output_path.join("repoverlay.ccl"),
        crate::generate_overlay_config(&overlay_name),
    )?;

    crate::print_overlay_created(&output_path, &copied_files);

    // Auto-commit
    auto_commit_overlay(&manager, &org, &repo, &overlay_name, true)?;

    // Auto-apply the overlay back to the source repo
    let overlay_source = output_path.to_string_lossy().to_string();
    crate::apply_overlay(
        &overlay_source,
        source,
        false,
        Some(overlay_name),
        None,
        false,
        crate::ConflictStrategy::Force,
        false,
        None,
        false,
    )?;

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
    let fetch_result = crate::git::run_git_with_spinner(
        &["fetch", "origin"],
        Some(manager.path()),
        "Fetching from remote...",
        false,
    );

    match fetch_result {
        Ok((status, _)) if status.success() => {
            // Try to pull/rebase to incorporate remote changes
            let pull_result = crate::git::run_git_with_spinner(
                &["pull", "--rebase", "--autostash"],
                Some(manager.path()),
                "Pulling latest changes...",
                false,
            );

            match pull_result {
                Ok((status, _)) if !status.success() => {
                    eprintln!(
                        "{} Could not pull latest changes, continuing...",
                        "Warning:".yellow(),
                    );
                }
                Err(e) => {
                    eprintln!("{} Could not pull latest changes: {e}", "Warning:".yellow(),);
                }
                _ => {}
            }
        }
        _ => {
            // Fetch failed, but continue - might be offline
            eprintln!(
                "{} Could not fetch from remote (offline?), continuing...",
                "Warning:".yellow()
            );
        }
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
    let push_result = crate::git::run_git_with_spinner(
        &["push"],
        Some(manager.path()),
        "Pushing to remote...",
        false,
    );

    match push_result {
        Ok((status, _)) if status.success() => {
            let check = "✓".green().bold();
            let action_word = if is_new { "created" } else { "updated" };
            println!("\n{check} Overlay {action_word}: {org}/{repo}/{name}");
        }
        Ok(_) | Err(_) => {
            let warn = "Warning:".yellow();
            eprintln!("\n{warn} Committed locally but failed to push.");
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

        // Lazily initialized — only created when we encounter a syncable overlay.
        // This avoids failing when no overlay repo is configured but all applied
        // overlays are local/GitHub sources (#205).
        let mut manager: Option<OverlayRepoManager> = None;

        let mut synced = 0u32;
        let mut skipped = 0u32;

        for overlay_name in &applied_overlays {
            let mut state = load_overlay_state(&target, overlay_name.as_str())?;
            crate::try_upgrade_github_source(&target, &mut state)?;

            // Use SourceResolver to check syncability (#146, #149)
            if !state.source.is_syncable() {
                let label = state.source.source_type_label();
                println!(
                    "{} Skipping '{}' ({label} source, not syncable)",
                    "Warning:".yellow(),
                    overlay_name
                );
                skipped += 1;
                continue;
            }

            // Initialize the manager on first syncable overlay
            let mgr = if let Some(m) = &manager {
                m
            } else {
                let config = load_config(None)?;
                let overlay_config = config.get_default_overlay_repo_config()?;
                let m = OverlayRepoManager::new(overlay_config)?;
                m.ensure_cloned()?;
                m.pull()?;
                manager = Some(m);
                manager.as_ref().unwrap()
            };

            // OverlayRepo source — sync directly
            match &state.source {
                OverlaySource::OverlayRepo {
                    org, repo, name, ..
                } => {
                    sync_single_overlay(&target, org, name, repo, &state, mgr, dry_run)?;
                    if !dry_run {
                        auto_commit_overlay(mgr, org, repo, name, false)?;
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
        let (detected_org, detected_repo, overlay_name) =
            parse_overlay_name_arg(&name_arg, &target)?;

        // Verify the overlay is currently applied
        let normalized_name = normalize_overlay_name(&overlay_name)?;
        let applied_overlays = list_applied_overlays(&target)?;

        if !applied_overlays
            .iter()
            .any(|n| n == normalized_name.as_str())
        {
            bail!(
                "Overlay '{overlay_name}' is not currently applied.\n\n\
                 To apply it first: repoverlay apply {detected_org}/{detected_repo}/{overlay_name}"
            );
        }

        // Load overlay state to get file mappings
        let mut state = load_overlay_state(&target, &normalized_name)?;
        crate::try_upgrade_github_source(&target, &mut state)?;

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

        // Use org/repo from saved state rather than git remote detection.
        // When an overlay was applied via upstream fallback (e.g., fork
        // alexvy86/FluidFramework resolved to upstream microsoft/FluidFramework),
        // the state records the correct upstream org/repo. Using the git-remote-
        // detected org/repo would point to the fork path which doesn't exist
        // in the overlay repo.
        let (org, repo) = match &state.source {
            OverlaySource::OverlayRepo { org, repo, .. } => (org.clone(), repo.clone()),
            _ => (detected_org, detected_repo),
        };

        // Load overlay repo config (respects source_name for multi-source configs, #147)
        let config = load_config(None)?;
        let source_name = match &state.source {
            OverlaySource::OverlayRepo { source_name, .. } => source_name.as_deref(),
            _ => None,
        };
        let overlay_config = config.get_overlay_repo_config_by_name(source_name)?;

        // Create manager, ensure cloned, and pull latest
        let manager = OverlayRepoManager::new(overlay_config)?;
        manager.ensure_cloned()?;
        manager.pull()?;

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

/// Interactively select an applied overlay by name.
///
/// Lists all applied overlays and lets the user pick one. Bails in non-TTY
/// environments since interactive selection requires a terminal.
fn select_overlay_interactive(target: &std::path::Path) -> Result<String> {
    use crate::selection::{FlatSelectionConfig, SelectableItem, ToSelectableItem, select_flat};

    let target = canonicalize_path(target, "Target directory")?;
    let applied = list_applied_overlays(&target)?;

    if applied.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    if applied.len() == 1 {
        return Ok(applied[0].to_string());
    }

    if !is_interactive() {
        bail!(
            "Multiple overlays applied — specify which one to edit.\n\n\
             Usage:\n  \
             repoverlay edit <name>\n  \
             repoverlay edit add <name> <files>...\n  \
             repoverlay edit remove <name> <files>..."
        );
    }

    let items: Vec<SelectableItem> = applied
        .iter()
        .map(|name| name.to_selectable_item(&target))
        .collect();

    let result = select_flat(
        &items,
        &FlatSelectionConfig {
            prompt: "Select overlay to edit:".into(),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlay selected");
    }

    Ok(result.selected_ids[0].clone())
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

    let mut state = load_overlay_state(&target, &normalized_name)?;
    crate::try_upgrade_github_source(&target, &mut state)?;

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
    let mut overlay_source_files: HashSet<PathBuf> = HashSet::new();
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
            overlay_source_files.insert(relative.clone());

            detected_files.push(DetectedFile {
                path: relative,
                category: FileCategory::Untracked, // Generic category for overlay files
                preselected: is_currently_applied,
                depth: 0,
                parent_dir: None,
            });
        }
    }

    // Discover files in the target repo that could be added to the overlay (#190).
    // This includes AI configs, gitignored files, and untracked files that aren't
    // already in the overlay source.
    let target_repo_candidates = crate::detection::discover_files(&target);
    for candidate in target_repo_candidates {
        // Skip files already in the overlay source
        if overlay_source_files.contains(&candidate.path) {
            continue;
        }
        // Skip files that are symlinks (already managed by an overlay)
        let full_path = target.join(&candidate.path);
        if full_path.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
            continue;
        }
        detected_files.push(DetectedFile {
            preselected: false, // Target repo files start unselected
            ..candidate
        });
    }

    if detected_files.is_empty() {
        bail!(
            "No files found in overlay source or target repository: {}",
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
        // Files from the overlay source need to be copied to the target first,
        // since add_files_to_overlay expects them to exist in the target.
        // Files from the target repo already exist there.
        for file in &to_add {
            if overlay_source_files.contains(file) {
                let source_file = source_path.join(file);
                let target_file = target.join(file);
                if let Some(parent) = target_file.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&source_file, &target_file).with_context(|| {
                    format!("Failed to copy {} from overlay source", file.display())
                })?;
            }
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

        // Remove from state and add to exclusions
        state.remove_file(file);
        state.add_exclusion(file.clone());
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
/// The files are linked to the overlay source and the overlay state is updated.
///
/// Source-type-aware behavior (#148):
/// - **`OverlayRepo`**: copies files to overlay repo, creates symlinks, auto-commits
/// - **Local**: copies files to local overlay directory, creates symlinks
/// - **GitHub**: rejected (read-only source)
///
/// File operations are performed atomically: if any step fails, all changes
/// are rolled back to prevent leaving the target in a half-modified state.
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

    /// Tracks completed operations for rollback on failure.
    enum RollbackEntry {
        File {
            target: PathBuf,
            overlay: PathBuf,
            original_content: Vec<u8>,
        },
        Directory {
            target: PathBuf,
            overlay: PathBuf,
        },
    }

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
             Usage: repoverlay edit <overlay-name> --add <file> [--add <file>...]"
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
    crate::try_upgrade_github_source(&target, &mut state)?;

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

    // Validate all files exist before any mutations
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

    // Validate all file copy destinations are writable before mutating anything.
    // This catches permission errors early to avoid partial mutations.
    for file in files {
        let overlay_file = overlay_repo_path.join(file);
        if let Some(parent) = overlay_file.parent() {
            // Verify we can create the parent directory
            if !parent.exists() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Cannot create directory in overlay source: {}",
                        parent.display()
                    )
                })?;
            }
        }
    }

    let mut completed: Vec<RollbackEntry> = Vec::new();
    let mut added_count = 0;

    let result: Result<()> = (|| {
        for file in files {
            let target_file = target.join(file);
            let overlay_file = overlay_repo_path.join(file);
            let is_dir = target_file.is_dir();

            // Ensure overlay parent directory exists
            if is_dir {
                if let Some(parent) = overlay_file.parent() {
                    fs::create_dir_all(parent)?;
                }
            } else if let Some(parent) = overlay_file.parent() {
                fs::create_dir_all(parent)?;
            }

            if is_dir {
                // Copy directory tree to overlay source
                fs::create_dir_all(&overlay_file)?;
                crate::overlay_repo::copy_dir_recursive(&target_file, &overlay_file).with_context(
                    || {
                        format!(
                            "Failed to copy directory {} to overlay source",
                            target_file.display()
                        )
                    },
                )?;

                // Remove original directory
                fs::remove_dir_all(&target_file).with_context(|| {
                    format!(
                        "Failed to remove directory {} for linking",
                        target_file.display()
                    )
                })?;

                // Create symlink/copy from overlay source to target
                match link_type {
                    LinkType::Symlink => {
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(&overlay_file, &target_file).with_context(
                            || {
                                format!(
                                    "Failed to create directory symlink: {}",
                                    target_file.display()
                                )
                            },
                        )?;
                        #[cfg(windows)]
                        std::os::windows::fs::symlink_dir(&overlay_file, &target_file)
                            .with_context(|| {
                                format!(
                                    "Failed to create directory symlink: {}",
                                    target_file.display()
                                )
                            })?;
                    }
                    LinkType::Copy | LinkType::Merged => {
                        fs::create_dir_all(&target_file).with_context(|| {
                            format!("Failed to create directory: {}", target_file.display())
                        })?;
                        crate::overlay_repo::copy_dir_recursive(&overlay_file, &target_file)
                            .with_context(|| {
                                format!("Failed to copy directory: {}", target_file.display())
                            })?;
                    }
                }

                completed.push(RollbackEntry::Directory {
                    target: target_file.clone(),
                    overlay: overlay_file,
                });

                state.add_file(FileEntry {
                    source: file.clone(),
                    target: file.clone(),
                    link_type,
                    entry_type: EntryType::Directory,
                });

                println!("  {} {}/", "+".green(), file.display());
            } else {
                // Read original content before any mutations (for rollback)
                let original_content = fs::read(&target_file).with_context(|| {
                    format!("Failed to read {} for backup", target_file.display())
                })?;

                // Copy file to overlay source directory
                fs::copy(&target_file, &overlay_file).with_context(|| {
                    format!("Failed to copy {} to overlay source", target_file.display())
                })?;

                // Remove original file (we'll replace it with symlink)
                fs::remove_file(&target_file).with_context(|| {
                    format!("Failed to remove {} for linking", target_file.display())
                })?;

                // Create symlink/copy from overlay source to target
                match link_type {
                    LinkType::Symlink => {
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(&overlay_file, &target_file).with_context(
                            || format!("Failed to create symlink: {}", target_file.display()),
                        )?;
                        #[cfg(windows)]
                        std::os::windows::fs::symlink_file(&overlay_file, &target_file)
                            .with_context(|| {
                                format!("Failed to create symlink: {}", target_file.display())
                            })?;
                    }
                    LinkType::Copy | LinkType::Merged => {
                        fs::copy(&overlay_file, &target_file).with_context(|| {
                            format!("Failed to copy file: {}", target_file.display())
                        })?;
                    }
                }

                completed.push(RollbackEntry::File {
                    target: target_file.clone(),
                    overlay: overlay_file,
                    original_content,
                });

                state.add_file(FileEntry {
                    source: file.clone(),
                    target: file.clone(),
                    link_type,
                    entry_type: EntryType::File,
                });

                println!("  {} {}", "+".green(), file.display());
            }

            added_count += 1;
        }

        Ok(())
    })();

    // On failure, roll back all completed operations
    if let Err(ref e) = result {
        eprintln!(
            "\n{} Rolling back {} file(s) due to error: {}",
            "Error:".red().bold(),
            completed.len(),
            e
        );
        for entry in completed.iter().rev() {
            match entry {
                RollbackEntry::File {
                    target: target_file,
                    overlay: overlay_file,
                    original_content,
                } => {
                    // Remove the symlink/copy we created
                    if target_file.exists() || target_file.is_symlink() {
                        let _ = fs::remove_file(target_file);
                    }
                    // Restore original file content
                    let _ = fs::write(target_file, original_content);
                    // Remove the copy in overlay source
                    let _ = fs::remove_file(overlay_file);
                }
                RollbackEntry::Directory {
                    target: target_file,
                    overlay: overlay_file,
                } => {
                    // Remove the symlink/copy we created
                    if target_file.is_symlink() {
                        let _ = fs::remove_file(target_file);
                    } else if target_file.exists() {
                        let _ = fs::remove_dir_all(target_file);
                    }
                    // Restore directory from overlay copy
                    let _ = fs::create_dir_all(target_file);
                    let _ = crate::overlay_repo::copy_dir_recursive(overlay_file, target_file);
                    // Remove the copy in overlay source
                    let _ = fs::remove_dir_all(overlay_file);
                }
            }
        }
        return result;
    }

    // Rebuild full exclude list from state (which now includes both old and new files)
    // Also clear any exclusions for files that were just added back
    for file in files {
        state.remove_exclusion(file);
    }
    let all_exclude_entries: Vec<String> = state
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
    update_git_exclude(&target, &normalized_name, &all_exclude_entries, true)?;

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

        #[test]
        fn create_output_with_name_creates_subdirectory() {
            let source = create_test_repo();
            let output = TempDir::new().unwrap();

            fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

            // Simulate what create_overlay_command does: output/<name>/
            let overlay_name = "my-overlay";
            let output_path = output.path().join(overlay_name);

            let result = create_overlay(
                source.path(),
                Some(output_path),
                &[PathBuf::from(".envrc")],
                Some(overlay_name.to_string()),
                false,
                false,
            );
            assert!(result.is_ok(), "create_overlay failed: {result:?}");

            // Files should be at output/my-overlay/, not directly in output/
            let overlay_dir = output.path().join("my-overlay");
            assert!(overlay_dir.exists(), "output/my-overlay/ should exist");
            assert!(
                overlay_dir.join(".envrc").exists(),
                ".envrc should be in output/my-overlay/"
            );
            assert!(
                overlay_dir.join("repoverlay.ccl").exists(),
                "repoverlay.ccl should be in output/my-overlay/"
            );

            // The config should use the correct name
            let config = fs::read_to_string(overlay_dir.join("repoverlay.ccl")).unwrap();
            assert!(
                config.contains("my-overlay"),
                "Config should contain overlay name"
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
                false, // no update for local paths
                ConflictStrategy::default(),
                false,
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
                false, // no update for local paths
                ConflictStrategy::default(),
                false,
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
                false, // no update for local paths
                ConflictStrategy::default(),
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
                false, // no update for local paths
                ConflictStrategy::default(),
                false,
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
                false, // no update for local paths
                ConflictStrategy::Force,
                false,
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
                false, // no update for local paths
                ConflictStrategy::SkipConflicts,
                false,
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
                false, // no update for local paths
                ConflictStrategy::default(),
                false,
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

        /// Issue #205: `sync --all` must skip local-source overlays without
        /// trying to load an overlay-repo config (which may not exist).
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

            // sync --all should succeed: the local overlay is skipped and no
            // overlay repo manager is needed.
            let result = handle_sync(repo.path(), None, true, false);
            assert!(
                result.is_ok(),
                "sync --all should succeed when only local overlays are applied: {result:?}"
            );
        }

        /// Issue #205: `sync --all` must skip GitHub-source overlays without
        /// trying to load an overlay-repo config.
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

            let result = handle_sync(repo.path(), None, true, false);
            assert!(
                result.is_ok(),
                "sync --all should succeed when only GitHub overlays are applied: {result:?}"
            );
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

        /// Issue #205: `sync --all` with a mix of local and GitHub overlays
        /// (but no `OverlayRepo` sources) should skip all and succeed.
        #[test]
        fn handle_sync_all_skips_all_non_syncable_mixed() {
            let repo = create_test_repo();

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

            let result = handle_sync(repo.path(), None, true, false);
            assert!(
                result.is_ok(),
                "sync --all should succeed when all overlays are non-syncable: {result:?}"
            );
        }

        #[test]
        fn try_upgrade_github_source_no_matching_configured_source() {
            // GitHub source with valid subpath but no matching configured source
            // should not be upgraded.
            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "build-troubleshooter",
                OverlaySource::github(
                    "https://github.com/fake-owner-xyz/fake-repo-xyz".to_string(),
                    "fake-owner-xyz".to_string(),
                    "fake-repo-xyz".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    Some("microsoft/FluidFramework/build-troubleshooter".to_string()),
                ),
                vec![(".envrc", ".envrc")],
            );

            let mut state = crate::load_overlay_state(repo.path(), "build-troubleshooter").unwrap();
            assert!(state.source.is_github());

            // No matching configured source → no upgrade
            let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state).unwrap();
            assert!(!upgraded);
            assert!(state.source.is_github());
        }

        #[test]
        fn try_upgrade_github_source_without_subpath_no_upgrade() {
            // A GitHub source without a subpath should not be upgraded
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

            let mut state = crate::load_overlay_state(repo.path(), "direct-github").unwrap();
            assert!(state.source.is_github());

            let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state).unwrap();
            assert!(!upgraded);
            assert!(state.source.is_github());
        }

        #[test]
        fn try_upgrade_github_source_invalid_subpath_no_upgrade() {
            // A GitHub source with an invalid subpath (not 3 parts) should not upgrade
            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "bad-subpath",
                OverlaySource::github(
                    "https://github.com/someone/repo-overlays".to_string(),
                    "someone".to_string(),
                    "repo-overlays".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    Some("only-two/parts".to_string()),
                ),
                vec![(".envrc", ".envrc")],
            );

            let mut state = crate::load_overlay_state(repo.path(), "bad-subpath").unwrap();
            let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state).unwrap();
            assert!(!upgraded);
            assert!(state.source.is_github());
        }

        #[test]
        fn try_upgrade_github_source_empty_subpath_parts_no_upgrade() {
            // A GitHub source with empty parts in subpath (e.g., "org//name") should not upgrade
            let repo = create_test_repo();

            for (name, subpath) in [
                ("empty-middle", "org//name"),
                ("empty-start", "/repo/name"),
                ("empty-end", "org/repo/"),
            ] {
                save_test_state(
                    repo.path(),
                    name,
                    OverlaySource::github(
                        "https://github.com/someone/repo-overlays".to_string(),
                        "someone".to_string(),
                        "repo-overlays".to_string(),
                        "main".to_string(),
                        "abc123def456".to_string(),
                        Some(subpath.to_string()),
                    ),
                    vec![(".envrc", ".envrc")],
                );

                let mut state = crate::load_overlay_state(repo.path(), name).unwrap();
                let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state).unwrap();
                assert!(!upgraded, "Expected no upgrade for subpath '{subpath}'");
                assert!(state.source.is_github());
            }
        }

        #[test]
        fn try_upgrade_github_source_four_part_subpath_no_upgrade() {
            // Too many parts in subpath
            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "too-many-parts",
                OverlaySource::github(
                    "https://github.com/someone/repo-overlays".to_string(),
                    "someone".to_string(),
                    "repo-overlays".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    Some("a/b/c/d".to_string()),
                ),
                vec![(".envrc", ".envrc")],
            );

            let mut state = crate::load_overlay_state(repo.path(), "too-many-parts").unwrap();
            let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state).unwrap();
            assert!(!upgraded);
            assert!(state.source.is_github());
        }

        #[test]
        fn try_upgrade_github_source_non_github_source_no_upgrade() {
            // Non-GitHub sources (Local, OverlayRepo) should not be upgraded
            let repo = create_test_repo();

            // Local source
            save_test_state(
                repo.path(),
                "local-overlay",
                OverlaySource::local(PathBuf::from("/tmp/overlay")),
                vec![(".envrc", ".envrc")],
            );

            let mut state = crate::load_overlay_state(repo.path(), "local-overlay").unwrap();
            let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state);
            assert!(!upgraded.unwrap());

            // OverlayRepo source (already upgraded, should be no-op)
            save_test_state(
                repo.path(),
                "repo-overlay",
                OverlaySource::overlay_repo(
                    "org".to_string(),
                    "repo".to_string(),
                    "name".to_string(),
                    "abc123".to_string(),
                ),
                vec![(".envrc", ".envrc")],
            );

            let mut state = crate::load_overlay_state(repo.path(), "repo-overlay").unwrap();
            let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state).unwrap();
            assert!(!upgraded);
            assert!(state.source.is_overlay_repo());
        }

        #[test]
        fn try_upgrade_github_source_single_part_subpath_no_upgrade() {
            // Single-part subpath (just an overlay name, no org/repo)
            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "single-part",
                OverlaySource::github(
                    "https://github.com/someone/repo-overlays".to_string(),
                    "someone".to_string(),
                    "repo-overlays".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    Some("just-a-name".to_string()),
                ),
                vec![(".envrc", ".envrc")],
            );

            let mut state = crate::load_overlay_state(repo.path(), "single-part").unwrap();
            let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state).unwrap();
            assert!(!upgraded);
            assert!(state.source.is_github());
        }

        #[test]
        fn try_upgrade_github_source_happy_path_with_configured_source() {
            // Test the full upgrade path using a real configured source.
            // Loads user config to find a source; if none configured, the test
            // verifies no upgrade happens (which is also valid).
            use crate::github;

            let config = crate::config::load_config(None).unwrap();
            let Some(source) = config.sources.first() else {
                // No sources configured — nothing to test for the happy path
                return;
            };

            let Some((owner, gh_repo)) = source.url.as_deref().and_then(github::parse_remote_url)
            else {
                return;
            };

            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "test-overlay",
                OverlaySource::github(
                    format!("https://github.com/{owner}/{gh_repo}"),
                    owner,
                    gh_repo,
                    "main".to_string(),
                    "abc123def456".to_string(),
                    Some("test-org/test-repo/test-overlay".to_string()),
                ),
                vec![(".envrc", ".envrc")],
            );

            let mut state = crate::load_overlay_state(repo.path(), "test-overlay").unwrap();
            assert!(state.source.is_github());

            let upgraded = crate::try_upgrade_github_source(repo.path(), &mut state).unwrap();
            assert!(upgraded, "Expected upgrade when source matches config");
            assert!(
                state.source.is_overlay_repo(),
                "Expected OverlayRepo after upgrade"
            );

            // Verify the OverlayRepo fields are correct
            if let OverlaySource::OverlayRepo {
                org,
                repo: target_repo,
                name,
                commit,
                source_name,
                ..
            } = &state.source
            {
                assert_eq!(org, "test-org");
                assert_eq!(target_repo, "test-repo");
                assert_eq!(name, "test-overlay");
                assert_eq!(commit, "abc123def456");
                assert_eq!(source_name.as_deref(), Some(source.name.as_str()));
            } else {
                panic!("Expected OverlayRepo source after upgrade");
            }

            // Verify the state was persisted to disk
            let reloaded = crate::load_overlay_state(repo.path(), "test-overlay").unwrap();
            assert!(reloaded.source.is_overlay_repo());
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
                "--no-update",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Apply {
                    source,
                    target,
                    copy,
                    name,
                    r#ref,
                    no_update,
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
                    assert!(no_update);
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
        fn switch_parses_dry_run() {
            let cli =
                Cli::try_parse_from(["repoverlay", "switch", "./overlay", "--dry-run"]).unwrap();
            match cli.command {
                Some(Commands::Switch { dry_run, .. }) => {
                    assert!(dry_run);
                }
                _ => panic!("Expected Switch command"),
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

        // --- edit subcommand tests ---

        #[test]
        fn edit_add_subcommand_parses() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "add",
                "my-overlay",
                "file1.txt",
                "file2.txt",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit {
                    command: Some(EditCommand::Add { name, files, .. }),
                    ..
                }) => {
                    assert_eq!(name, "my-overlay");
                    assert_eq!(
                        files,
                        vec![PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]
                    );
                }
                _ => panic!("Expected Edit Add subcommand"),
            }
        }

        #[test]
        fn edit_remove_subcommand_parses() {
            let cli =
                Cli::try_parse_from(["repoverlay", "edit", "remove", "my-overlay", "file1.txt"])
                    .unwrap();

            match cli.command {
                Some(Commands::Edit {
                    command: Some(EditCommand::Remove { name, files, .. }),
                    ..
                }) => {
                    assert_eq!(name, "my-overlay");
                    assert_eq!(files, vec![PathBuf::from("file1.txt")]);
                }
                _ => panic!("Expected Edit Remove subcommand"),
            }
        }

        #[test]
        fn edit_add_subcommand_with_target_and_dry_run() {
            let cli = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "add",
                "my-overlay",
                "f.txt",
                "-t",
                "/repo",
                "--dry-run",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit {
                    command:
                        Some(EditCommand::Add {
                            target, dry_run, ..
                        }),
                    ..
                }) => {
                    assert_eq!(target, Some(PathBuf::from("/repo")));
                    assert!(dry_run);
                }
                _ => panic!("Expected Edit Add subcommand"),
            }
        }

        #[test]
        fn edit_no_args_parses_for_interactive_selection() {
            let cli = Cli::try_parse_from(["repoverlay", "edit"]).unwrap();

            match cli.command {
                Some(Commands::Edit {
                    command: None,
                    name: None,
                    ..
                }) => {}
                _ => panic!("Expected Edit with no subcommand and no name"),
            }
        }

        #[test]
        fn edit_name_only_is_interactive() {
            let cli = Cli::try_parse_from(["repoverlay", "edit", "my-overlay"]).unwrap();

            match cli.command {
                Some(Commands::Edit {
                    command: None,
                    name,
                    add,
                    remove,
                    ..
                }) => {
                    assert_eq!(name, Some("my-overlay".to_string()));
                    assert!(add.is_empty());
                    assert!(remove.is_empty());
                }
                _ => panic!("Expected Edit with no subcommand"),
            }
        }

        // --- deprecated flag tests (backwards compat) ---

        #[test]
        fn edit_deprecated_add_flag_parses() {
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
                    command: None,
                    name,
                    add,
                    remove,
                    ..
                }) => {
                    assert_eq!(name, Some("my-overlay".to_string()));
                    assert_eq!(
                        add,
                        vec![PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]
                    );
                    assert!(remove.is_empty());
                }
                _ => panic!("Expected Edit with deprecated flags"),
            }
        }

        #[test]
        fn edit_deprecated_remove_flag_parses() {
            let cli =
                Cli::try_parse_from(["repoverlay", "edit", "my-overlay", "--remove", "file1.txt"])
                    .unwrap();

            match cli.command {
                Some(Commands::Edit {
                    command: None,
                    remove,
                    ..
                }) => {
                    assert_eq!(remove, vec![PathBuf::from("file1.txt")]);
                }
                _ => panic!("Expected Edit with deprecated flags"),
            }
        }

        #[test]
        fn edit_deprecated_combined_add_remove() {
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
                Some(Commands::Edit {
                    command: None,
                    add,
                    remove,
                    ..
                }) => {
                    assert_eq!(add, vec![PathBuf::from("new.txt")]);
                    assert_eq!(remove, vec![PathBuf::from("old.txt")]);
                }
                _ => panic!("Expected Edit with deprecated flags"),
            }
        }

        #[test]
        fn edit_deprecated_interactive_conflicts_with_add() {
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
        fn edit_deprecated_interactive_conflicts_with_remove() {
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
        fn edit_deprecated_dry_run_parses() {
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
        fn edit_deprecated_target_flag_parses() {
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

        /// Issue #143: `add_files_to_overlay` should use `resolve_local_path()` from
        /// the `SourceResolver` trait for Local sources instead of going through the
        /// overlay repo code path. With the fix, `dry_run` succeeds for local sources
        /// by resolving directly to the local path.
        #[test]
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

            // With the SourceResolver fix (#149), add_files_to_overlay uses
            // resolve_local_path() which returns the local path directly for
            // Local sources — no overlay repo lookup needed.
            let result = add_files_to_overlay(
                "org/repo/local-overlay",
                repo.path(),
                &[PathBuf::from("new-file.txt")],
                true, // dry_run to avoid side effects
            );

            // Should succeed: Local sources are mutable and resolve_local_path()
            // returns the stored local path directly.
            assert!(
                result.is_ok(),
                "add_files_to_overlay should succeed for Local sources (dry run). \
                 Got error: {:?}",
                result.unwrap_err()
            );
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

        /// Issue #145: `update_overlays` should distinguish `OverlayRepo` from Local
        /// sources. With the `SourceResolver` fix (#149), `source_type_label()` returns
        /// different labels and `is_updatable()` returns different values for each.
        #[test]
        fn issue_145_update_code_should_handle_overlay_repo_separately() {
            use crate::state::SourceResolver;

            let local_source = OverlaySource::local(PathBuf::from("/path"));
            let repo_source = OverlaySource::overlay_repo(
                "org".to_string(),
                "repo".to_string(),
                "name".to_string(),
                "abc".to_string(),
            );

            // SourceResolver provides distinct labels for each source type
            assert_ne!(
                local_source.source_type_label(),
                repo_source.source_type_label(),
                "Local and OverlayRepo should have different labels"
            );
            assert_eq!(local_source.source_type_label(), "local");
            assert_eq!(repo_source.source_type_label(), "overlay repo");

            // is_updatable() distinguishes them: OverlayRepo is updatable, Local is not
            assert!(
                !local_source.is_updatable(),
                "Local sources should not be updatable"
            );
            assert!(
                repo_source.is_updatable(),
                "OverlayRepo sources should be updatable"
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

        /// Issue #171: `handle_sync` for a single name should call
        /// `try_upgrade_github_source` before checking syncability, just like
        /// the `--all` path does. Without this, a GitHub-sourced overlay that
        /// matches a configured source won't be upgraded and sync will reject it.
        #[test]
        fn issue_171_sync_single_name_calls_try_upgrade_github_source() {
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

            // Create a GitHub-sourced overlay with a valid subpath but no
            // matching configured source. The upgrade will be a no-op, but the
            // code path must still load state as `mut` and call the upgrade.
            save_test_state(
                repo.path(),
                "github-overlay",
                OverlaySource::github(
                    "https://github.com/fake-owner-xyz/fake-repo-xyz".to_string(),
                    "fake-owner-xyz".to_string(),
                    "fake-repo-xyz".to_string(),
                    "main".to_string(),
                    "abc123def456".to_string(),
                    Some("testorg/testrepo/github-overlay".to_string()),
                ),
                vec![(".envrc", ".envrc")],
            );

            let result = handle_sync(
                repo.path(),
                Some("github-overlay".to_string()),
                false, // not --all
                false,
            );

            // Should fail because GitHub sources (without matching config) aren't syncable
            assert!(result.is_err(), "sync should fail for GitHub sources");
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("GitHub") && (msg.contains("sync") || msg.contains("syncable")),
                "sync error for GitHub source should mention the source type. Got: {msg}"
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
