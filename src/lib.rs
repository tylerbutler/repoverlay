//! repoverlay - Overlay config files into git repositories without committing them.
//!
//! This is a CLI-first crate. The only supported library entry point is [`run`].

mod cache;
mod cli;
mod config;
mod create;
mod detection;
mod fs_util;
mod fuzzy;
pub(crate) mod git;
mod git_exclude;
mod github;
mod harness_process;
mod json_merge;
mod library;
mod overlay_name;
mod overlay_repo;
mod path_safety;
mod plugin;
mod profile;
mod profile_applicators;
mod profile_plan;
mod reference;
mod remove;
mod resolve;
mod selection;
mod sources;
mod state;
mod status;
#[cfg(test)]
mod testutil;
mod update;
mod upstream;
mod widgets;

pub(crate) use create::{
    copy_files_to_overlay, create_overlay, expand_include_globs, generate_overlay_config,
    print_overlay_created, restore_overlays, switch_overlay, update_overlays,
};
pub(crate) use git_exclude::{
    ensure_repoverlay_excluded, parse_github_owner_repo, repair_git_exclude, update_git_exclude,
};
pub(crate) use remove::{remove_overlay, remove_single_overlay};
pub(crate) use resolve::{
    ResolvedSource, ResolvedSources, canonicalize_path, get_cached_repo_commit,
    list_overlays_from_cached_repo, resolve_source, try_upgrade_github_source,
};
pub(crate) use status::{show_status, show_status_json, status_has_overlays};

// Re-exports used only by test modules
#[cfg(test)]
pub(crate) use git_exclude::remove_overlay_section;
#[cfg(test)]
pub(crate) use resolve::{
    format_not_found_error, fuzzy_suggest, list_overlays_from_path, resolve_local_path,
    visible_subdirs,
};

/// Run the CLI application.
///
/// This is the only public entry point. All other functionality is internal.
pub fn run() -> anyhow::Result<()> {
    git::install_ctrlc_handler();
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
use json_merge::{JsonMergeError, is_json_file, merge_json_files};
pub(crate) use overlay_name::OverlayName;
use overlay_repo::copy_dir_recursive;
use path_safety::check_no_symlink_ancestors;
use state::{
    CONFIG_FILE, EntryType, FileEntry, GlobalMeta, LinkType, META_FILE, OVERLAYS_DIR,
    OverlayConfig, OverlayState, STATE_DIR, list_applied_overlays, load_all_overlay_targets,
    load_overlay_state, normalize_overlay_name, save_external_state, save_overlay_state,
};

// Re-export git utilities so existing callers (including test modules) continue to work.
#[cfg(test)]
pub(crate) use git::resolve_git_dir;
pub(crate) use git::validate_git_repo;

// Imports used only by test modules
#[cfg(test)]
use state::OverlaySource;

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

    /// Prompt the user interactively for each conflict.
    ///
    /// For each conflicting file, the user can choose to:
    /// - Overwrite the existing file
    /// - Skip the file
    /// - View a diff between existing and overlay files
    /// - Abort the entire apply operation
    Interactive,
}

/// Result of an interactive conflict prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractiveChoice {
    Overwrite,
    Skip,
    Abort,
}

/// Parse result from interactive input parsing.
///
/// Separates "show diff" (a UI action) from terminal choices so the
/// prompt loop can handle them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractiveInput {
    Choice(InteractiveChoice),
    ShowDiff,
    Invalid,
}

/// Parse a single line of user input into an [`InteractiveInput`].
///
/// Accepts both short (`o`, `s`, `d`, `a`) and long (`overwrite`, `skip`,
/// `diff`, `abort`) forms, case-insensitively.
fn parse_interactive_input(input: &str) -> InteractiveInput {
    match input.trim().to_lowercase().as_str() {
        "o" | "overwrite" | "f" | "force" => InteractiveInput::Choice(InteractiveChoice::Overwrite),
        "s" | "skip" => InteractiveInput::Choice(InteractiveChoice::Skip),
        "a" | "abort" => InteractiveInput::Choice(InteractiveChoice::Abort),
        "d" | "diff" => InteractiveInput::ShowDiff,
        _ => InteractiveInput::Invalid,
    }
}

/// Prompt the user interactively for a file conflict resolution.
///
/// Shows the conflict and lets the user choose to overwrite, skip, diff, or abort.
fn prompt_conflict_interactive(
    conflict_path: &Path,
    existing_path: &Path,
    overlay_path: &Path,
    context: &str,
) -> Result<InteractiveChoice> {
    use std::io::{self, Write};

    loop {
        eprint!(
            "  {} {} {}\n  [o]verwrite/[f]orce  [s]kip  [d]iff  [a]bort: ",
            "Conflict:".yellow(),
            conflict_path.display(),
            context,
        );
        io::stderr().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match parse_interactive_input(&input) {
            InteractiveInput::Choice(choice) => return Ok(choice),
            InteractiveInput::ShowDiff => {
                show_file_diff(existing_path, overlay_path, conflict_path);
            }
            InteractiveInput::Invalid => {
                eprintln!("  Invalid choice. Please enter o/f, s, d, or a.");
            }
        }
    }
}

/// Generate a unified diff string between two content strings.
///
/// Returns `None` if the contents are identical.
fn generate_diff(
    existing_content: &str,
    overlay_content: &str,
    display_path: &Path,
) -> Option<String> {
    let diff = similar::TextDiff::from_lines(existing_content, overlay_content);
    let mut unified = diff.unified_diff();
    let formatted = unified
        .header(
            &format!("existing {}", display_path.display()),
            &format!("overlay {}", display_path.display()),
        )
        .to_string();

    if formatted.trim().is_empty() {
        None
    } else {
        Some(formatted)
    }
}

