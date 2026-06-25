//! CLI implementation for repoverlay.
//!
//! Defines the command structure using clap and dispatches to `lib.rs` functions.
//! The `run()` function is the internal entry point called from `lib::run()`.

use anyhow::{Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use colored::Colorize;
use std::io::{self};
use std::path::PathBuf;

use crate::reference::SourceReference;
use crate::{
    ApplyOptions, ConflictStrategy, apply_overlay, config, repair_git_exclude, restore_overlays,
    show_status, show_status_json, status_has_overlays, switch_overlay, update_overlays,
    version::{check_for_updates, version_string},
};

pub(crate) mod commands;

pub(crate) use commands::browse::browse_overlays;
pub(crate) use commands::cache::handle_cache_command;
pub(crate) use commands::claude::handle_claude_command;
pub(crate) use commands::copilot::handle_copilot_command;
pub(crate) use commands::create::{create_into_library, create_overlay_command};
pub(crate) use commands::edit::{add_files_to_overlay, edit_overlay, remove_files_from_overlay};
pub(crate) use commands::handle_remove;
pub(crate) use commands::library::handle_library_command;
pub(crate) use commands::marketplace::handle_marketplace_command;
pub(crate) use commands::profile::handle_profile_command;
pub(crate) use commands::source::handle_source_command;
pub(crate) use commands::sync::{handle_sync, select_overlay_interactive};

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
    /// Apply an overlay to a git repository (scripting / power-user)
    ///
    /// Applies an overlay directly from a path, GitHub URL, or configured source.
    /// Intended for scripting and automation workflows.
    ///
    /// For interactive discovery and application, use `repoverlay browse` instead —
    /// it lists available overlays and lets you select which to apply.
    Apply {
        /// Overlay source: local path, GitHub URL, or configured source reference
        ///
        /// Supported formats:
        ///   ./my-overlay             (local path)
        ///   /absolute/path           (local absolute path)
        ///   <https://github.com/owner/repo>
        ///   <https://github.com/owner/repo/tree/main/overlays/rust>
        ///   org/repo/overlay-name    (configured source reference)
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

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Output directory for local overlay creation (no overlay repo required)
        #[arg(short, long, conflicts_with = "into")]
        output: Option<PathBuf>,

        /// Create the overlay directly into a destination ("library")
        #[arg(long, value_name = "DEST", conflicts_with = "output")]
        into: Option<String>,

        /// Skip applying the overlay after creating into the library
        #[arg(long, requires = "into")]
        no_apply: bool,

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

    /// Move an overlay to a new location
    ///
    /// Relocates an overlay's source files and updates applied state references.
    /// Destinations: "library", "source:<name>", or a filesystem path.
    ///
    /// Examples:
    ///   repoverlay move my-overlay --to library
    ///   repoverlay move my-overlay --to /path/to/dir
    ///   repoverlay move my-overlay --to source:shared-repo
    Move {
        /// Name of the applied overlay to move
        overlay: String,

        /// Destination: "library", "source:<name>", or a filesystem path
        #[arg(long)]
        to: String,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Overwrite if destination already exists
        #[arg(long)]
        force: bool,

        /// Rename the overlay at the destination
        #[arg(long)]
        name: Option<String>,

        /// Override the target repository (org/repo format, e.g. acme/my-app)
        ///
        /// Used when moving to a named source and the git origin remote cannot be parsed.
        #[arg(long)]
        target_repo: Option<String>,

        /// Show what would happen without making changes
        #[arg(long)]
        dry_run: bool,
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

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would change without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Manage overlay sources (for multi-source configurations)
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },

    /// Manage plugin marketplaces
    Marketplace {
        #[command(subcommand)]
        command: MarketplaceCommand,
    },

    /// Manage repository profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },

    /// Manage the in-repo overlay library
    Library {
        #[command(subcommand)]
        command: LibraryCommand,
    },

    /// Run GitHub Copilot with one or more profiles applied for the process lifetime
    Copilot {
        /// Profile name to apply while Copilot runs (repeat to apply several)
        #[arg(long = "profile", required = true)]
        profiles: Vec<String>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Extra arguments forwarded to the Copilot harness
        #[arg(last = true)]
        extra_args: Vec<String>,
    },

    /// Run Claude with one or more profiles applied for the process lifetime
    Claude {
        /// Profile name to apply while Claude runs (repeat to apply several)
        #[arg(long = "profile", required = true)]
        profiles: Vec<String>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Extra arguments forwarded to the Claude harness
        #[arg(last = true)]
        extra_args: Vec<String>,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
pub(crate) enum SourceCommand {
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
pub(crate) enum MarketplaceCommand {
    /// Register a plugin marketplace
    Add {
        /// Name for this marketplace (used in `marketplace/plugin` references)
        name: String,

        /// Marketplace git URL or GitHub shorthand (owner/repo)
        url: String,

        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// List registered marketplaces
    List,

    /// Remove a registered marketplace
    Remove {
        /// Name of the marketplace to remove
        name: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProfileCommand {
    /// List configured profiles
    List {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Show a configured profile
    Show {
        /// Profile name
        name: String,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Apply a profile persistently
    Apply {
        /// Profile name
        name: String,

        #[arg(long)]
        harness: crate::profile_applicators::AgentHarness,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Show applied profile state
    Status {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        #[arg(long)]
        harness: Option<crate::profile_applicators::AgentHarness>,
    },

    /// Remove an applied profile
    Remove {
        /// Profile name
        name: String,

        #[arg(long)]
        harness: crate::profile_applicators::AgentHarness,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum CacheCommand {
    /// List cached repositories
    List,

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

#[derive(Subcommand)]
pub(crate) enum LibraryCommand {
    /// List overlays in the library
    List {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Import an overlay into the library
    Import {
        /// Overlay source (path, GitHub URL, or org/repo/name)
        source: String,

        /// Name for the imported overlay (defaults to source name)
        #[arg(long)]
        name: Option<String>,

        /// Force overwrite if overlay already exists
        #[arg(short, long)]
        force: bool,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Export an overlay from the library
    Export {
        /// Name of the overlay to export
        overlay: String,

        /// Destination path
        #[arg(long = "to")]
        dest: String,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Remove an overlay from the library
    Remove {
        /// Name of the overlay to remove
        overlay: String,

        /// Force removal even if overlay is currently applied
        #[arg(short, long)]
        force: bool,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },
}

pub(crate) fn run() -> Result<()> {
    let cli = Cli::parse();

    // Handle markdown help generation (for documentation)
    if cli.markdown_help {
        print!("{}", markdown_help());
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
            let conflict_strategy = conflict_strategy(force, skip_conflicts, interactive);
            // Nudge users toward `browse` when using the three-part configured-source syntax
            // interactively; `apply` remains fully supported for scripting.
            if matches!(
                SourceReference::parse(&source),
                SourceReference::ThreePart { .. }
            ) {
                eprintln!(
                    "{} `repoverlay browse` is the recommended way to interactively discover and apply overlays.",
                    "tip:".cyan().bold(),
                );
            }
            apply_overlay(
                &source,
                &target,
                &ApplyOptions {
                    force_copy: copy,
                    name_override: name,
                    ref_override: r#ref,
                    update_cache: !no_update,
                    conflict_strategy,
                    merge,
                    source_filter: from_source,
                    dry_run,
                },
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

                // Auto-repair git exclude entries if they're out of sync
                // Only run in interactive (non-quiet, non-json) mode
                if matches!(repair_git_exclude(&target), Ok(true)) {
                    eprintln!(
                        "{} Repaired .git/info/exclude entries.",
                        "Maintenance:".cyan().bold()
                    );
                }
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
            let conflict_strategy = conflict_strategy(force, skip_conflicts, interactive);
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
            let conflict_strategy = conflict_strategy(force, skip_conflicts, interactive);
            // Profile plugin re-resolution only runs for a full `update` (no
            // overlay name filter); a name filter scopes the command to one
            // overlay and should not silently churn applied profiles.
            let profiles_checked = if name.is_none() {
                crate::profile_plan::update_profile_plugins(&target, dry_run)?
            } else {
                0
            };
            let overlays_dir = target.join(crate::STATE_DIR).join(crate::OVERLAYS_DIR);
            let have_overlays = overlays_dir.exists()
                && !crate::list_applied_overlays(&target)
                    .unwrap_or_default()
                    .is_empty();
            if name.is_some() || have_overlays {
                update_overlays(&target, name, dry_run, conflict_strategy, merge)?;
            } else if profiles_checked == 0 {
                bail!("No overlays are currently applied in: {}", target.display());
            }
        }
        Commands::Create {
            name,
            include,
            source,
            target,
            output,
            into,
            no_apply,
            dry_run,
            yes,
            force,
        } => {
            let source = source.unwrap_or_else(|| PathBuf::from("."));
            if into.as_deref() == Some("library") {
                let target = target.unwrap_or_else(|| PathBuf::from("."));
                create_into_library(
                    &source, &target, name, &include, dry_run, yes, no_apply, force,
                )?;
            } else if let Some(dest) = &into {
                bail!("Unknown --into destination: {dest}. Valid values: library");
            } else {
                create_overlay_command(&source, name, output, &include, dry_run, yes, force)?;
            }
        }
        Commands::Move {
            overlay,
            to,
            target,
            force,
            name,
            target_repo,
            dry_run,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            commands::r#move::handle_move_command(
                &overlay,
                &to,
                &target,
                force,
                name.as_deref(),
                target_repo.as_deref(),
                dry_run,
            )?;
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
            let conflict_strategy = conflict_strategy(force, skip_conflicts, interactive);
            switch_overlay(
                &source,
                &target,
                &ApplyOptions {
                    force_copy: copy,
                    name_override: name,
                    ref_override: r#ref,
                    update_cache: !no_update, // default: sync before switching
                    conflict_strategy,
                    merge,
                    dry_run,
                    ..ApplyOptions::default()
                },
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
                // When no name is given, interactively select an overlay
                // and go straight to interactive file selection.
                let name = match name {
                    Some(n) => n,
                    None => select_overlay_interactive(&target)?,
                };
                edit_overlay(&name, &target, dry_run)?;
            }
        },
        Commands::Source { command } => {
            handle_source_command(command)?;
        }
        Commands::Marketplace { command } => {
            handle_marketplace_command(command)?;
        }
        Commands::Profile { command } => {
            handle_profile_command(command)?;
        }
        Commands::Library { command } => {
            handle_library_command(command)?;
        }
        Commands::Copilot {
            profiles,
            target,
            extra_args,
        } => {
            handle_copilot_command(&profiles, target, extra_args)?;
        }
        Commands::Claude {
            profiles,
            target,
            extra_args,
        } => {
            handle_claude_command(&profiles, target, extra_args)?;
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

fn markdown_help() -> String {
    clap_markdown::help_markdown::<Cli>()
        .replace("###### **Subcommands:**", "### Subcommands")
        .replace("###### **Arguments:**", "### Arguments")
        .replace("###### **Options:**", "### Options")
}

/// Build a conflict strategy from the mutually-exclusive CLI flags.
const fn conflict_strategy(
    force: bool,
    skip_conflicts: bool,
    interactive: bool,
) -> ConflictStrategy {
    if force {
        ConflictStrategy::Force
    } else if skip_conflicts {
        ConflictStrategy::SkipConflicts
    } else if interactive {
        ConflictStrategy::Interactive
    } else {
        ConflictStrategy::Fail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::create::{detect_target_repo, parse_overlay_name_arg};
    use crate::cli::commands::edit::resolve_overlay_source_path;
    use crate::cli::commands::sync::sync_single_overlay;
    use crate::{create_overlay, remove_overlay};
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    force_copy: true,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    name_override: Some("custom-name".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    name_override: Some("first".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("second".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    conflict_strategy: ConflictStrategy::Force,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            // Apply again with same name using force
            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    conflict_strategy: ConflictStrategy::Force,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    conflict_strategy: ConflictStrategy::SkipConflicts,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            // skip_conflicts should NOT allow re-applying same name
            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    conflict_strategy: ConflictStrategy::SkipConflicts,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    conflict_strategy: ConflictStrategy::Force,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    conflict_strategy: ConflictStrategy::SkipConflicts,
                    ..ApplyOptions::default()
                },
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

        #[cfg(unix)]
        #[test]
        fn skip_conflicts_skips_existing_file_through_symlink_ancestor() {
            use std::os::unix::fs::symlink;

            let repo = create_test_repo();
            let outside = TempDir::new().unwrap();
            fs::write(outside.path().join(".envrc"), "external content").unwrap();
            symlink(outside.path(), repo.path().join("linked")).unwrap();

            let overlay = create_test_overlay(&[
                ("linked/.envrc", "overlay content"),
                ("other.txt", "other content"),
            ]);

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    conflict_strategy: ConflictStrategy::SkipConflicts,
                    ..ApplyOptions::default()
                },
            );

            assert!(result.is_ok(), "skip_conflicts should succeed: {result:?}");
            assert_eq!(
                fs::read_to_string(outside.path().join(".envrc")).unwrap(),
                "external content"
            );
            assert_eq!(
                fs::read_to_string(repo.path().join("other.txt")).unwrap(),
                "other content"
            );
        }

        #[test]
        fn force_fails_on_cross_overlay_file_conflict() {
            let repo = create_test_repo();

            // Apply first overlay with .envrc
            let overlay1 = create_test_overlay(&[(".envrc", "first")]);
            apply_overlay(
                overlay1.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("first".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            // Try to apply second overlay with same file using Force
            let overlay2 = create_test_overlay(&[(".envrc", "second")]);
            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("second".to_string()),
                    conflict_strategy: ConflictStrategy::Force,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("first".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            // Apply second overlay with overlapping file + unique file using SkipConflicts
            let overlay2 =
                create_test_overlay(&[(".envrc", "second"), ("unique.txt", "unique content")]);
            let result = apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("second".to_string()),
                    conflict_strategy: ConflictStrategy::SkipConflicts,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    conflict_strategy: ConflictStrategy::Force,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    conflict_strategy: ConflictStrategy::SkipConflicts,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
            );
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No files found"));
        }

        #[test]
        fn fails_on_nonexistent_source() {
            let repo = create_test_repo();
            let result = apply_overlay("/nonexistent/path", repo.path(), &ApplyOptions::default());
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    force_copy: true,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
            );
            assert!(result.is_ok());

            // Regular file should still be symlinked
            assert!(repo.path().join(".envrc").is_symlink());
            // scratch as a directory symlink should not exist (it was a file in overlay)
            assert!(!repo.path().join("scratch").is_symlink());
        }

        #[test]
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
                &ApplyOptions::default(),
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    name_override: Some("test-overlay".to_string()),
                    dry_run: true,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("overlay-a".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("overlay-b".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("overlay-a".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("overlay-b".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("real-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    force_copy: true,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("overlay-a".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("overlay-b".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("test".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("overlay-a".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("overlay-b".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("overlay-a".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("overlay-b".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("real".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("first-overlay".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            // Verify first overlay is applied
            assert!(repo.path().join(".envrc").exists());

            // Switch to second overlay
            let result = switch_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("second-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("new-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions::default(),
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
                &ApplyOptions {
                    name_override: Some("overlay-a".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();
            apply_overlay(
                overlay2.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("overlay-b".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            // Verify both overlays are applied
            assert!(repo.path().join(".envrc").exists());
            assert!(repo.path().join(".env.local").exists());

            // Switch to third overlay
            switch_overlay(
                overlay3.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    name_override: Some("overlay-c".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    conflict_strategy: ConflictStrategy::Force,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    conflict_strategy: ConflictStrategy::SkipConflicts,
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    name_override: Some("my-overlay".to_string()),
                    ..ApplyOptions::default()
                },
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

        /// Issue #277: `sync --all` must skip library-source overlays without
        /// trying to load an overlay-repo config.
        #[test]
        fn handle_sync_all_skips_library_sources() {
            let repo = create_test_repo();

            save_test_state(
                repo.path(),
                "library-overlay",
                OverlaySource::library("library-overlay".to_string()),
                vec![(".envrc", ".envrc")],
            );

            let result = handle_sync(repo.path(), None, true, false);
            assert!(
                result.is_ok(),
                "sync --all should succeed when only library overlays are applied: {result:?}"
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
            let result = Cli::try_parse_from(["repoverlay", "cache", "clear"]);
            assert!(result.is_err());
        }

        #[test]
        fn cache_clear_with_yes_flag() {
            let result = Cli::try_parse_from(["repoverlay", "cache", "clear", "--yes"]);
            assert!(result.is_err());
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
                    ..
                }) => {
                    assert_eq!(name, Some("my-overlay".to_string()));
                }
                _ => panic!("Expected Edit with no subcommand"),
            }
        }

        // --- removed deprecated flag tests ---

        #[test]
        fn edit_deprecated_add_flag_is_rejected() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--add",
                "file1.txt",
                "file2.txt",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_deprecated_remove_flag_is_rejected() {
            let result =
                Cli::try_parse_from(["repoverlay", "edit", "my-overlay", "--remove", "file1.txt"]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_deprecated_combined_add_remove_is_rejected() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--add",
                "new.txt",
                "--remove",
                "old.txt",
            ]);
            assert!(result.is_err());
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
        fn edit_deprecated_add_with_dry_run_is_rejected() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--add",
                "f.txt",
                "--dry-run",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_deprecated_add_with_target_flag_is_rejected() {
            let result = Cli::try_parse_from([
                "repoverlay",
                "edit",
                "my-overlay",
                "--add",
                "f.txt",
                "-t",
                "/repo",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_deprecated_interactive_flag_is_rejected() {
            let result = Cli::try_parse_from(["repoverlay", "edit", "my-overlay", "--interactive"]);
            assert!(result.is_err());
        }

        #[test]
        fn create_local_command_is_rejected() {
            let result = Cli::try_parse_from(["repoverlay", "create-local", "--help"]);
            assert!(result.is_err());
        }

        #[test]
        fn list_command_is_rejected() {
            let result = Cli::try_parse_from(["repoverlay", "list", "--help"]);
            assert!(result.is_err());
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

        /// Issue #277: `handle_sync` for a single name with a library source
        /// should fail with a clear error about library sources not being
        /// syncable (not a confusing "Could not detect target repository" error).
        #[test]
        fn issue_277_sync_single_rejects_library_source() {
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

            // Create a library-sourced overlay
            save_test_state(
                repo.path(),
                "library-overlay",
                OverlaySource::library("library-overlay".to_string()),
                vec![(".envrc", ".envrc")],
            );

            let result = handle_sync(
                repo.path(),
                Some("library-overlay".to_string()),
                false, // not --all
                false,
            );

            // Should fail because library sources aren't syncable
            assert!(result.is_err(), "sync should fail for library sources");
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("library") && (msg.contains("sync") || msg.contains("syncable")),
                "Bug #277: sync error for library source should mention the source type \
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