/// Display a unified diff between two files.
fn show_file_diff(existing_path: &Path, overlay_path: &Path, display_path: &Path) {
    let existing_content = match fs::read_to_string(existing_path) {
        Ok(content) => content,
        Err(e) => {
            log::warn!(
                "Failed to read existing file {}: {e}",
                existing_path.display()
            );
            eprintln!(
                "  {} could not read existing file: {e}",
                "warning:".yellow().bold()
            );
            return;
        }
    };
    let overlay_content = match fs::read_to_string(overlay_path) {
        Ok(content) => content,
        Err(e) => {
            log::warn!(
                "Failed to read overlay file {}: {e}",
                overlay_path.display()
            );
            eprintln!(
                "  {} could not read overlay file: {e}",
                "warning:".yellow().bold()
            );
            return;
        }
    };

    match generate_diff(&existing_content, &overlay_content, display_path) {
        None => {
            eprintln!("  (files are identical)");
        }
        Some(formatted) => {
            for line in formatted.lines() {
                if line.starts_with("---") || line.starts_with("+++") {
                    eprintln!("  {}", line.bold());
                } else if line.starts_with("@@") {
                    eprintln!("  {}", line.cyan());
                } else if line.starts_with('-') {
                    eprintln!("  {}", line.red());
                } else if line.starts_with('+') {
                    eprintln!("  {}", line.green());
                } else {
                    eprintln!("  {line}");
                }
            }
        }
    }
    eprintln!();
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

/// RAII guard that removes overlay files created during a single
/// `apply_resolved_overlay` call if the call returns before committing.
///
/// Profile application uses [`ConflictStrategy::Fail`], so any tracked path was
/// newly created by this call (never an overwrite of a pre-existing file), which
/// makes removing it on partial failure safe. On the success path the caller
/// invokes [`OverlayApplyGuard::commit`] so nothing is removed.
struct OverlayApplyGuard {
    target: PathBuf,
    overlay_name: String,
    created: Vec<PathBuf>,
    committed: bool,
}

impl OverlayApplyGuard {
    fn new(target: &Path, overlay_name: &str) -> Self {
        Self {
            target: target.to_path_buf(),
            overlay_name: overlay_name.to_string(),
            created: Vec::new(),
            committed: false,
        }
    }

    fn track(&mut self, path: PathBuf) {
        self.created.push(path);
    }

    const fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for OverlayApplyGuard {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for path in self.created.iter().rev() {
            if let Ok(metadata) = fs::symlink_metadata(path) {
                if metadata.file_type().is_dir() {
                    let _ = fs::remove_dir_all(path);
                } else {
                    let _ = fs::remove_file(path);
                }
            }
        }
        // Best-effort: drop any exclude section written before the failure.
        let _ = update_git_exclude(&self.target, &self.overlay_name, &[], false);
    }
}

/// Apply a single resolved overlay to a target repository.
///
/// This contains the core overlay application logic, separated from source resolution
/// so it can be reused by both single-apply and multi-apply paths.
pub(crate) fn apply_resolved_overlay(
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

    // Resolve composition (extends/includes) if used
    let composition = if uses_composition(&config) {
        let library_path = library::get_library_path(target)?;
        if !source.starts_with(&library_path) {
            bail!(
                "Overlay composition (extends/includes) is only supported for library overlays. \
                 Source '{}' is not in the library.",
                source.display()
            );
        }
        let mut visited = std::collections::HashSet::new();
        Some(resolve_composition(
            source,
            &config,
            &library_path,
            &mut visited,
        )?)
    } else {
        None
    };

    // Determine overlay name (priority: CLI override > config > directory name)
    let overlay_name = resolve_overlay_display_name(&config, source, name_override);
    let normalized_name = normalize_overlay_name(&overlay_name)?;

    // Check if this specific overlay already exists
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    let overlay_state_path = overlays_dir.join(format!("{normalized_name}.ccl"));
    if overlay_state_path.exists() {
        match conflict_strategy {
            ConflictStrategy::Force | ConflictStrategy::Interactive => {
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
    // Track files/directories created during this call so a partial failure
    // (before state is persisted) can be rolled back automatically.
    let mut apply_guard = OverlayApplyGuard::new(target, &normalized_name);

    // Load exclusions from previous external state (survives remove/reapply cycles)
    let previous_exclusions =
        crate::state::load_external_exclusions(target, &normalized_name).unwrap_or_default();
    if !previous_exclusions.is_empty() {
        debug!(
            "loaded {} exclusion(s) from previous state",
            previous_exclusions.len()
        );
        for excl in previous_exclusions {
            state.add_exclusion(excl.path, excl.entry_type);
        }
    }

    // Build the list of directories to process
    let dir_entries: Vec<(String, PathBuf)> = composition.as_ref().map_or_else(
        || {
            config
                .directories
                .iter()
                .map(|d| (d.clone(), source.join(d)))
                .collect()
        },
        |comp| comp.directories.clone(),
    );

    // Process directories first (symlink as units)
    for (dir_name, source_dir) in &dir_entries {
        let dir_path = PathBuf::from(dir_name);
        let source_dir = source_dir.clone();

        // Skip excluded directories
        if state.is_excluded(&dir_path) {
            debug!("skipping excluded directory: {}", dir_path.display());
            continue;
        }

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

        // In symlink mode the whole directory is exposed as-is, so embedded
        // symlinks must not escape the overlay (copy mode checks during copy).
        // Vet before conflict resolution so nothing is removed for a bad overlay.
        if link_type == LinkType::Symlink {
            crate::overlay_repo::ensure_no_escaping_symlinks(&source_dir).with_context(|| {
                format!(
                    "Refusing to apply directory '{}' from overlay '{}'",
                    dir_path.display(),
                    overlay_name
                )
            })?;
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
                ConflictStrategy::Interactive => {
                    let context = format!("(managed by overlay '{conflicting_overlay}')");
                    match prompt_conflict_interactive(
                        &dir_path,
                        &target.join(&dir_path),
                        &source_dir,
                        &context,
                    )? {
                        InteractiveChoice::Skip => continue,
                        InteractiveChoice::Abort => bail!("Aborted by user"),
                        InteractiveChoice::Overwrite => {
                            // Cross-overlay conflicts still fail even in interactive mode
                            bail!(
                                "Conflict: directory '{}' is already managed by overlay '{}'\n\
                                 Remove that overlay first to overwrite.",
                                dir_path.display(),
                                conflicting_overlay
                            );
                        }
                    }
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
                    validate_managed_target_path(target, &dir_path)?;
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
                ConflictStrategy::Interactive => {
                    match prompt_conflict_interactive(
                        &dir_path,
                        &target_dir,
                        &source_dir,
                        "(already exists)",
                    )? {
                        InteractiveChoice::Skip => continue,
                        InteractiveChoice::Abort => bail!("Aborted by user"),
                        InteractiveChoice::Overwrite => {
                            validate_managed_target_path(target, &dir_path)?;
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
                    }
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

        validate_managed_target_path(target, &dir_path)?;

        // Create parent directories if needed
        if let Some(parent) = target_dir.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Create directory symlink or copy
        match link_type {
            LinkType::Symlink => {
                fs_util::create_symlink(&source_dir, &target_dir, fs_util::SymlinkKind::Dir)
                    .with_context(|| {
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

        apply_guard.track(target_dir.clone());
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

    // Build the file list from composition or direct collection
    let file_entries: Vec<(PathBuf, PathBuf, String)> = composition.as_ref().map_or_else(
        || {
            collect_overlay_files(source, &config)
                .into_iter()
                .map(|(rel_path, target_rel)| (source.join(&rel_path), rel_path, target_rel))
                .collect()
        },
        |comp| {
            comp.files
                .iter()
                .map(|f| {
                    (
                        f.source_abs.clone(),
                        f.source_rel.clone(),
                        f.target_rel.clone(),
                    )
                })
                .collect()
        },
    );

    for (source_file, rel_path, target_rel_str) in &file_entries {
        let rel_str = rel_path.to_string_lossy().to_string();
        let target_rel = PathBuf::from(target_rel_str);

        // Skip excluded files
        if state.is_excluded(&target_rel) {
            debug!("skipping excluded file: {}", target_rel.display());
            continue;
        }

        let source_file = source_file.clone();
        let target_file = target.join(&target_rel);

        // Check for conflicts with existing overlays
        if let Some(conflicting_overlay) = existing_targets.get(target_rel_str.as_str()) {
            if merge && is_json_file(&target_rel) && target_file.exists() {
                eprintln!(
                    "  {} Merging '{}' (managed by overlay '{}')",
                    "Merge:".cyan(),
                    target_rel.display(),
                    conflicting_overlay
                );
                if let Some((entry, exclude_path)) =
                    try_merge_json(target, &target_file, &source_file, &target_rel, rel_path)?
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
                ConflictStrategy::Interactive => {
                    let context = format!("(managed by overlay '{conflicting_overlay}')");
                    match prompt_conflict_interactive(
                        &target_rel,
                        &target_file,
                        &source_file,
                        &context,
                    )? {
                        InteractiveChoice::Skip => continue,
                        InteractiveChoice::Abort => bail!("Aborted by user"),
                        InteractiveChoice::Overwrite => {
                            // Cross-overlay conflicts still fail even in interactive mode
                            bail!(
                                "Conflict: file '{}' is already managed by overlay '{}'\n\
                                 Remove that overlay first to overwrite.",
                                target_rel.display(),
                                conflicting_overlay
                            );
                        }
                    }
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
        if target_file.exists() || target_file.is_symlink() {
            if merge && is_json_file(&target_rel) {
                eprintln!(
                    "  {} Merging '{}' with existing repo file",
                    "Merge:".cyan(),
                    target_rel.display()
                );
                if let Some((entry, exclude_path)) =
                    try_merge_json(target, &target_file, &source_file, &target_rel, rel_path)?
                {
                    state.add_file(entry);
                    exclude_entries.push(exclude_path);
                    continue;
                }
                // Merge failed; fall through to existing conflict handling
            }
            match conflict_strategy {
                ConflictStrategy::Force => {
                    validate_managed_target_path(target, &target_rel).with_context(|| {
                        format!(
                            "Unsafe target path for mapping '{}' -> '{}': target paths must stay within the repository and must not contain symlinks",
                            rel_str,
                            target_rel.display()
                        )
                    })?;
                    eprintln!(
                        "  {} Overwriting existing file: {}",
                        "Force:".yellow(),
                        target_rel.display()
                    );
                    fs::remove_file(&target_file).with_context(|| {
                        format!("Failed to remove existing file: {}", target_file.display())
                    })?;
                }
                ConflictStrategy::Interactive => {
                    match prompt_conflict_interactive(
                        &target_rel,
                        &target_file,
                        &source_file,
                        "(already exists)",
                    )? {
                        InteractiveChoice::Skip => continue,
                        InteractiveChoice::Abort => bail!("Aborted by user"),
                        InteractiveChoice::Overwrite => {
                            validate_managed_target_path(target, &target_rel).with_context(
                                || {
                                    format!(
                                        "Unsafe target path for mapping '{}' -> '{}': target paths must stay within the repository and must not contain symlinks",
                                        rel_str,
                                        target_rel.display()
                                    )
                                },
                            )?;
                            eprintln!(
                                "  {} Overwriting existing file: {}",
                                "Force:".yellow(),
                                target_rel.display()
                            );
                            fs::remove_file(&target_file).with_context(|| {
                                format!("Failed to remove existing file: {}", target_file.display())
                            })?;
                        }
                    }
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

        validate_managed_target_path(target, &target_rel).with_context(|| {
            format!(
                "Unsafe target path for mapping '{}' -> '{}': target paths must stay within the repository and must not contain symlinks",
                rel_str,
                target_rel.display()
            )
        })?;

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
                fs_util::create_symlink(&source_file, &target_file, fs_util::SymlinkKind::File)
                    .with_context(|| {
                        format!("Failed to create symlink: {}", target_file.display())
                    })?;
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

        apply_guard.track(target_file.clone());
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

    // Update .git/info/exclude with this overlay's entries. This must succeed:
    // otherwise managed files could be committed accidentally.
    if let Err(e) = update_git_exclude(target, &normalized_name, &exclude_entries, true) {
        rollback_created_overlay_entries(target, &state);
        return Err(e).context(
            "Failed to update git exclude; rolled back created overlay files where practical",
        );
    }

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
    // State is persisted; the overlay is now removable via the normal path, so
    // the partial-apply rollback guard must not delete the applied files.
    apply_guard.commit();

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

fn rollback_created_overlay_entries(target: &Path, state: &OverlayState) {
    for entry in state.file_entries().iter().rev() {
        let path = target.join(&entry.target);
        let result = match entry.entry_type {
            EntryType::Directory => {
                if path.is_symlink() {
                    #[cfg(unix)]
                    {
                        fs::remove_file(&path)
                    }
                    #[cfg(windows)]
                    {
                        fs::remove_dir(&path)
                    }
                } else if path.exists() {
                    fs::remove_dir_all(&path)
                } else {
                    Ok(())
                }
            }
            EntryType::File => {
                if entry.link_type == LinkType::Merged {
                    Ok(())
                } else if path.exists() || path.is_symlink() {
                    fs::remove_file(&path)
                } else {
                    Ok(())
                }
            }
        };

        if let Err(e) = result {
            eprintln!(
                "  {} Could not roll back '{}': {}",
                "Warning:".yellow(),
                entry.target.display(),
                e
            );
        }

        let mut parent = path.parent();
        while let Some(dir) = parent {
            if dir == target {
                break;
            }
            if dir
                .read_dir()
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false)
            {
                if fs::remove_dir(dir).is_err() {
                    break;
                }
                parent = dir.parent();
            } else {
                break;
            }
        }
    }
}

/// Apply multiple overlays atomically.
///
/// Pre-checks for conflicts between the selected overlays and with existing overlays,
/// then applies each overlay in sequence. If any overlay fails to apply, all previously
/// applied overlays from this batch are rolled back.
pub(crate) fn apply_multiple_overlays(
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
                ConflictStrategy::Force | ConflictStrategy::Interactive => {
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
        if let Some(targets) = config.mappings.get(&rel_string) {
            for target in targets {
                files.push((rel_path.to_path_buf(), target.clone()));
            }
        } else {
            files.push((rel_path.to_path_buf(), rel_string));
        }
    }

    files
}

/// A resolved file from overlay composition.
///
/// Unlike `collect_overlay_files` which returns source-relative paths,
/// this includes the absolute source directory so files from different
/// overlay directories can be handled correctly.
struct ResolvedFile {
    /// Absolute path to the source file.
    source_abs: PathBuf,
    /// Relative path within the source overlay (for state tracking).
    source_rel: PathBuf,
    /// Target-relative path in the repo.
    target_rel: String,
}

/// Result of resolving overlay composition (extends + includes).
struct CompositionResult {
    /// Resolved files (from includes, extends, and the overlay itself).
    files: Vec<ResolvedFile>,
    /// Directories to symlink as units (merged from composition chain).
    directories: Vec<(String, PathBuf)>,
}

/// Resolve overlay composition by processing `extends` and `includes`.
///
/// Recursively resolves the composition chain and returns the merged file list.
/// Precedence (highest to lowest): child > extends > includes.
/// Referenced overlays must be library overlays.
fn resolve_composition(
    source: &Path,
    config: &OverlayConfig,
    library_path: &Path,
    visited: &mut std::collections::HashSet<String>,
) -> Result<CompositionResult> {
    use std::collections::HashMap;

    let overlay_name = source.file_name().map_or_else(
        || "unnamed".to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    if !visited.insert(overlay_name.clone()) {
        bail!("Circular extends/includes cycle detected: '{overlay_name}' was already visited");
    }

    // target_rel -> ResolvedFile, preserving insertion order via Vec + HashMap for dedup
    let mut file_map: HashMap<String, ResolvedFile> = HashMap::new();
    // dir_name -> source_dir (absolute)
    let mut dir_map: HashMap<String, PathBuf> = HashMap::new();

    // 1. Process includes (lowest precedence)
    for include in &config.includes {
        let include_source = library_path.join(&include.overlay);
        if !include_source.is_dir() {
            bail!(
                "Included overlay '{}' not found in library at {}",
                include.overlay,
                library_path.display()
            );
        }

        let include_config = load_overlay_config(&include_source)?;
        let resolved =
            resolve_composition(&include_source, &include_config, library_path, visited)?;

        // Build lookups by both target_rel and source_rel for matching
        let mut by_target: HashMap<String, ResolvedFile> = HashMap::new();
        let mut source_to_target: HashMap<String, String> = HashMap::new();
        for f in resolved.files {
            source_to_target.insert(
                f.source_rel.to_string_lossy().to_string(),
                f.target_rel.clone(),
            );
            by_target.insert(f.target_rel.clone(), f);
        }

        // Cherry-pick only the listed files
        for file_name in &include.files {
            let key = if by_target.contains_key(file_name.as_str()) {
                Some(file_name.clone())
            } else {
                source_to_target.get(file_name.as_str()).cloned()
            };

            if let Some(key) = key {
                if let Some(resolved_file) = by_target.remove(&key) {
                    file_map.insert(resolved_file.target_rel.clone(), resolved_file);
                }
            } else {
                bail!(
                    "File '{}' not found in overlay '{}' (resolved from includes)",
                    file_name,
                    include.overlay
                );
            }
        }
    }

    // 2. Process extends (overrides includes)
    if let Some(extends) = &config.extends {
        let parent_source = library_path.join(&extends.overlay);
        if !parent_source.is_dir() {
            bail!(
                "Extended overlay '{}' not found in library at {}",
                extends.overlay,
                library_path.display()
            );
        }

        let parent_config = load_overlay_config(&parent_source)?;
        let resolved = resolve_composition(&parent_source, &parent_config, library_path, visited)?;

        // Parent files override includes
        for file in resolved.files {
            file_map.insert(file.target_rel.clone(), file);
        }

        // Inherit directories from parent
        for (dir_name, dir_source) in resolved.directories {
            dir_map.insert(dir_name, dir_source);
        }
    }

    // 3. Collect child's own files (highest precedence)
    for (rel_path, target_rel) in collect_overlay_files(source, config) {
        file_map.insert(
            target_rel.clone(),
            ResolvedFile {
                source_abs: source.join(&rel_path),
                source_rel: rel_path,
                target_rel,
            },
        );
    }

    // Child's own directories override inherited ones
    for dir_name in &config.directories {
        dir_map.insert(dir_name.clone(), source.join(dir_name));
    }

    // Remove from visited to allow diamond dependencies (A->B->D, A->C->D).
    // True cycles are still caught because the name stays in the set during recursion.
    visited.remove(&overlay_name);

    let mut files: Vec<_> = file_map.into_values().collect();
    files.sort_by(|a, b| a.target_rel.cmp(&b.target_rel));

    let mut directories: Vec<_> = dir_map.into_iter().collect();
    directories.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(CompositionResult { files, directories })
}

/// Check if an overlay config uses composition (extends or includes).
const fn uses_composition(config: &OverlayConfig) -> bool {
    config.extends.is_some() || !config.includes.is_empty()
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

fn validate_managed_target_path(repo_root: &Path, target_rel: &Path) -> Result<()> {
    check_no_symlink_ancestors(repo_root, target_rel).with_context(|| {
        format!(
            "Unsafe target path '{}': target paths must stay within the repository and must not contain symlinks",
            target_rel.display()
        )
    })
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

/// Update .git/info/exclude file.
/// Attempt to deep merge a JSON overlay file into an existing target file.
///
/// Returns `Some((file_entry, exclude_path))` on success, or `None` if JSON parsing/merge failed
/// (with a warning printed to stderr). Path safety violations are returned as errors so callers
/// abort instead of falling through to conflict handling.
fn try_merge_json(
    repo_root: &Path,
    target_file: &Path,
    source_file: &Path,
    target_rel: &Path,
    rel_path: &Path,
) -> Result<Option<(FileEntry, String)>> {
    check_no_symlink_ancestors(repo_root, target_rel).with_context(|| {
        format!(
            "Unsafe JSON merge target '{}': target paths must stay within the repository and must not contain symlinks",
            target_rel.display()
        )
    })?;

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
            Ok(Some((entry, exclude_path)))
        }
        Err(e @ JsonMergeError::Parse { .. }) => {
            eprintln!(
                "  {} JSON merge failed for '{}': {e}",
                "Warning:".yellow(),
                target_rel.display()
            );
            Ok(None)
        }
        Err(e) => Err(anyhow::anyhow!(
            "Failed to merge JSON '{}': {e}",
            target_rel.display()
        )),
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

    // Tests for the overlay partial-apply rollback guard
    mod overlay_apply_guard_tests {
        use super::*;

        #[test]
        fn removes_tracked_files_when_not_committed() {
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("created.txt");
            let nested = dir.path().join("nested");
            fs::create_dir_all(&nested).unwrap();
            fs::write(&file, "data").unwrap();
            fs::write(nested.join("inner.txt"), "data").unwrap();

            {
                let mut guard = OverlayApplyGuard::new(dir.path(), "overlay");
                guard.track(file.clone());
                guard.track(nested.clone());
            }

            assert!(!file.exists());
            assert!(!nested.exists());
        }

        #[test]
        fn preserves_tracked_files_after_commit() {
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("created.txt");
            fs::write(&file, "data").unwrap();

            {
                let mut guard = OverlayApplyGuard::new(dir.path(), "overlay");
                guard.track(file.clone());
                guard.commit();
            }

            assert!(file.exists());
        }

        #[cfg(unix)]
        #[test]
        fn removes_symlink_without_touching_target() {
            let dir = TempDir::new().unwrap();
            let outside = dir.path().join("outside.txt");
            fs::write(&outside, "keep").unwrap();
            let link = dir.path().join("link.txt");
            std::os::unix::fs::symlink(&outside, &link).unwrap();

            {
                let mut guard = OverlayApplyGuard::new(dir.path(), "overlay");
                guard.track(link.clone());
            }

            assert!(!link.exists());
            assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
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
                OverlaySource::Local { path, .. } => {
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

    mod apply_path_safety_tests {
        use super::*;

        fn write_overlay_file(dir: &Path, path: &str, content: &str) {
            let file_path = dir.join(path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(file_path, content).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn apply_rejects_normal_file_target_through_symlink_ancestor() {
            use std::os::unix::fs::symlink;

            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            let outside = TempDir::new().unwrap();

            symlink(outside.path(), repo.path().join("linked")).unwrap();
            write_overlay_file(overlay.path(), "linked/config.txt", "secret");

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                false,
                Some("unsafe".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            );

            assert!(result.is_err());
            let message = result.unwrap_err().to_string();
            assert!(
                message.contains("Unsafe target path") || message.contains("symlink"),
                "unexpected error: {message}"
            );
            assert!(!outside.path().join("config.txt").exists());
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
                false,
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
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Unsafe target path"),
                "error should mention unsafe target path"
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
        fn rejects_traversal_even_when_it_would_stay_within_target() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("file.txt", "content")],
                "mappings =\n  file.txt = foo/../bar\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            assert!(
                result.is_err(),
                "foo/../bar should be rejected by the 1.0 path policy"
            );
        }

        #[test]
        fn rejects_deeper_traversal_even_when_it_would_stay_within_target() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("file.txt", "content")],
                "mappings =\n  file.txt = foo/bar/../../baz\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            assert!(
                result.is_err(),
                "foo/bar/../../baz should be rejected by the 1.0 path policy"
            );
        }

        #[test]
        fn rejects_absolute_unix_path_in_mapping() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("secret.txt", "payload")],
                "mappings =\n  secret.txt = /etc/passwd\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            assert!(
                result.is_err(),
                "should reject absolute path /etc/passwd in mapping"
            );
        }

        #[test]
        fn windows_style_absolute_path_in_mapping_on_unix() {
            // FINDING: On Unix, Windows-style absolute paths (C:\...) are treated as
            // relative paths because backslash is a valid filename character on Unix
            // and the path doesn't start with '/'. This is a known gap — Windows-style
            // paths are only dangerous on Windows systems where they resolve as absolute.
            // On Unix, `C:\Windows\System32\cmd.exe` creates a file literally named
            // that relative to the target, which is safe (stays within target dir).
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("secret.txt", "payload")],
                "mappings =\n  secret.txt = C:\\Windows\\System32\\cmd.exe\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            // On Unix this succeeds (backslash is a valid filename char, path is relative)
            // On Windows this should be rejected (absolute path). Document as known gap.
            if cfg!(unix) {
                assert!(
                    result.is_ok(),
                    "On Unix, Windows-style paths are treated as relative: {result:?}"
                );
            } else {
                assert!(
                    result.is_err(),
                    "On Windows, should reject absolute path C:\\Windows\\System32\\cmd.exe"
                );
            }
        }

        #[test]
        fn rejects_escape_through_deep_chain() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            make_overlay_with_config(
                overlay.path(),
                &[("file.txt", "content")],
                "mappings =\n  file.txt = a/b/c/../../../../../../../etc/passwd\n",
            );

            let result = try_apply(overlay.path(), repo.path());
            assert!(
                result.is_err(),
                "should reject deep chain traversal that escapes target"
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
                false,
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

        #[test]
        #[cfg(unix)]
        fn directory_symlink_mode_rejects_escaping_symlink_inside_directory() {
            let repo = create_test_repo();
            let parent = TempDir::new().unwrap();
            let overlay = parent.path().join("overlay");
            let outside = parent.path().join("outside");
            fs::create_dir_all(overlay.join(".claude")).unwrap();
            fs::create_dir_all(&outside).unwrap();
            fs::write(outside.join("secret.txt"), "secret").unwrap();

            fs::write(overlay.join(".claude/CLAUDE.md"), "instructions").unwrap();
            // Malicious symlink inside the declared directory escaping the overlay
            std::os::unix::fs::symlink(&outside, overlay.join(".claude/evil")).unwrap();
            fs::write(
                overlay.join(CONFIG_FILE),
                "overlay =\n  name = test\n\ndirectories =\n  = .claude\n",
            )
            .unwrap();

            let resolved = ResolvedSource {
                path: overlay.clone(),
                source_info: OverlaySource::local(overlay),
            };
            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_resolved_overlay(
                &resolved,
                &canonical,
                false, // symlink mode: the whole directory would be linked as-is
                None,
                ConflictStrategy::default(),
                false,
            );
            let err = result.expect_err("apply should reject directory with escaping symlink");
            assert!(
                err.to_string().contains("escape") || format!("{err:#}").contains("escape"),
                "error should mention symlink escape: {err:#}"
            );
            assert!(
                !canonical.join(".claude").exists(),
                "directory must not be linked into the target"
            );
        }

        #[test]
        #[cfg(unix)]
        fn directory_symlink_mode_allows_internal_symlinks() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::create_dir_all(overlay.path().join(".claude")).unwrap();
            fs::write(overlay.path().join(".claude/CLAUDE.md"), "instructions").unwrap();
            // Relative symlink staying inside the declared directory is fine
            std::os::unix::fs::symlink("CLAUDE.md", overlay.path().join(".claude/alias.md"))
                .unwrap();
            fs::write(
                overlay.path().join(CONFIG_FILE),
                "overlay =\n  name = test\n\ndirectories =\n  = .claude\n",
            )
            .unwrap();

            let resolved = ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::local(overlay.path().to_path_buf()),
            };
            let canonical = repo.path().canonicalize().unwrap();
            let result = apply_resolved_overlay(
                &resolved,
                &canonical,
                false,
                None,
                ConflictStrategy::default(),
                false,
            );
            assert!(result.is_ok(), "apply should succeed: {result:?}");
            assert!(
                canonical.join(".claude/CLAUDE.md").exists(),
                "directory should be linked into the target"
            );
        }
    }

    #[test]
    fn test_overlay_items_mark_applied_as_disabled() {
        let available = [
            "org/repo/a".to_string(),
            "org/repo/b".to_string(),
            "org/repo/c".to_string(),
        ];
        let applied = ["org/repo/b".to_string()];
        let applied_set: std::collections::HashSet<&str> =
            applied.iter().map(String::as_str).collect();

        let items: Vec<_> = available
            .iter()
            .map(|o| {
                let disabled = applied_set.contains(o.as_str());
                (o.clone(), disabled)
            })
            .collect();

        assert!(!items[0].1); // a not disabled
        assert!(items[1].1); // b disabled (already applied)
        assert!(!items[2].1); // c not disabled
    }

    // Tests for fuzzy_suggest
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

    // Tests for format_not_found_error
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

    // Tests for resolve_git_dir additional edge cases
    mod resolve_git_dir_additional_tests {
        use super::*;

        #[test]
        fn git_file_with_extra_whitespace() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            let worktree_git_dir = temp.path().join("actual-git-dir");
            fs::create_dir_all(&worktree_git_dir).unwrap();

            // Extra whitespace around gitdir path
            let git_file_content = format!("gitdir:   {}  \n", worktree_git_dir.display());
            fs::write(repo_path.join(".git"), git_file_content).unwrap();

            let result = resolve_git_dir(repo_path);
            assert!(result.is_ok());
        }

        #[test]
        fn git_file_pointing_to_nonexistent_directory() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            fs::write(
                repo_path.join(".git"),
                "gitdir: /nonexistent/path/abc123xyz\n",
            )
            .unwrap();

            let result = resolve_git_dir(repo_path);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Failed to resolve gitdir path")
            );
        }
    }

    // Tests for list_overlays_from_path additional edge cases
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

    // Tests for conflict_strategy
    mod conflict_strategy_tests {
        use super::*;

        #[test]
        fn default_is_fail() {
            assert_eq!(ConflictStrategy::default(), ConflictStrategy::Fail);
        }

        #[test]
        fn enum_equality() {
            assert_eq!(ConflictStrategy::Fail, ConflictStrategy::Fail);
            assert_eq!(ConflictStrategy::Force, ConflictStrategy::Force);
            assert_eq!(
                ConflictStrategy::SkipConflicts,
                ConflictStrategy::SkipConflicts
            );
            assert_eq!(ConflictStrategy::Interactive, ConflictStrategy::Interactive);
            assert_ne!(ConflictStrategy::Fail, ConflictStrategy::Force);
            assert_ne!(ConflictStrategy::Fail, ConflictStrategy::Interactive);
            assert_ne!(ConflictStrategy::Force, ConflictStrategy::Interactive);
            assert_ne!(
                ConflictStrategy::SkipConflicts,
                ConflictStrategy::Interactive
            );
        }

        #[test]
        fn debug_format() {
            assert_eq!(format!("{:?}", ConflictStrategy::Fail), "Fail");
            assert_eq!(format!("{:?}", ConflictStrategy::Force), "Force");
            assert_eq!(
                format!("{:?}", ConflictStrategy::SkipConflicts),
                "SkipConflicts"
            );
            assert_eq!(
                format!("{:?}", ConflictStrategy::Interactive),
                "Interactive"
            );
        }
    }

    mod interactive_input_tests {
        use super::*;

        #[test]
        fn short_overwrite() {
            assert_eq!(
                parse_interactive_input("o"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
        }

        #[test]
        fn long_overwrite() {
            assert_eq!(
                parse_interactive_input("overwrite"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
        }

        #[test]
        fn short_skip() {
            assert_eq!(
                parse_interactive_input("s"),
                InteractiveInput::Choice(InteractiveChoice::Skip)
            );
        }

        #[test]
        fn long_skip() {
            assert_eq!(
                parse_interactive_input("skip"),
                InteractiveInput::Choice(InteractiveChoice::Skip)
            );
        }

        #[test]
        fn short_abort() {
            assert_eq!(
                parse_interactive_input("a"),
                InteractiveInput::Choice(InteractiveChoice::Abort)
            );
        }

        #[test]
        fn long_abort() {
            assert_eq!(
                parse_interactive_input("abort"),
                InteractiveInput::Choice(InteractiveChoice::Abort)
            );
        }

        #[test]
        fn short_diff() {
            assert_eq!(parse_interactive_input("d"), InteractiveInput::ShowDiff);
        }

        #[test]
        fn long_diff() {
            assert_eq!(parse_interactive_input("diff"), InteractiveInput::ShowDiff);
        }

        #[test]
        fn case_insensitive() {
            assert_eq!(
                parse_interactive_input("O"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
            assert_eq!(
                parse_interactive_input("OVERWRITE"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
            assert_eq!(
                parse_interactive_input("Skip"),
                InteractiveInput::Choice(InteractiveChoice::Skip)
            );
            assert_eq!(parse_interactive_input("DIFF"), InteractiveInput::ShowDiff);
        }

        #[test]
        fn whitespace_trimmed() {
            assert_eq!(
                parse_interactive_input("  o  "),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
            assert_eq!(
                parse_interactive_input("\ts\n"),
                InteractiveInput::Choice(InteractiveChoice::Skip)
            );
        }

        #[test]
        fn invalid_input() {
            assert_eq!(parse_interactive_input("x"), InteractiveInput::Invalid);
            assert_eq!(parse_interactive_input("yes"), InteractiveInput::Invalid);
            assert_eq!(parse_interactive_input(""), InteractiveInput::Invalid);
            assert_eq!(parse_interactive_input("oo"), InteractiveInput::Invalid);
            assert_eq!(
                parse_interactive_input("overwritee"),
                InteractiveInput::Invalid
            );
        }

        #[test]
        fn interactive_choice_clone_and_debug() {
            let choice = InteractiveChoice::Overwrite;
            let cloned = choice;
            assert_eq!(choice, cloned);
            assert_eq!(format!("{choice:?}"), "Overwrite");
        }

        #[test]
        fn interactive_input_clone_and_debug() {
            let input = InteractiveInput::ShowDiff;
            let cloned = input;
            assert_eq!(input, cloned);
            assert_eq!(format!("{input:?}"), "ShowDiff");
        }

        #[test]
        fn short_force_alias() {
            assert_eq!(
                parse_interactive_input("f"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
        }

        #[test]
        fn long_force_alias() {
            assert_eq!(
                parse_interactive_input("force"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
        }

        #[test]
        fn force_alias_case_insensitive() {
            assert_eq!(
                parse_interactive_input("F"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
            assert_eq!(
                parse_interactive_input("FORCE"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
            assert_eq!(
                parse_interactive_input("Force"),
                InteractiveInput::Choice(InteractiveChoice::Overwrite)
            );
        }

        #[test]
        fn newline_only_is_invalid() {
            assert_eq!(parse_interactive_input("\n"), InteractiveInput::Invalid);
        }

        #[test]
        fn partial_keywords_are_invalid() {
            assert_eq!(parse_interactive_input("ov"), InteractiveInput::Invalid);
            assert_eq!(parse_interactive_input("sk"), InteractiveInput::Invalid);
            assert_eq!(parse_interactive_input("ab"), InteractiveInput::Invalid);
            assert_eq!(parse_interactive_input("di"), InteractiveInput::Invalid);
            assert_eq!(parse_interactive_input("fo"), InteractiveInput::Invalid);
        }
    }

    mod generate_diff_tests {
        use super::*;

        #[test]
        fn identical_content_returns_none() {
            let content = "hello\nworld\n";
            assert!(generate_diff(content, content, Path::new("test.txt")).is_none());
        }

        #[test]
        fn different_content_returns_some() {
            let existing = "hello\n";
            let overlay = "world\n";
            let result = generate_diff(existing, overlay, Path::new("test.txt"));
            assert!(result.is_some());
            let diff = result.unwrap();
            assert!(diff.contains("-hello"));
            assert!(diff.contains("+world"));
        }

        #[test]
        fn diff_contains_header() {
            let existing = "a\n";
            let overlay = "b\n";
            let result = generate_diff(existing, overlay, Path::new("config.yml")).unwrap();
            assert!(result.contains("existing config.yml"));
            assert!(result.contains("overlay config.yml"));
        }

        #[test]
        fn empty_existing_shows_additions() {
            let result = generate_diff("", "new content\n", Path::new("f.txt")).unwrap();
            assert!(result.contains("+new content"));
        }

        #[test]
        fn empty_overlay_shows_removals() {
            let result = generate_diff("old content\n", "", Path::new("f.txt")).unwrap();
            assert!(result.contains("-old content"));
        }

        #[test]
        fn multiline_diff() {
            let existing = "line1\nline2\nline3\n";
            let overlay = "line1\nchanged\nline3\n";
            let result = generate_diff(existing, overlay, Path::new("f.txt")).unwrap();
            assert!(result.contains("-line2"));
            assert!(result.contains("+changed"));
            // Unchanged lines should still appear as context
            assert!(result.contains(" line1"));
            assert!(result.contains(" line3"));
        }

        #[test]
        fn both_empty_returns_none() {
            assert!(generate_diff("", "", Path::new("f.txt")).is_none());
        }

        #[test]
        fn unicode_content() {
            let existing = "héllo wörld\n日本語\n";
            let overlay = "héllo wörld\nchanged\n";
            let result = generate_diff(existing, overlay, Path::new("f.txt")).unwrap();
            assert!(result.contains("-日本語"));
            assert!(result.contains("+changed"));
        }

        #[test]
        fn single_line_no_trailing_newline() {
            let existing = "old";
            let overlay = "new";
            let result = generate_diff(existing, overlay, Path::new("f.txt")).unwrap();
            assert!(result.contains("-old"));
            assert!(result.contains("+new"));
        }

        #[test]
        fn whitespace_only_changes() {
            let existing = "line1\nline2\n";
            let overlay = "line1\nline2 \n";
            let result = generate_diff(existing, overlay, Path::new("f.txt"));
            assert!(result.is_some());
        }
    }

    mod show_file_diff_tests {
        use super::*;

        #[test]
        fn handles_nonexistent_existing_file() {
            let tmp = tempfile::TempDir::new().unwrap();
            let overlay = tmp.path().join("overlay.txt");
            fs::write(&overlay, "content\n").unwrap();
            // Should not panic when existing file doesn't exist
            show_file_diff(
                &tmp.path().join("nonexistent.txt"),
                &overlay,
                Path::new("test.txt"),
            );
        }

        #[test]
        fn handles_nonexistent_overlay_file() {
            let tmp = tempfile::TempDir::new().unwrap();
            let existing = tmp.path().join("existing.txt");
            fs::write(&existing, "content\n").unwrap();
            // Should not panic when overlay file doesn't exist
            show_file_diff(
                &existing,
                &tmp.path().join("nonexistent.txt"),
                Path::new("test.txt"),
            );
        }

        #[test]
        fn handles_identical_files() {
            let tmp = tempfile::TempDir::new().unwrap();
            let existing = tmp.path().join("existing.txt");
            let overlay = tmp.path().join("overlay.txt");
            fs::write(&existing, "same\n").unwrap();
            fs::write(&overlay, "same\n").unwrap();
            // Should not panic with identical content
            show_file_diff(&existing, &overlay, Path::new("test.txt"));
        }

        #[test]
        fn handles_different_files() {
            let tmp = tempfile::TempDir::new().unwrap();
            let existing = tmp.path().join("existing.txt");
            let overlay = tmp.path().join("overlay.txt");
            fs::write(&existing, "old\n").unwrap();
            fs::write(&overlay, "new\n").unwrap();
            // Should not panic with different content
            show_file_diff(&existing, &overlay, Path::new("test.txt"));
        }

        #[test]
        fn handles_both_files_nonexistent() {
            let tmp = tempfile::TempDir::new().unwrap();
            // Should not panic when neither file exists (both read as empty → identical)
            show_file_diff(
                &tmp.path().join("a.txt"),
                &tmp.path().join("b.txt"),
                Path::new("test.txt"),
            );
        }

        #[test]
        fn handles_empty_files() {
            let tmp = tempfile::TempDir::new().unwrap();
            let existing = tmp.path().join("existing.txt");
            let overlay = tmp.path().join("overlay.txt");
            fs::write(&existing, "").unwrap();
            fs::write(&overlay, "").unwrap();
            // Should not panic with empty content
            show_file_diff(&existing, &overlay, Path::new("test.txt"));
        }
    }

    mod interactive_apply_tests {
        use super::*;
        use crate::testutil::{create_test_overlay, create_test_repo};

        #[cfg(unix)]
        #[test]
        fn merge_rejects_symlink_target_without_touching_external_file() {
            use std::os::unix::fs::symlink;

            let repo = create_test_repo();
            let overlay = create_test_overlay(&[("settings.json", r#"{"overlay": true}"#)]);
            let external_dir = TempDir::new().unwrap();
            let external = external_dir.path().join("external.json");
            fs::write(&external, r#"{"external": true}"#).unwrap();
            symlink(&external, repo.path().join("settings.json")).unwrap();

            let resolved = ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::Local {
                    path: overlay.path().to_path_buf(),
                    source_name: None,
                },
            };

            let result = apply_resolved_overlay(
                &resolved,
                repo.path(),
                true,
                Some("symlink-target".to_string()),
                ConflictStrategy::Fail,
                true,
            );

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("symlink"));
            assert_eq!(
                fs::read_to_string(&external).unwrap(),
                r#"{"external": true}"#
            );
        }

        #[cfg(unix)]
        #[test]
        fn merge_rejects_symlink_ancestor_without_touching_external_file() {
            use std::os::unix::fs::symlink;

            let repo = create_test_repo();
            let overlay = create_test_overlay(&[("config/settings.json", r#"{"overlay": true}"#)]);
            let external_dir = TempDir::new().unwrap();
            let external = external_dir.path().join("settings.json");
            fs::write(&external, r#"{"external": true}"#).unwrap();
            symlink(external_dir.path(), repo.path().join("config")).unwrap();

            let resolved = ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::Local {
                    path: overlay.path().to_path_buf(),
                    source_name: None,
                },
            };

            let result = apply_resolved_overlay(
                &resolved,
                repo.path(),
                true,
                Some("symlink-ancestor".to_string()),
                ConflictStrategy::Fail,
                true,
            );

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("symlink"));
            assert_eq!(
                fs::read_to_string(&external).unwrap(),
                r#"{"external": true}"#
            );
        }

        #[cfg(unix)]
        #[test]
        fn merge_write_failure_aborts_instead_of_skip_conflicts_fallback() {
            use std::os::unix::fs::PermissionsExt;

            let repo = create_test_repo();
            let overlay = create_test_overlay(&[("locked/settings.json", r#"{"overlay": true}"#)]);
            let locked_dir = repo.path().join("locked");
            fs::create_dir(&locked_dir).unwrap();
            let target = locked_dir.join("settings.json");
            fs::write(&target, r#"{"repo": true}"#).unwrap();
            let original_permissions = fs::metadata(&locked_dir).unwrap().permissions();
            let mut readonly_permissions = original_permissions.clone();
            readonly_permissions.set_mode(0o555);
            fs::set_permissions(&locked_dir, readonly_permissions).unwrap();

            let resolved = ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::Local {
                    path: overlay.path().to_path_buf(),
                    source_name: None,
                },
            };

            let result = apply_resolved_overlay(
                &resolved,
                repo.path(),
                true,
                Some("write-failure".to_string()),
                ConflictStrategy::SkipConflicts,
                true,
            );

            fs::set_permissions(&locked_dir, original_permissions).unwrap();

            assert!(result.is_err());
            let error = result.unwrap_err().to_string();
            assert!(error.contains("write"), "{error}");
            assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"repo": true}"#);
        }

        #[test]
        fn apply_fails_and_rolls_back_files_when_git_exclude_cannot_be_updated() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[("nested/file.txt", "content")]);
            let exclude_path = repo.path().join(".git/info/exclude");
            fs::remove_file(&exclude_path).unwrap();
            fs::create_dir(&exclude_path).unwrap();

            let resolved = ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::Local {
                    path: overlay.path().to_path_buf(),
                    source_name: None,
                },
            };

            let result = apply_resolved_overlay(
                &resolved,
                repo.path(),
                true,
                Some("exclude-fails".to_string()),
                ConflictStrategy::Fail,
                false,
            );

            assert!(result.is_err());
            let error = result.unwrap_err().to_string();
            assert!(error.contains("Failed to update git exclude"));
            assert!(!repo.path().join("nested/file.txt").exists());
            assert!(!repo.path().join("nested").exists());
            assert!(
                !repo
                    .path()
                    .join(STATE_DIR)
                    .join(OVERLAYS_DIR)
                    .join("exclude-fails.ccl")
                    .exists()
            );
        }

        #[test]
        fn interactive_reapplies_same_name_overlay() {
            // When an overlay with the same name is already applied,
            // Interactive strategy should auto-remove and re-apply (like Force).
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[("test.txt", "original")]);

            let resolved = ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::Local {
                    path: overlay.path().to_path_buf(),
                    source_name: None,
                },
            };
            apply_resolved_overlay(
                &resolved,
                repo.path(),
                true,
                Some("my-overlay".to_string()),
                ConflictStrategy::Fail,
                false,
            )
            .unwrap();
            assert_eq!(
                fs::read_to_string(repo.path().join("test.txt")).unwrap(),
                "original"
            );

            // Apply again with same name and Interactive strategy
            let overlay2 = create_test_overlay(&[("test.txt", "updated")]);
            let resolved2 = ResolvedSource {
                path: overlay2.path().to_path_buf(),
                source_info: OverlaySource::Local {
                    path: overlay2.path().to_path_buf(),
                    source_name: None,
                },
            };
            apply_resolved_overlay(
                &resolved2,
                repo.path(),
                true,
                Some("my-overlay".to_string()),
                ConflictStrategy::Interactive,
                false,
            )
            .unwrap();

            // The file should now have the updated content
            let content = fs::read_to_string(repo.path().join("test.txt")).unwrap();
            assert_eq!(content, "updated");
        }

        #[test]
        fn interactive_batch_reapplies_same_name_overlay() {
            // In batch mode, Interactive should also auto-remove existing same-name overlays.
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[("a.txt", "first")]);
            // Add a config so the overlay name resolves consistently
            fs::write(
                overlay.path().join(CONFIG_FILE),
                "overlay =\n  name = batch-test\n",
            )
            .unwrap();

            let resolved = ResolvedSource {
                path: overlay.path().to_path_buf(),
                source_info: OverlaySource::Local {
                    path: overlay.path().to_path_buf(),
                    source_name: None,
                },
            };
            apply_resolved_overlay(
                &resolved,
                repo.path(),
                true,
                Some("batch-test".to_string()),
                ConflictStrategy::Fail,
                false,
            )
            .unwrap();

            // Re-apply via batch with Interactive — the config name ensures
            // apply_multiple_overlays resolves the same "batch-test" name and
            // auto-removes the existing overlay instead of prompting on stdin.
            let overlay2 = create_test_overlay(&[("a.txt", "second")]);
            fs::write(
                overlay2.path().join(CONFIG_FILE),
                "overlay =\n  name = batch-test\n",
            )
            .unwrap();
            let resolved2 = ResolvedSource {
                path: overlay2.path().to_path_buf(),
                source_info: OverlaySource::Local {
                    path: overlay2.path().to_path_buf(),
                    source_name: None,
                },
            };
            apply_multiple_overlays(
                &[resolved2],
                repo.path(),
                true,
                false,
                ConflictStrategy::Interactive,
                false,
            )
            .unwrap();

            let content = fs::read_to_string(repo.path().join("a.txt")).unwrap();
            assert_eq!(content, "second");
        }
    }

    // Tests for resolve_git_dir
    mod resolve_git_dir_tests {
        use super::*;

        #[test]
        fn regular_git_repo_returns_git_dir() {
            let repo = create_test_repo();
            let result = resolve_git_dir(repo.path()).unwrap();
            assert!(result.is_dir());
            // The resolved path should end with .git
            assert!(result.ends_with(".git") || result.to_string_lossy().contains(".git"));
        }

        #[test]
        fn worktree_with_relative_gitdir() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path().join("worktree");
            fs::create_dir_all(&repo_path).unwrap();

            // Create the actual git dir relative to the worktree
            let actual_git_dir = temp.path().join("actual-git-dir");
            fs::create_dir_all(&actual_git_dir).unwrap();

            // Write a .git file with relative path
            let relative_path = "../actual-git-dir";
            fs::write(repo_path.join(".git"), format!("gitdir: {relative_path}\n")).unwrap();

            let result = resolve_git_dir(&repo_path).unwrap();
            assert!(result.is_dir());
        }

        #[test]
        fn worktree_with_absolute_gitdir() {
            let temp = TempDir::new().unwrap();
            let repo_path = temp.path();

            let actual_git_dir = temp.path().join("actual-git-dir");
            fs::create_dir_all(&actual_git_dir).unwrap();

            let abs_path = actual_git_dir.canonicalize().unwrap();
            fs::write(
                repo_path.join(".git"),
                format!("gitdir: {}\n", abs_path.display()),
            )
            .unwrap();

            let result = resolve_git_dir(repo_path).unwrap();
            assert!(result.is_dir());
        }

        #[test]
        fn git_file_without_gitdir_prefix_fails() {
            let temp = TempDir::new().unwrap();
            fs::write(
                temp.path().join(".git"),
                "some random content\nno gitdir here\n",
            )
            .unwrap();

            let result = resolve_git_dir(temp.path());
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("no gitdir found"));
        }

        #[test]
        fn no_git_at_all_fails() {
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
    }

    // Tests for list_overlays_from_path
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

    // Tests for visible_subdirs additional edge cases
    mod visible_subdirs_edge_cases {
        use super::*;

        #[test]
        fn only_hidden_directories_returns_empty() {
            let temp = TempDir::new().unwrap();
            fs::create_dir(temp.path().join(".hidden1")).unwrap();
            fs::create_dir(temp.path().join(".hidden2")).unwrap();

            let result = visible_subdirs(temp.path()).unwrap();
            assert!(result.is_empty());
        }

        #[test]
        fn only_files_returns_empty() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("file1.txt"), "").unwrap();
            fs::write(temp.path().join("file2.txt"), "").unwrap();

            let result = visible_subdirs(temp.path()).unwrap();
            assert!(result.is_empty());
        }

        #[test]
        fn mixed_visible_hidden_and_files() {
            let temp = TempDir::new().unwrap();
            fs::create_dir(temp.path().join("visible")).unwrap();
            fs::create_dir(temp.path().join(".hidden")).unwrap();
            fs::write(temp.path().join("file.txt"), "").unwrap();

            let result = visible_subdirs(temp.path()).unwrap();
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].1, "visible");
        }

        #[test]
        fn nonexistent_directory_returns_error() {
            let result = visible_subdirs(Path::new("/nonexistent/path/abc123"));
            assert!(result.is_err());
        }
    }

    // Tests for resolve_overlay_display_name
    mod resolve_overlay_display_name_tests {
        use super::*;

        #[test]
        fn name_override_takes_priority() {
            let config = OverlayConfig {
                overlay: state::OverlayConfigMeta {
                    name: Some("config-name".to_string()),
                    description: None,
                },
                ..Default::default()
            };
            let result = resolve_overlay_display_name(
                &config,
                Path::new("/some/dir-name"),
                Some("override".to_string()),
            );
            assert_eq!(result, "override");
        }

        #[test]
        fn config_name_used_when_no_override() {
            let config = OverlayConfig {
                overlay: state::OverlayConfigMeta {
                    name: Some("config-name".to_string()),
                    description: None,
                },
                ..Default::default()
            };
            let result = resolve_overlay_display_name(&config, Path::new("/some/dir-name"), None);
            assert_eq!(result, "config-name");
        }

        #[test]
        fn directory_name_used_as_fallback() {
            let config = OverlayConfig::default();
            let result = resolve_overlay_display_name(&config, Path::new("/some/my-overlay"), None);
            assert_eq!(result, "my-overlay");
        }

        #[test]
        fn root_path_returns_unnamed() {
            let config = OverlayConfig::default();
            let result = resolve_overlay_display_name(&config, Path::new("/"), None);
            // Path::new("/").file_name() is None
            assert_eq!(result, "unnamed");
        }
    }

    // Tests for determine_overlay_name
    mod determine_overlay_name_tests {
        use super::*;

        #[test]
        fn returns_normalized_name() {
            let config = OverlayConfig::default();
            let result =
                determine_overlay_name(&config, Path::new("/some/My Overlay"), None).unwrap();
            // normalize_overlay_name lowercases and replaces spaces
            assert!(!result.contains(' '));
        }

        #[test]
        fn name_override_is_normalized() {
            let config = OverlayConfig::default();
            let result = determine_overlay_name(
                &config,
                Path::new("/some/dir"),
                Some("My Override".to_string()),
            )
            .unwrap();
            assert!(!result.contains(' '));
        }
    }

    // Tests for collect_overlay_files
    mod collect_overlay_files_tests {
        use super::*;

        #[test]
        fn collects_regular_files() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("file1.txt"), "content1").unwrap();
            fs::write(temp.path().join("file2.txt"), "content2").unwrap();

            let config = OverlayConfig::default();
            let files = collect_overlay_files(temp.path(), &config);
            assert_eq!(files.len(), 2);
        }

        #[test]
        fn skips_config_file() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("file.txt"), "content").unwrap();
            fs::write(temp.path().join(CONFIG_FILE), "overlay config").unwrap();

            let config = OverlayConfig::default();
            let files = collect_overlay_files(temp.path(), &config);
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].1, "file.txt");
        }

        #[test]
        fn skips_git_directory() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("file.txt"), "content").unwrap();
            fs::create_dir_all(temp.path().join(".git/objects")).unwrap();
            fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();

            let config = OverlayConfig::default();
            let files = collect_overlay_files(temp.path(), &config);
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].1, "file.txt");
        }

        #[test]
        fn skips_cache_meta_file() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("file.txt"), "content").unwrap();
            fs::write(temp.path().join(".repoverlay-cache-meta.ccl"), "meta").unwrap();

            let config = OverlayConfig::default();
            let files = collect_overlay_files(temp.path(), &config);
            assert_eq!(files.len(), 1);
        }

        #[test]
        fn skips_files_in_configured_directories() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("keep.txt"), "keep").unwrap();
            fs::create_dir_all(temp.path().join(".vscode")).unwrap();
            fs::write(temp.path().join(".vscode/settings.json"), "{}").unwrap();

            let config = OverlayConfig {
                directories: vec![".vscode".to_string()],
                ..Default::default()
            };
            let files = collect_overlay_files(temp.path(), &config);
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].1, "keep.txt");
        }

        #[test]
        fn applies_mappings() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("source.txt"), "content").unwrap();

            let mut mappings = std::collections::HashMap::new();
            mappings.insert("source.txt".to_string(), vec!["target.txt".to_string()]);
            let config = OverlayConfig {
                mappings,
                ..Default::default()
            };
            let files = collect_overlay_files(temp.path(), &config);
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].0, PathBuf::from("source.txt"));
            assert_eq!(files[0].1, "target.txt");
        }

        #[test]
        fn collects_nested_files() {
            let temp = TempDir::new().unwrap();
            fs::create_dir_all(temp.path().join("dir/subdir")).unwrap();
            fs::write(temp.path().join("dir/file.txt"), "").unwrap();
            fs::write(temp.path().join("dir/subdir/nested.txt"), "").unwrap();

            let config = OverlayConfig::default();
            let files = collect_overlay_files(temp.path(), &config);
            assert_eq!(files.len(), 2);
        }

        #[test]
        fn empty_source_returns_empty() {
            let temp = TempDir::new().unwrap();
            let config = OverlayConfig::default();
            let files = collect_overlay_files(temp.path(), &config);
            assert!(files.is_empty());
        }
    }

    // Tests for load_overlay_config
    mod load_overlay_config_tests {
        use super::*;

        #[test]
        fn missing_config_returns_default() {
            let temp = TempDir::new().unwrap();
            let config = load_overlay_config(temp.path()).unwrap();
            assert!(config.overlay.name.is_none());
            assert!(config.mappings.is_empty());
            assert!(config.directories.is_empty());
        }

        #[test]
        fn reads_config_with_name() {
            let temp = TempDir::new().unwrap();
            // CCL format: section with nested key-value
            fs::write(
                temp.path().join(CONFIG_FILE),
                "overlay =\n  name = my-overlay\n",
            )
            .unwrap();

            let config = load_overlay_config(temp.path()).unwrap();
            assert_eq!(config.overlay.name.as_deref(), Some("my-overlay"));
        }

        #[test]
        fn empty_config_returns_default() {
            let temp = TempDir::new().unwrap();
            // An empty file is valid CCL - all fields have defaults
            fs::write(temp.path().join(CONFIG_FILE), "").unwrap();

            let config = load_overlay_config(temp.path()).unwrap();
            assert!(config.overlay.name.is_none());
        }
    }

    // Tests for OverlayName via the public type
    mod overlay_name_integration_tests {
        use super::*;

        #[test]
        fn as_ref_returns_inner_str() {
            let name = OverlayName::new("test-overlay");
            let s: &str = name.as_ref();
            assert_eq!(s, "test-overlay");
        }

        #[test]
        fn clone_produces_equal_value() {
            let name = OverlayName::new("overlay");
            let cloned = name.clone();
            assert_eq!(name, cloned);
        }

        #[test]
        fn ordering_is_alphabetical() {
            let a = OverlayName::new("alpha");
            let b = OverlayName::new("beta");
            let c = OverlayName::new("alpha");
            assert!(a < b);
            assert_eq!(a.cmp(&c), std::cmp::Ordering::Equal);
        }

        #[test]
        fn hash_consistent_with_equality() {
            use std::collections::HashSet;
            let mut set = HashSet::new();
            set.insert(OverlayName::new("foo"));
            set.insert(OverlayName::new("foo"));
            set.insert(OverlayName::new("bar"));
            assert_eq!(set.len(), 2);
        }

        #[test]
        fn debug_format() {
            let name = OverlayName::new("test");
            let debug = format!("{name:?}");
            assert!(debug.contains("test"));
        }

        #[test]
        fn partial_eq_str_ref() {
            let name = OverlayName::new("my-overlay");
            let s: &str = "my-overlay";
            assert!(name == s);
            assert!(name == "my-overlay");
        }
    }

    // Tests for ResolvedSource and ResolvedSources
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

    // Tests for check_files_against_existing
    mod check_files_against_existing_tests {
        use super::*;
        use std::collections::HashMap;

        #[test]
        fn no_conflicts_succeeds() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("new-file.txt"), "content").unwrap();

            let config = OverlayConfig::default();
            let existing: HashMap<String, String> = HashMap::new();

            let result = check_files_against_existing(temp.path(), &config, &existing);
            assert!(result.is_ok());
        }

        #[test]
        fn file_conflict_fails() {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("conflicting.txt"), "content").unwrap();

            let config = OverlayConfig::default();
            let mut existing = HashMap::new();
            existing.insert("conflicting.txt".to_string(), "other-overlay".to_string());

            let result = check_files_against_existing(temp.path(), &config, &existing);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("conflicting.txt"));
            assert!(err.contains("other-overlay"));
        }

        #[test]
        fn directory_conflict_fails() {
            let temp = TempDir::new().unwrap();

            let config = OverlayConfig {
                directories: vec![".vscode".to_string()],
                ..Default::default()
            };
            let mut existing = HashMap::new();
            existing.insert(".vscode".to_string(), "existing-overlay".to_string());

            let result = check_files_against_existing(temp.path(), &config, &existing);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains(".vscode"));
        }
    }

    // Tests for resolve_local_path with prefix requirement
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

    // Tests for apply_overlay with different conflict strategies
    mod apply_overlay_conflict_tests {
        use super::*;

        #[test]
        fn apply_same_name_twice_fails_by_default() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("dup-test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let result = apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("dup-test".to_string()),
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
        fn apply_same_name_with_force_succeeds() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content1").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("force-test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            // Update the file
            fs::write(overlay.path().join("file.txt"), "content2").unwrap();

            // Re-apply with force
            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("force-test".to_string()),
                None,
                false,
                ConflictStrategy::Force,
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let content = fs::read_to_string(canonical.join("file.txt")).unwrap();
            assert_eq!(content, "content2");
        }

        #[test]
        fn dry_run_does_not_apply() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("dry-run-test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                true, // dry_run
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            assert!(!canonical.join("file.txt").exists());
        }
    }

    // Tests for apply_overlay with config file
    mod apply_overlay_config_tests {
        use super::*;

        #[test]
        fn overlay_uses_config_name() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();
            fs::write(
                overlay.path().join(CONFIG_FILE),
                "overlay =\n  name = config-name\n",
            )
            .unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                None, // no name override
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let state_file = canonical
                .join(STATE_DIR)
                .join(OVERLAYS_DIR)
                .join("config-name.ccl");
            assert!(state_file.exists());
        }

        #[test]
        fn name_override_beats_config() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();
            fs::write(
                overlay.path().join(CONFIG_FILE),
                "overlay =\n  name = config-name\n",
            )
            .unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("override-name".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let state_file = canonical
                .join(STATE_DIR)
                .join(OVERLAYS_DIR)
                .join("override-name.ccl");
            assert!(state_file.exists());
        }

        #[test]
        fn overlay_with_mappings() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join(".envrc"), "content").unwrap();
            fs::write(
                overlay.path().join(CONFIG_FILE),
                "mappings =\n  .envrc = .env\n",
            )
            .unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("mapped".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            assert!(canonical.join(".env").exists());
            assert!(!canonical.join(".envrc").exists());
        }

        #[test]
        fn overlay_with_directories() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::create_dir_all(overlay.path().join(".vscode")).unwrap();
            fs::write(
                overlay.path().join(".vscode/settings.json"),
                r#"{"editor.tabSize": 2}"#,
            )
            .unwrap();
            fs::write(
                overlay.path().join(CONFIG_FILE),
                "directories =\n  = .vscode\n",
            )
            .unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("dir-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            assert!(canonical.join(".vscode").exists());
            assert!(canonical.join(".vscode/settings.json").exists());
        }
    }
}
