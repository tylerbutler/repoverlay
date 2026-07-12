use anyhow::{Context, Result, bail};
use chrono::Utc;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::profile::{
    ProfileFileEntry, ProfileMode, ProfileState, SkippedCapability, save_profile_state,
};
use crate::profile_applicators::{AgentHarness, ProfileContext};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProfilePlan {
    pub(crate) profile_name: String,
    pub(crate) harness: AgentHarness,
    pub(crate) actions: Vec<ProfileAction>,
    /// Provenance for each managed (resolved-and-cached) plugin, recorded into
    /// `ProfileState` so removal/`show`/`update` can reason about what was placed.
    pub(crate) plugins: Vec<PluginProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginProvenance {
    /// Human-readable reference (`marketplace/name` or local path) for display.
    pub(crate) reference: String,
    /// Resolved git commit of the bundle, when backed by a git repo.
    pub(crate) resolved_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProfileAction {
    ApplyOverlay {
        reference: String,
    },
    WriteFile {
        source: PathBuf,
        target: PathBuf,
    },
    MergeJson {
        target: PathBuf,
        value: Value,
        /// RFC 6901 JSON-pointer paths this merge owns (e.g. `/servers/rust`).
        /// Ownership is tracked per-pointer for conflict detection and clean
        /// removal; each owned pointer's value is replaced wholesale.
        owned_paths: Vec<String>,
    },
    SkipCapability {
        capability: String,
        reason: String,
    },
    /// Insert or replace a profile-owned managed region inside a shared markdown
    /// file (e.g. `<repo>/AGENTS.md`). All of a profile's instruction bodies are
    /// concatenated into a single marker-delimited block keyed by `marker_id`,
    /// coexisting with user content and other profiles' regions.
    WriteManagedRegion {
        bodies: Vec<InstructionBody>,
        target: PathBuf,
        marker_id: String,
    },
    /// Place a decomposed plugin part (a skill directory or an agent file) into
    /// a native harness location by copying the tree/file.
    ///
    /// Unlike [`ProfileAction::WriteFile`], the `source` lives in the resolved
    /// plugin bundle (the cache), not the profile asset directory. A
    /// pre-existing target is backed up on apply and restored on removal.
    PlacePluginDir {
        source: PathBuf,
        target: PathBuf,
    },
}

/// One piece of instruction content destined for a profile's managed region.
///
/// A `File` carries its own `base_dir` (the directory of the config file that
/// defined the entry), which is the containment root the preflight check
/// validates the resolved path against. `Inline` content has no file and is
/// used verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InstructionBody {
    File { path: PathBuf, base_dir: PathBuf },
    Inline(String),
}

pub(crate) fn apply_profile(
    name: &str,
    harness: AgentHarness,
    target: &Path,
    mode: ProfileMode,
    session_id: Option<String>,
) -> Result<ProfileState> {
    apply_profile_with_harness_home(
        name,
        harness,
        target,
        mode,
        session_id,
        harness.home_from_env()?,
    )
}

fn apply_profile_with_harness_home(
    name: &str,
    harness: AgentHarness,
    target: &Path,
    mode: ProfileMode,
    session_id: Option<String>,
    harness_home: PathBuf,
) -> Result<ProfileState> {
    crate::profile::validate_profile_state_component(name)?;
    let state_path = crate::profile::profile_state_path(target, name, harness)?;
    match state_path.try_exists() {
        Ok(true) => bail!(
            "Profile '{name}' is already applied for {harness}; remove it before applying again"
        ),
        Ok(false) => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!("Failed to inspect profile state: {}", state_path.display())
            });
        }
    }

    let config = crate::config::load_config(Some(target))?;
    let profile = config
        .profiles
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Profile '{name}' not found"))?;
    let applicator = harness.applicator();
    let context = ProfileContext {
        profile_name: name.to_string(),
        harness,
        target: target.to_path_buf(),
        profile_asset_dir: target.to_path_buf(),
        harness_home,
        mode,
        session_id: session_id.clone(),
        marketplaces: config.marketplaces.clone(),
        cache: crate::cache::CacheManager::new()?,
    };
    let plan = applicator.plan(profile, &context)?;
    preflight_plan(&plan, &context.profile_asset_dir)?;
    check_mcp_ownership_conflicts(&plan, target, harness)?;
    let mut state = ProfileState {
        name: name.to_string(),
        harness,
        mode,
        session_id,
        applied_at: Utc::now(),
        profile_fingerprint: format!("sickle-hash:{}", simple_profile_fingerprint(profile)),
        overlays: Vec::new(),
        files: Vec::new(),
        skipped: Vec::new(),
        plugins: plan
            .plugins
            .iter()
            .map(|p| crate::profile::ProfilePluginEntry {
                reference: p.reference.clone(),
                resolved_commit: p.resolved_commit.clone(),
            })
            .collect(),
    };

    // Keep a copy of the resolved actions so a successful apply can write a
    // self-contained external snapshot for `restore` after `git clean`.
    let snapshot_actions = plan.actions.clone();

    let apply_result = (|| -> Result<()> {
        for (action_index, action) in plan.actions.into_iter().enumerate() {
            execute_profile_action(
                action,
                action_index,
                name,
                harness,
                target,
                &context.profile_asset_dir,
                &mut state,
            )?;
        }

        save_profile_state(target, &state)
    })();

    if let Err(err) = apply_result {
        // A partial apply may have mutated repo-local files (write-file,
        // merge-json, managed-region) and placed plugin directories without
        // persisting a state file. Reverse every recorded mutation using the same
        // restore routines as removal so a failed apply leaves no orphaned changes
        // (which a later retry would otherwise mis-record as pre-existing data).
        restore_profile_files(name, harness, target, &state.files).with_context(|| {
            format!("Profile apply failed ({err}); failed to roll back profile file changes")
        })?;
        rollback_applied_overlays(target, &state.overlays).with_context(|| {
            format!("Profile apply failed ({err}); failed to roll back applied overlays")
        })?;
        return Err(err);
    }

    // Exclude the profile's repo-local files from git so they don't show up as
    // untracked. Best-effort: a failure here only means the files may appear in
    // `git status`, so we warn rather than tearing down a successful apply.
    let exclude_entries = profile_exclude_entries(target, &state);
    if !exclude_entries.is_empty() {
        let section = profile_exclude_section(name, harness);
        if let Err(err) = crate::update_git_exclude(target, &section, &exclude_entries, true) {
            eprintln!(
                "Warning: could not update git exclude (profile files may show as untracked): {err}"
            );
        }
    }

    // Mirror the in-repo state to an external snapshot so `restore` can rebuild
    // the profile after `git clean -fdx` wipes `.repoverlay/`. Best-effort: a
    // failure here only costs restore-after-clean, not the apply itself.
    if let Err(err) = save_external_profile_snapshot(
        target,
        &state,
        &context.profile_asset_dir,
        &snapshot_actions,
    ) {
        eprintln!(
            "Warning: could not write external profile snapshot \
             (restore after `git clean` may be unavailable): {err}"
        );
    }

    println!("Applied profile {name} for {harness}");
    Ok(state)
}

/// Execute a single resolved profile action, mutating `state` to record the
/// placement so it can later be removed or rolled back.
///
/// Shared by `profile apply` (where `asset_dir` is the repo/profile asset
/// directory) and `restore` (where `asset_dir` is the external snapshot's blob
/// directory). The `ApplyOverlay` arm is only reached during apply; restore
/// reconstructs overlay names from the snapshot and relies on overlay restore.
fn execute_profile_action(
    action: ProfileAction,
    action_index: usize,
    name: &str,
    harness: AgentHarness,
    target: &Path,
    asset_dir: &Path,
    state: &mut ProfileState,
) -> Result<()> {
    match action {
        ProfileAction::ApplyOverlay { reference } => {
            let before = crate::state::list_applied_overlays(target)?;
            crate::apply_overlay(
                &reference,
                target,
                &crate::ApplyOptions {
                    update_cache: true,
                    ..crate::ApplyOptions::default()
                },
            )?;
            let after = crate::state::list_applied_overlays(target)?;
            let new_overlays: Vec<_> = after
                .into_iter()
                .filter(|overlay| !before.contains(overlay))
                .collect();
            if new_overlays.is_empty() {
                bail!(
                    "Profile overlay '{reference}' did not create overlay state; \
                     cannot record removable overlay name"
                );
            }
            state
                .overlays
                .extend(new_overlays.into_iter().map(|overlay| overlay.to_string()));
        }
        ProfileAction::WriteFile {
            source,
            target: file_target,
        } => {
            let (backup, existed) =
                capture_write_file_backup(name, harness, target, &file_target, action_index)?;
            copy_profile_file(&source, &file_target, asset_dir)?;
            state.files.push(ProfileFileEntry {
                source,
                target: file_target,
                action: "write-file".to_string(),
                backup,
                existed,
            });
        }
        ProfileAction::MergeJson {
            target: json_target,
            value,
            owned_paths,
        } => {
            reject_symlink_profile_target(&json_target)?;
            let backup_source = capture_merge_json_backup(
                name,
                harness,
                target,
                &json_target,
                action_index,
                &value,
                &owned_paths,
            )?;
            merge_json_value(&json_target, &value, &owned_paths)?;
            state.files.push(ProfileFileEntry {
                source: backup_source,
                target: json_target,
                action: "merge-json".to_string(),
                backup: None,
                existed: false,
            });
        }
        ProfileAction::SkipCapability { capability, reason } => {
            eprintln!("Warning: skipped {capability}: {reason}");
            state.skipped.push(SkippedCapability { capability, reason });
        }
        ProfileAction::WriteManagedRegion {
            bodies: instruction_bodies,
            target: region_target,
            marker_id,
        } => {
            crate::profile::validate_profile_marker_component(&marker_id)?;
            reject_symlink_profile_target(&region_target)?;
            let mut bodies = Vec::with_capacity(instruction_bodies.len());
            for instruction in &instruction_bodies {
                let content = match instruction {
                    InstructionBody::File { path, .. } => {
                        fs::read_to_string(path).with_context(|| {
                            format!("Failed to read instruction source {}", path.display())
                        })?
                    }
                    InstructionBody::Inline(text) => text.clone(),
                };
                bodies.push(content.trim_end_matches('\n').to_string());
            }
            let body = bodies.join("\n\n");
            if body.lines().any(is_reserved_marker_line) {
                bail!(
                    "Instruction content for profile '{marker_id}' contains a reserved \
                     repoverlay managed-region marker line; remove it before applying"
                );
            }
            // Removal strips this profile's block from the live file rather
            // than restoring a snapshot, so other profiles' regions and user
            // edits survive; we only record whether we created the file, to
            // decide deletion on empty during removal.
            let (current, existed) = match fs::read_to_string(&region_target) {
                Ok(content) => (content, true),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => (String::new(), false),
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!(
                            "Failed to read managed-region target {}",
                            region_target.display()
                        )
                    });
                }
            };
            let updated = upsert_managed_region(&current, &marker_id, &body);
            if let Some(parent) = region_target.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create parent directory for {}",
                        region_target.display()
                    )
                })?;
            }
            crate::fs_util::atomic_write(&region_target, &updated)?;
            state.files.push(ProfileFileEntry {
                source: region_target.clone(),
                target: region_target,
                action: "managed-region".to_string(),
                backup: None,
                existed,
            });
        }
        ProfileAction::PlacePluginDir {
            source,
            target: dir_target,
        } => {
            let (backup, existed) =
                place_plugin_dir(name, harness, target, &source, &dir_target, action_index)?;
            state.files.push(ProfileFileEntry {
                source,
                target: dir_target,
                action: "place-plugin-dir".to_string(),
                backup,
                existed,
            });
        }
    }
    Ok(())
}

/// Section name used in `.git/info/exclude` for a profile's repo-local files.
///
/// Namespaced as `profile:<name>@<harness>` so it cannot collide with an overlay
/// section (which uses the bare overlay name) and so the same profile applied for
/// two harnesses gets two independent sections.
pub(crate) fn profile_exclude_section(name: &str, harness: AgentHarness) -> String {
    format!("profile:{name}@{harness}")
}

/// Compute the repo-relative `.git/info/exclude` entries for a profile's
/// in-repo file placements.
///
/// Entries are derived from `state.files` (the files this profile wrote into the
/// repo). Targets outside `target` are skipped, directories get a trailing `/`,
/// and duplicates are collapsed.
pub(crate) fn profile_exclude_entries(target: &Path, state: &ProfileState) -> Vec<String> {
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for file in &state.files {
        let Ok(rel) = file.target.strip_prefix(target) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let mut entry = rel.to_string_lossy().replace('\\', "/");
        if file.target.is_dir() && !entry.ends_with('/') {
            entry.push('/');
        }
        if seen.insert(entry.clone()) {
            entries.push(entry);
        }
    }
    entries
}

/// Re-resolve managed plugins for every persistently-applied profile and
/// re-apply any profile whose managed plugin sources changed.
///
/// Returns the number of applied (persistent) profiles inspected, so the
/// `update` command can distinguish "nothing is applied" from "profiles were
/// checked and are up to date".
///
/// Each profile is processed independently: managed plugins are re-resolved
/// with `update = true` (refreshing the local cache to the latest source), and
/// only when every managed plugin for a profile resolves cleanly *and* at least
/// one resolved commit differs from the recorded one is the profile re-applied
/// (`remove` then `apply`). Resolution failures (for example a marketplace that
/// is no longer registered) are reported and that profile is left untouched, so
/// `update` never tears down a profile it cannot reconstruct.
pub(crate) fn update_profile_plugins(target: &Path, dry_run: bool) -> Result<usize> {
    use crate::plugin::{InstallMode, PluginRef, ResolvedPlugin, resolve_plugin};

    // Match the canonical target used by `profile apply`/`remove`, so the
    // managed-skills-root placement guard sees the same path it recorded.
    let target = crate::resolve::canonicalize_path(target, "Target directory")?;
    let target = target.as_path();

    let states = crate::profile::list_applied_profile_states(target)?;
    let persistent: Vec<_> = states
        .into_iter()
        .filter(|s| s.mode == ProfileMode::Persistent)
        .collect();
    if persistent.is_empty() {
        return Ok(0);
    }

    let config = crate::config::load_config(Some(target))?;
    let cache = crate::cache::CacheManager::new()?;
    let inspected = persistent.len();

    for state in persistent {
        let Some(profile) = config.profiles.get(&state.name) else {
            println!(
                "  ? profile '{}' ({}) is no longer in config; skipping plugin update",
                state.name, state.harness
            );
            continue;
        };

        let mut changed = false;
        let mut failed = false;
        for plugin in &profile.plugins {
            // Delegate plugins place nothing on disk, so there is nothing to
            // re-place; their enablement is recorded in harness settings.
            if matches!(
                plugin,
                PluginRef::Marketplace {
                    install: InstallMode::Delegate,
                    ..
                }
            ) {
                continue;
            }

            let reference = plugin.to_string();
            match resolve_plugin(plugin, &config.marketplaces, &cache, target, true) {
                Ok(resolved) => {
                    let new_commit = match resolved {
                        ResolvedPlugin::Bundle {
                            resolved_commit, ..
                        } => resolved_commit,
                        ResolvedPlugin::Delegate { .. } => None,
                    };
                    let recorded = state
                        .plugins
                        .iter()
                        .find(|p| p.reference == reference)
                        .and_then(|p| p.resolved_commit.clone());
                    if new_commit.is_some() && new_commit != recorded {
                        changed = true;
                        println!(
                            "  ↑ {} ({}) plugin {reference} changed",
                            state.name, state.harness
                        );
                    }
                }
                Err(err) => {
                    failed = true;
                    eprintln!(
                        "  ! {} ({}) plugin {reference} could not be re-resolved: {err}",
                        state.name, state.harness
                    );
                }
            }
        }

        if failed {
            eprintln!(
                "  ! skipping re-apply of '{}' ({}) due to resolution errors",
                state.name, state.harness
            );
            continue;
        }
        if !changed {
            println!(
                "  {} ({}) plugins are up to date",
                state.name, state.harness
            );
            continue;
        }
        if dry_run {
            println!(
                "  (dry run) '{}' ({}) would be re-applied",
                state.name, state.harness
            );
            continue;
        }

        remove_profile(&state.name, state.harness, target).with_context(|| {
            format!(
                "Failed to remove '{}' ({}) before re-applying updated plugins",
                state.name, state.harness
            )
        })?;
        apply_profile(
            &state.name,
            state.harness,
            target,
            ProfileMode::Persistent,
            None,
        )
        .with_context(|| {
            format!(
                "'{}' ({}) was removed but could not be re-applied with updated plugins; \
                 re-apply it manually with `repoverlay profile apply`",
                state.name, state.harness
            )
        })?;
        println!("  ✓ re-applied '{}' ({})", state.name, state.harness);
    }

    Ok(inspected)
}

pub(crate) fn remove_profile(name: &str, harness: AgentHarness, target: &Path) -> Result<()> {
    crate::profile::validate_profile_state_component(name)?;

    let lock_path = crate::profile::profile_lock_path(target, name, harness)?;
    match crate::profile::inspect_lock(&lock_path)? {
        crate::profile::LockState::Live => {
            bail!(
                "Profile '{name}' is currently in use by an ephemeral '{harness}' session; \
                 stop that session before removing the profile"
            );
        }
        crate::profile::LockState::Stale => {
            // The owning session died without releasing the lock (e.g. SIGKILL or
            // power loss). Recover it so removal can proceed.
            std::fs::remove_file(&lock_path).with_context(|| {
                format!(
                    "Failed to remove stale profile lock: {}",
                    lock_path.display()
                )
            })?;
        }
        crate::profile::LockState::Absent => {}
    }

    remove_profile_inner(name, harness, target)
}

/// Remove a profile on behalf of the ephemeral session that owns its lock.
///
/// This bypasses the profile lock check enforced by [`remove_profile`] so the
/// owning `repoverlay copilot --profile` session can clean itself up while still
/// holding its lock. Callers must only use this for the session that created the
/// lock.
pub(crate) fn remove_profile_for_session(
    name: &str,
    harness: AgentHarness,
    target: &Path,
) -> Result<()> {
    crate::profile::validate_profile_state_component(name)?;
    remove_profile_inner(name, harness, target)
}

/// Reverse the per-file mutations recorded in `files`, in reverse application
/// order, dispatching to the same restore routines used during removal.
///
/// Shared by [`remove_profile_inner`] and the apply failure path so that a
/// partial apply is undone exactly as a full removal would be: write-file and
/// merge-json mutations are restored from their backups, managed regions are
/// stripped, and placed plugin directories are removed (restoring any displaced
/// content).
fn restore_profile_files(
    name: &str,
    harness: AgentHarness,
    target: &Path,
    files: &[ProfileFileEntry],
) -> Result<()> {
    for file in files.iter().rev() {
        match file.action.as_str() {
            "write-file" => restore_write_file_backup(name, harness, target, file)?,
            "merge-json" => restore_merge_json_backup(name, harness, target, file)?,
            "managed-region" => restore_managed_region(name, harness, target, file)?,
            "place-plugin-dir" => restore_placed_plugin_dir(name, harness, target, file)?,
            _ => {}
        }
    }
    Ok(())
}

fn remove_profile_inner(name: &str, harness: AgentHarness, target: &Path) -> Result<()> {
    let state = crate::profile::load_profile_state(target, name, harness)?;
    restore_profile_files(name, harness, target, &state.files)?;
    for overlay in &state.overlays {
        let overlay_state_path = target
            .join(crate::state::STATE_DIR)
            .join(crate::state::OVERLAYS_DIR)
            .join(format!("{overlay}.ccl"));
        match overlay_state_path.try_exists() {
            Ok(false) => (),
            Ok(true) => crate::remove_overlay(target, Some(overlay.clone()), false, false)?,
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "Failed to inspect recorded overlay state: {}",
                        overlay_state_path.display()
                    )
                });
            }
        }
    }
    crate::profile::remove_profile_state(target, name, harness)?;
    if let Err(err) = remove_external_profile_snapshot(target, name, harness) {
        eprintln!("Warning: could not update external profile snapshot during removal: {err}");
    }
    let section = profile_exclude_section(name, harness);
    if let Err(err) = crate::update_git_exclude(target, &section, &[], false) {
        eprintln!("Warning: could not update git exclude while removing profile: {err}");
    }
    println!("Removed profile {name} for {harness}");
    Ok(())
}

pub(crate) fn list_profile_states(target: &Path) -> Result<Vec<ProfileState>> {
    let dir = target.join(crate::state::STATE_DIR).join("profiles");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut states = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("ccl") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                eprintln!(
                    "Warning: failed to load profile state {}: {err}",
                    path.display()
                );
                continue;
            }
        };
        match sickle::from_str(&content) {
            Ok(state) => states.push(state),
            Err(err) => {
                eprintln!(
                    "Warning: failed to load profile state {}: {err}",
                    path.display()
                );
            }
        }
    }
    states.sort_by(|left: &ProfileState, right: &ProfileState| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.harness.as_str().cmp(right.harness.as_str()))
    });
    Ok(states)
}

fn ensure_removable_profile_file(
    _name: &str,
    _harness: AgentHarness,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    // Every harness places removable profile files under the repo root.
    let allowed_root = repo_target.to_path_buf();

    if !file.target.starts_with(&allowed_root) {
        bail!(
            "Refusing to remove profile file outside managed location: {}",
            file.target.display()
        );
    }

    let allowed_root = fs::canonicalize(&allowed_root).with_context(|| {
        format!(
            "Failed to inspect profile removal root: {}",
            allowed_root.display()
        )
    })?;
    let target_parent = file
        .target
        .parent()
        .context("Profile removal target has no parent directory")?;
    let target_parent = fs::canonicalize(target_parent).with_context(|| {
        format!(
            "Failed to inspect profile removal target: {}",
            file.target.display()
        )
    })?;
    if !target_parent.starts_with(&allowed_root) {
        bail!(
            "Refusing to remove profile file outside managed location: {}",
            file.target.display()
        );
    }

    Ok(())
}

fn preflight_plan(plan: &ProfilePlan, profile_asset_dir: &Path) -> Result<()> {
    let mut seen_targets = BTreeSet::new();
    for action in &plan.actions {
        match action {
            ProfileAction::WriteFile { source, target, .. } => {
                ensure_regular_profile_source(source, profile_asset_dir)?;
                if target.as_os_str().is_empty() {
                    bail!("Profile target path must not be empty");
                }
                reject_duplicate_profile_target(&mut seen_targets, target)?;
            }
            ProfileAction::MergeJson { target, .. } => {
                if target.as_os_str().is_empty() {
                    bail!("Profile JSON target path must not be empty");
                }
                reject_symlink_profile_target(target)?;
                reject_duplicate_profile_target(&mut seen_targets, target)?;
            }
            ProfileAction::WriteManagedRegion {
                bodies,
                target,
                marker_id,
            } => {
                crate::profile::validate_profile_marker_component(marker_id)?;
                if target.as_os_str().is_empty() {
                    bail!("Profile managed-region target path must not be empty");
                }
                if bodies.is_empty() {
                    bail!("Profile managed-region must have at least one instruction");
                }
                for body in bodies {
                    if let InstructionBody::File { path, base_dir } = body {
                        ensure_regular_profile_source(path, base_dir)?;
                    }
                }
                reject_duplicate_profile_target(&mut seen_targets, target)?;
            }
            ProfileAction::ApplyOverlay { reference } => {
                if reference.trim().is_empty() {
                    bail!("Profile overlay reference must not be empty");
                }
            }
            ProfileAction::PlacePluginDir { source, target, .. } => {
                if target.as_os_str().is_empty() {
                    bail!("Profile plugin directory target must not be empty");
                }
                // The placed target must be exactly `<managed-root>/<name>` with a
                // single, traversal-free final component; everything else is rejected
                // before any filesystem mutation runs.
                validate_plugin_dir_target(target)?;
                if !source.exists() {
                    bail!("Plugin part source does not exist: {}", source.display());
                }
                reject_duplicate_profile_target(&mut seen_targets, target)?;
            }
            ProfileAction::SkipCapability { .. } => {}
        }
    }
    Ok(())
}

fn reject_duplicate_profile_target(
    seen_targets: &mut BTreeSet<PathBuf>,
    target: &Path,
) -> Result<()> {
    if !seen_targets.insert(target.to_path_buf()) {
        bail!("duplicate profile target: {}", target.display());
    }
    Ok(())
}

/// Read the JSON-pointer paths an already-applied profile manages for a target.
///
/// Returns `None` if the backup metadata cannot be read or parsed, in which case
/// callers should fall back to conservative same-target locking.
fn owned_json_pointers(file: &ProfileFileEntry) -> Option<BTreeSet<String>> {
    Some(owned_json_backup(file)?.paths.keys().cloned().collect())
}

/// Read the full pointer-keyed merge backup for an applied `merge-json` file.
fn owned_json_backup(file: &ProfileFileEntry) -> Option<MergeJsonBackup> {
    let content = fs::read_to_string(&file.source).ok()?;
    serde_json::from_str(&content).ok()
}

/// Whether `pointer` addresses an `extraKnownMarketplaces` registration entry,
/// which is shared (reference-counted) across profiles rather than exclusively
/// owned by one.
fn is_shared_marketplace_pointer(pointer: &str) -> bool {
    pointer.starts_with("/extraKnownMarketplaces/")
}

/// Gather the JSON pointers still owned by other applied profiles for the same
/// harness and JSON target. Removing the profile named `name` must not delete
/// these (they remain in use by another profile).
fn protected_json_pointers(
    name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    json_target: &Path,
) -> BTreeSet<String> {
    let mut protected = BTreeSet::new();
    let Ok(states) = list_profile_states(repo_target) else {
        return protected;
    };
    for state in states {
        if state.name == name || state.harness != harness {
            continue;
        }
        for file in &state.files {
            if file.action != "merge-json" || file.target != *json_target {
                continue;
            }
            if let Some(owned) = owned_json_pointers(file) {
                protected.extend(owned);
            }
        }
    }
    protected
}

/// Reject applying a profile whose planned JSON merge would manage JSON-pointer
/// paths already owned by another applied profile for the same harness.
///
/// Precise pointer tracking is used when the other profile's backup metadata is
/// readable; otherwise we conservatively reject any other profile that already
/// manages the same JSON target.
fn check_mcp_ownership_conflicts(
    plan: &ProfilePlan,
    target: &Path,
    harness: AgentHarness,
) -> Result<()> {
    let existing = list_profile_states(target)?;
    for action in &plan.actions {
        let ProfileAction::MergeJson {
            target: json_target,
            value: planned_value,
            owned_paths,
            ..
        } = action
        else {
            continue;
        };
        let planned_keys: BTreeSet<String> = owned_paths.iter().cloned().collect();
        for state in &existing {
            if state.harness != harness {
                continue;
            }
            for file in &state.files {
                if file.action != "merge-json" || file.target != *json_target {
                    continue;
                }
                match owned_json_backup(file) {
                    Some(backup) => {
                        let owned: BTreeSet<String> = backup.paths.keys().cloned().collect();
                        // `extraKnownMarketplaces` entries are shareable across
                        // profiles (reference-counted) — but only when both
                        // profiles register the SAME source. A same-named
                        // marketplace pointing at a different value is a genuine
                        // conflict.
                        let mut overlap: Vec<&String> = planned_keys
                            .intersection(&owned)
                            .filter(|key| {
                                if is_shared_marketplace_pointer(key) {
                                    planned_value.pointer(key)
                                        != backup.paths.get(*key).map(|e| &e.written)
                                } else {
                                    true
                                }
                            })
                            .collect();
                        overlap.sort();
                        if !overlap.is_empty() {
                            let keys = overlap
                                .iter()
                                .map(|key| format!("'{key}'"))
                                .collect::<Vec<_>>()
                                .join(", ");
                            bail!(
                                "JSON path(s) {keys} in {} are already managed by applied \
                                 profile '{}' for {harness}; remove that profile first",
                                json_target.display(),
                                state.name
                            );
                        }
                    }
                    None => {
                        bail!(
                            "{} is already managed by applied profile '{}' for {harness} and its \
                             ownership metadata could not be read; remove that profile first",
                            json_target.display(),
                            state.name
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn rollback_applied_overlays(repo_target: &Path, overlays: &[String]) -> Result<()> {
    for overlay in overlays.iter().rev() {
        crate::remove_overlay(repo_target, Some(overlay.clone()), false, false)
            .with_context(|| format!("Failed to remove profile overlay '{overlay}'"))?;
    }
    Ok(())
}

fn profile_backup_dir(repo_target: &Path, name: &str, harness: AgentHarness) -> Result<PathBuf> {
    crate::profile::validate_profile_state_component(name)?;

    Ok(repo_target
        .join(crate::state::STATE_DIR)
        .join("profiles")
        .join("backups")
        .join(format!("{name}.{harness}")))
}

fn merge_json_backup_path(
    repo_target: &Path,
    name: &str,
    harness: AgentHarness,
    action_index: usize,
) -> Result<PathBuf> {
    Ok(profile_backup_dir(repo_target, name, harness)?
        .join(format!("merge-json-{action_index}.bak")))
}

fn write_file_backup_path(
    repo_target: &Path,
    name: &str,
    harness: AgentHarness,
    action_index: usize,
) -> Result<PathBuf> {
    Ok(profile_backup_dir(repo_target, name, harness)?
        .join(format!("write-file-{action_index}.bak")))
}

fn capture_write_file_backup(
    name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    write_target: &Path,
    action_index: usize,
) -> Result<(Option<PathBuf>, bool)> {
    match fs::read(write_target) {
        Ok(bytes) => {
            let backup_path = write_file_backup_path(repo_target, name, harness, action_index)?;
            if let Some(parent) = backup_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create profile backup directory: {}",
                        parent.display()
                    )
                })?;
            }
            fs::write(&backup_path, bytes).with_context(|| {
                format!(
                    "Failed to back up profile file target {} to {}",
                    write_target.display(),
                    backup_path.display()
                )
            })?;
            Ok((Some(backup_path), true))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok((None, false)),
        Err(err) => Err(err).with_context(|| {
            format!(
                "Failed to capture existing profile file before modifying {}",
                write_target.display()
            )
        }),
    }
}

fn capture_merge_json_backup(
    name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    merge_target: &Path,
    action_index: usize,
    value: &Value,
    owned_paths: &[String],
) -> Result<PathBuf> {
    let backup_path = merge_json_backup_path(repo_target, name, harness, action_index)?;
    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create profile backup directory: {}",
                parent.display()
            )
        })?;
    }

    let existing = match fs::read_to_string(merge_target) {
        Ok(content) => Some(serde_json::from_str::<Value>(&content).with_context(|| {
            format!(
                "Failed to parse existing profile JSON before modifying {}",
                merge_target.display()
            )
        })?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "Failed to capture existing profile JSON before modifying {}",
                    merge_target.display()
                )
            });
        }
    };
    let backup = MergeJsonBackup::from_existing(value, existing.as_ref(), owned_paths);
    fs::write(&backup_path, serde_json::to_vec_pretty(&backup)?).with_context(|| {
        format!(
            "Failed to back up profile JSON target {} to {}",
            merge_target.display(),
            backup_path.display()
        )
    })?;
    Ok(backup_path)
}

/// Backup metadata for a `merge-json` action, keyed by RFC 6901 JSON pointer.
///
/// For each owned pointer we record whether a prior value existed (and what it
/// was) and the value we wrote, so removal can restore the prior value, remove a
/// key we added (only if it still equals what we wrote), or leave a value the
/// user has since changed.
#[derive(Debug, Deserialize, Serialize)]
struct MergeJsonBackup {
    paths: BTreeMap<String, MergeJsonPathBackup>,
}

#[derive(Debug, Deserialize, Serialize)]
struct MergeJsonPathBackup {
    existed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    prior: Option<Value>,
    written: Value,
}

impl MergeJsonBackup {
    fn from_existing(
        applied_value: &Value,
        existing: Option<&Value>,
        owned_paths: &[String],
    ) -> Self {
        let mut paths = BTreeMap::new();
        for pointer in owned_paths {
            let written = applied_value
                .pointer(pointer)
                .cloned()
                .unwrap_or(Value::Null);
            let prior = existing.and_then(|value| value.pointer(pointer));
            paths.insert(
                pointer.clone(),
                MergeJsonPathBackup {
                    existed: prior.is_some(),
                    prior: prior.cloned(),
                    written,
                },
            );
        }
        Self { paths }
    }
}

fn restore_merge_json_backup(
    name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    ensure_removable_profile_json(harness, repo_target, file)?;

    ensure_valid_profile_backup_source(name, harness, repo_target, &file.source)?;
    let backup_content = fs::read_to_string(&file.source).with_context(|| {
        format!(
            "Failed to read profile JSON backup {}",
            file.source.display()
        )
    })?;
    let backup: MergeJsonBackup = serde_json::from_str(&backup_content).with_context(|| {
        format!(
            "Failed to parse profile JSON backup {}",
            file.source.display()
        )
    })?;
    let current_content = fs::read_to_string(&file.target).with_context(|| {
        format!(
            "Profile JSON target is missing during removal: {}",
            file.target.display()
        )
    })?;
    let mut current: Value = serde_json::from_str(&current_content).with_context(|| {
        format!(
            "Failed to parse current profile JSON target during removal: {}",
            file.target.display()
        )
    })?;
    let protected = protected_json_pointers(name, harness, repo_target, &file.target);
    restore_owned_json_paths(&mut current, &backup, &file.target, &protected);
    prune_empty_objects(&mut current);
    if is_empty_profile_json(&current) {
        fs::remove_file(&file.target)
            .with_context(|| format!("Failed to remove {}", file.target.display()))?;
    } else {
        crate::fs_util::atomic_write(&file.target, &serde_json::to_string_pretty(&current)?)?;
    }
    if let Err(err) = fs::remove_file(&file.source) {
        eprintln!(
            "Warning: failed to remove profile JSON backup {}: {err}",
            file.source.display()
        );
    }
    Ok(())
}

/// Validate that a managed-region target is the one allowed shared file for the
/// harness (`<repo>/AGENTS.md` for Copilot, `<repo>/CLAUDE.md` for Claude),
/// refusing to touch anything else.
fn ensure_removable_managed_region(
    harness: AgentHarness,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    let allowed = harness.managed_region_path(repo_target);
    if file.target != allowed {
        bail!(
            "Refusing to modify managed region outside the allowed path: {}",
            file.target.display()
        );
    }
    Ok(())
}

/// Reverse a `managed-region` action by stripping this profile's marker block.
///
/// Other profiles' regions and user content are preserved. If the file becomes
/// empty and the profile created it, the file is removed; otherwise it is
/// rewritten with the block stripped.
fn restore_managed_region(
    name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    ensure_removable_managed_region(harness, repo_target, file)?;
    reject_symlink_profile_target(&file.target)?;

    let current = match fs::read_to_string(&file.target) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "Failed to read managed-region target during removal: {}",
                    file.target.display()
                )
            });
        }
    };

    let stripped = strip_managed_region(&current, name);
    if stripped.trim().is_empty() && !file.existed {
        fs::remove_file(&file.target)
            .with_context(|| format!("Failed to remove {}", file.target.display()))?;
    } else {
        crate::fs_util::atomic_write(&file.target, &stripped)?;
    }
    Ok(())
}
///
/// Pointers in `protected` are left untouched: another applied profile still
/// owns them (e.g. a shared `extraKnownMarketplaces` registration), so removing
/// this profile must not unregister them.
///
/// Shared marketplace pointers are special-cased: they are remove-when-last-owner
/// and never restore a captured `prior`. When the first profile to register a
/// marketplace records `existed=false`, a later profile sharing it records
/// `existed=true, prior=<managed value>`. Restoring that managed `prior` would
/// orphan the registration once the last owner is removed in apply order, so we
/// instead delete the key whenever it still holds the value we wrote.
fn restore_owned_json_paths(
    current: &mut Value,
    backup: &MergeJsonBackup,
    target: &Path,
    protected: &BTreeSet<String>,
) {
    for (pointer, entry) in &backup.paths {
        if protected.contains(pointer) {
            continue;
        }
        if entry.existed && !is_shared_marketplace_pointer(pointer) {
            let prior = entry.prior.clone().unwrap_or(Value::Null);
            set_json_pointer(current, pointer, prior);
        } else {
            match current.pointer(pointer) {
                Some(actual) if *actual == entry.written => {
                    remove_json_pointer(current, pointer);
                }
                Some(_) => {
                    eprintln!(
                        "Warning: leaving {pointer} in {} — it was changed after the profile \
                         was applied",
                        target.display()
                    );
                }
                None => {}
            }
        }
    }
}

/// Build an RFC 6901 JSON pointer from path segments, escaping `~` and `/`.
pub(crate) fn json_pointer(segments: &[&str]) -> String {
    let mut out = String::new();
    for segment in segments {
        out.push('/');
        out.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    }
    out
}

/// The begin/end marker lines that delimit a profile's managed region.
fn region_markers(marker_id: &str) -> (String, String) {
    (
        format!("<!-- repoverlay:profile:{marker_id}:begin -->"),
        format!("<!-- repoverlay:profile:{marker_id}:end -->"),
    )
}

/// Whether a line is a reserved repoverlay managed-region marker (begin or end
/// for any profile). Such lines must never appear inside instruction content, or
/// they could split or hijack another profile's block.
fn is_reserved_marker_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("<!-- repoverlay:profile:")
        && (trimmed.ends_with(":begin -->") || trimmed.ends_with(":end -->"))
}

/// Join lines with `\n` and ensure a single trailing newline (empty stays empty).
fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Collapse consecutive blank lines and trim leading/trailing blank lines.
fn tidy_blank_lines(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        let blank = line.trim().is_empty();
        if blank && out.last().is_none_or(|last| last.trim().is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|last| last.trim().is_empty()) {
        out.pop();
    }
    out
}

/// Insert or replace the managed region for `marker_id` in `content`.
///
/// If a region for this id already exists it is replaced in place; otherwise the
/// block is appended after a blank separator, preserving all existing content.
fn upsert_managed_region(content: &str, marker_id: &str, body: &str) -> String {
    let (begin, end) = region_markers(marker_id);
    let block = if body.is_empty() {
        format!("{begin}\n{end}")
    } else {
        format!("{begin}\n{body}\n{end}")
    };
    let lines: Vec<&str> = content.lines().collect();
    let begin_idx = lines.iter().position(|line| *line == begin);
    let end_idx = begin_idx.and_then(|b| {
        lines[b + 1..]
            .iter()
            .position(|line| *line == end)
            .map(|i| i + b + 1)
    });

    if let (Some(begin_idx), Some(end_idx)) = (begin_idx, end_idx) {
        let mut out: Vec<String> = lines[..begin_idx]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        out.extend(block.lines().map(str::to_string));
        out.extend(lines[end_idx + 1..].iter().map(|s| (*s).to_string()));
        return join_lines(&out);
    }

    let mut out: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
    if out.last().is_some_and(|last| !last.trim().is_empty()) {
        out.push(String::new());
    }
    out.extend(block.lines().map(str::to_string));
    join_lines(&out)
}

/// Remove the managed region for `marker_id` from `content`, tidying the blank
/// lines left behind. Other content (user text, other profiles' regions) is kept.
fn strip_managed_region(content: &str, marker_id: &str) -> String {
    let (begin, end) = region_markers(marker_id);
    let lines: Vec<&str> = content.lines().collect();
    let begin_idx = lines.iter().position(|line| *line == begin);
    let end_idx = begin_idx.and_then(|b| {
        lines[b + 1..]
            .iter()
            .position(|line| *line == end)
            .map(|i| i + b + 1)
    });

    if let (Some(begin_idx), Some(end_idx)) = (begin_idx, end_idx) {
        let mut out: Vec<String> = lines[..begin_idx]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        out.extend(lines[end_idx + 1..].iter().map(|s| (*s).to_string()));
        return join_lines(&tidy_blank_lines(out));
    }

    join_lines(&lines.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
}

/// Split a JSON pointer into its unescaped tokens.
fn pointer_tokens(pointer: &str) -> Vec<String> {
    pointer
        .split('/')
        .skip(1)
        .map(|token| token.replace("~1", "/").replace("~0", "~"))
        .collect()
}

/// Set the value at a JSON pointer, creating intermediate objects as needed.
fn set_json_pointer(root: &mut Value, pointer: &str, new_value: Value) {
    let tokens = pointer_tokens(pointer);
    if tokens.is_empty() {
        *root = new_value;
        return;
    }
    let mut current = root;
    for (index, token) in tokens.iter().enumerate() {
        if !current.is_object() {
            *current = Value::Object(serde_json::Map::new());
        }
        let map = current
            .as_object_mut()
            .expect("current was just ensured to be an object");
        if index + 1 == tokens.len() {
            map.insert(token.clone(), new_value);
            return;
        }
        current = map
            .entry(token.clone())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
}

/// Remove the value at a JSON pointer, if present.
fn remove_json_pointer(root: &mut Value, pointer: &str) {
    let tokens = pointer_tokens(pointer);
    let Some((last, parents)) = tokens.split_last() else {
        return;
    };
    let mut current = root;
    for token in parents {
        let Some(next) = current.as_object_mut().and_then(|map| map.get_mut(token)) else {
            return;
        };
        current = next;
    }
    if let Some(map) = current.as_object_mut() {
        map.remove(last);
    }
}

/// Recursively remove empty objects left behind after restoration.
fn prune_empty_objects(value: &mut Value) {
    let Value::Object(map) = value else {
        return;
    };
    let empty_keys: Vec<String> = map
        .iter_mut()
        .filter_map(|(key, child)| {
            prune_empty_objects(child);
            matches!(child, Value::Object(inner) if inner.is_empty()).then(|| key.clone())
        })
        .collect();
    for key in empty_keys {
        map.remove(&key);
    }
}

fn is_empty_profile_json(value: &Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

fn restore_write_file_backup(
    name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    match (&file.backup, file.existed) {
        (Some(backup), _) => {
            ensure_removable_profile_file(name, harness, repo_target, file)?;
            ensure_valid_profile_backup_source(name, harness, repo_target, backup)?;
            if let Some(parent) = file.target.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to recreate parent directory for {}",
                        file.target.display()
                    )
                })?;
            }
            reject_symlink_profile_target(&file.target)?;
            fs::copy(backup, &file.target).with_context(|| {
                format!(
                    "Failed to restore {} from {}",
                    file.target.display(),
                    backup.display()
                )
            })?;
            if let Err(err) = fs::remove_file(backup) {
                eprintln!(
                    "Warning: failed to remove profile file backup {}: {err}",
                    backup.display()
                );
            }
        }
        (None, true) => {
            bail!(
                "Profile file {} was recorded as existing but has no backup",
                file.target.display()
            );
        }
        (None, false) => {
            if file.target.exists() {
                ensure_removable_profile_file(name, harness, repo_target, file)?;
                fs::remove_file(&file.target)
                    .with_context(|| format!("Failed to remove {}", file.target.display()))?;
            }
        }
    }
    Ok(())
}

/// Lexically validate that a plugin-directory target's final component is a
/// single, traversal-free name. The harness-specific root check happens at
/// placement/removal time via [`ensure_plugin_dir_under_skills_root`].
fn validate_plugin_dir_target(target: &Path) -> Result<()> {
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .context("Plugin directory target has no final component")?;
    if name.is_empty() || name == "." || name == ".." {
        bail!("Invalid plugin directory name: {name:?}");
    }
    // Plugin bundles are untrusted: a skill/agent directory name is later written
    // verbatim into `.git/info/exclude`, so a control character (e.g. a newline)
    // could inject extra ignore patterns or corrupt the managed section markers.
    if name.chars().any(char::is_control) {
        bail!("Plugin directory name must not contain control characters: {name:?}");
    }
    if name.contains('/') || name.contains('\\') {
        bail!("Plugin directory name must be a single path component: {name:?}");
    }
    if target
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!(
            "Refusing plugin directory target with parent traversal: {}",
            target.display()
        );
    }
    Ok(())
}

/// Verify a plugin-directory target is exactly `<managed-root>/<single-component>`
/// for the given harness, where `<managed-root>` is the skills root or the agents
/// root, rejecting anything outside those managed locations.
fn ensure_plugin_dir_under_managed_root(
    harness: AgentHarness,
    repo_target: &Path,
    target: &Path,
) -> Result<()> {
    validate_plugin_dir_target(target)?;
    let skills_root = harness.skills_root(repo_target);
    let agents_root = harness.agents_root(repo_target);
    let parent = target
        .parent()
        .context("Plugin directory target has no parent")?;
    if parent != skills_root && parent != agents_root {
        bail!(
            "Refusing plugin placement outside managed skills/agents roots: {}",
            target.display()
        );
    }
    Ok(())
}

/// Recursively copy `src` into `dst`, bailing if any entry is a symlink.
///
/// Plugin bundles ship plain markdown/script trees; rejecting symlinks prevents
/// a malicious or compromised bundle from escaping the placement target.
fn copy_tree_no_symlinks(src: &Path, dst: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(src)
        .with_context(|| format!("Failed to inspect plugin source: {}", src.display()))?;
    if meta.file_type().is_symlink() {
        bail!(
            "Refusing to copy plugin path containing a symlink: {}",
            src.display()
        );
    }
    if meta.is_dir() {
        fs::create_dir_all(dst)
            .with_context(|| format!("Failed to create directory {}", dst.display()))?;
        for entry in fs::read_dir(src)
            .with_context(|| format!("Failed to read plugin directory {}", src.display()))?
        {
            let entry = entry?;
            copy_tree_no_symlinks(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }
        fs::copy(src, dst)
            .with_context(|| format!("Failed to copy {} to {}", src.display(), dst.display()))?;
    }
    Ok(())
}

/// Recursively copy `src` to `dst`, recreating symlinks as symlinks.
///
/// Used only to back up (and later restore) pre-existing *user* content that the
/// placement is about to displace, so symlinks are preserved rather than rejected.
fn copy_tree_preserve(src: &Path, dst: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(src)
        .with_context(|| format!("Failed to inspect {}", src.display()))?;
    if meta.file_type().is_symlink() {
        let link_target = fs::read_link(src)
            .with_context(|| format!("Failed to read symlink {}", src.display()))?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let kind = if fs::metadata(src).map(|m| m.is_dir()).unwrap_or(false) {
            crate::fs_util::SymlinkKind::Dir
        } else {
            crate::fs_util::SymlinkKind::File
        };
        crate::fs_util::create_symlink(&link_target, dst, kind)
            .with_context(|| format!("Failed to recreate symlink {}", dst.display()))?;
    } else if meta.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_tree_preserve(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)
            .with_context(|| format!("Failed to copy {} to {}", src.display(), dst.display()))?;
    }
    Ok(())
}

/// Remove a path whether it is a file, directory tree, or symlink.
fn remove_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => fs::remove_dir_all(path)
            .with_context(|| format!("Failed to remove directory {}", path.display())),
        Ok(_) => {
            fs::remove_file(path).with_context(|| format!("Failed to remove {}", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

/// Move `from` to `to`, falling back to a symlink-preserving copy when a plain
/// rename fails (e.g. across filesystems).
fn move_path(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_tree_preserve(from, to)?;
    remove_path(from)
}

/// Place a plugin skill directory at `dir_target`, backing up any pre-existing
/// content. Returns `(backup, existed)`.
fn place_plugin_dir(
    name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    source: &Path,
    dir_target: &Path,
    action_index: usize,
) -> Result<(Option<PathBuf>, bool)> {
    ensure_plugin_dir_under_managed_root(harness, repo_target, dir_target)?;
    reject_symlink_profile_target(dir_target)?;

    let mut backup = None;
    let existed = dir_target.exists();
    if existed {
        let backup_path = profile_backup_dir(repo_target, name, harness)?
            .join(format!("place-plugin-dir-{action_index}.bak"));
        if backup_path.exists() {
            bail!(
                "Plugin directory backup already exists; refusing to overwrite: {}",
                backup_path.display()
            );
        }
        move_path(dir_target, &backup_path).with_context(|| {
            format!(
                "Failed to back up existing plugin directory {}",
                dir_target.display()
            )
        })?;
        backup = Some(backup_path);
    }

    copy_tree_no_symlinks(source, dir_target).with_context(|| {
        format!(
            "Failed to place plugin directory {} -> {}",
            source.display(),
            dir_target.display()
        )
    })?;
    Ok((backup, existed))
}

/// Remove a placed plugin directory and restore any backed-up prior content.
fn restore_placed_plugin_dir(
    _name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    ensure_plugin_dir_under_managed_root(harness, repo_target, &file.target)?;
    remove_path(&file.target)?;
    if file.existed {
        if let Some(backup) = &file.backup {
            move_path(backup, &file.target).with_context(|| {
                format!(
                    "Failed to restore displaced content for {}",
                    file.target.display()
                )
            })?;
        } else {
            bail!(
                "Plugin directory {} was recorded as existing but has no backup",
                file.target.display()
            );
        }
    }
    Ok(())
}

fn ensure_removable_profile_json(
    harness: AgentHarness,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    // Plugin MCP servers and Claude delegate settings are decomposed into
    // repo-local files; only those exact paths may be restored.
    let allowed = harness.removable_json_targets(repo_target);

    if !allowed.contains(&file.target) {
        bail!(
            "Refusing to restore profile JSON outside managed location: {}",
            file.target.display()
        );
    }
    Ok(())
}

fn ensure_valid_profile_backup_source(
    name: &str,
    harness: AgentHarness,
    repo_target: &Path,
    source: &Path,
) -> Result<()> {
    if source.as_os_str().is_empty() {
        bail!("Profile backup source must not be empty");
    }
    if source
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!(
            "Refusing profile backup source with parent directory traversal: {}",
            source.display()
        );
    }

    let backup_root = profile_backup_dir(repo_target, name, harness)?;
    if !source.starts_with(&backup_root) {
        bail!(
            "Refusing profile backup outside managed location: {}",
            source.display()
        );
    }
    Ok(())
}

fn ensure_regular_profile_source(source: &Path, profile_asset_dir: &Path) -> Result<()> {
    let root_canonical = fs::canonicalize(profile_asset_dir).with_context(|| {
        format!(
            "Failed to inspect profile asset directory: {}",
            profile_asset_dir.display()
        )
    })?;
    let source_canonical = match fs::canonicalize(source) {
        Ok(path) => path,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            bail!("Profile source file not found: {}", source.display());
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "Failed to inspect profile source file: {}",
                    source.display()
                )
            });
        }
    };
    if !source_canonical.starts_with(&root_canonical) {
        bail!(
            "Refusing profile source escaping the profile asset directory: {}",
            source.display()
        );
    }

    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            bail!("Profile source file not found: {}", source.display());
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "Failed to inspect profile source file: {}",
                    source.display()
                )
            });
        }
    };

    if metadata.file_type().is_symlink() {
        bail!("Refusing profile source symlink: {}", source.display());
    }
    if !metadata.is_file() {
        bail!("Profile source file not found: {}", source.display());
    }

    Ok(())
}

fn copy_profile_file(source: &Path, target: &Path, profile_asset_dir: &Path) -> Result<()> {
    ensure_regular_profile_source(source, profile_asset_dir)?;
    reject_symlink_profile_target(target)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, target).with_context(|| {
        format!(
            "Failed to copy {} to {}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}

/// Refuse to write through a profile-managed target whose final component is a
/// symlink, so a pre-existing symlink cannot redirect writes/restores to a file
/// outside the managed location.
fn reject_symlink_profile_target(target: &Path) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!(
            "Refusing to write through symlinked profile target: {}",
            target.display()
        ),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("Failed to inspect profile target: {}", target.display())),
    }
}

/// Apply a `merge-json` action by writing each owned JSON-pointer's value into
/// the target document, replacing it wholesale and creating intermediate
/// objects as needed. Unowned content in the target is preserved.
fn merge_json_value(target: &Path, value: &Value, owned_paths: &[String]) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut merged = if target.exists() {
        let content = fs::read_to_string(target)?;
        serde_json::from_str::<Value>(&content)?
    } else {
        Value::Object(serde_json::Map::new())
    };
    for pointer in owned_paths {
        let new_value = value.pointer(pointer).cloned().unwrap_or(Value::Null);
        set_json_pointer(&mut merged, pointer, new_value);
    }
    crate::fs_util::atomic_write(target, &serde_json::to_string_pretty(&merged)?)?;
    Ok(())
}

fn simple_profile_fingerprint(profile: &crate::profile::ProfileConfig) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    sickle::to_string(profile)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// External profile snapshots (restore after `git clean`)
// ---------------------------------------------------------------------------
//
// All in-repo profile state lives under `.repoverlay/profiles/`, which a
// `git clean -fdx` wipes. To make `restore` able to rebuild a profile we mirror
// a self-contained snapshot of the *resolved* plan (file/dir sources copied
// into `blobs/`, instruction bodies inlined) into the same external location
// used for overlay state. On restore we redirect the recorded sources at the
// external blob directory and re-run the normal apply executor, which
// regenerates a correct in-repo state (including fresh removal backups).

/// On-disk manifest for an externally snapshotted profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileSnapshot {
    name: String,
    harness: AgentHarness,
    mode: ProfileMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    applied_at: chrono::DateTime<chrono::Utc>,
    profile_fingerprint: String,
    #[serde(default)]
    plugins: Vec<crate::profile::ProfilePluginEntry>,
    #[serde(default)]
    skipped: Vec<SkippedCapability>,
    /// Overlay state names this profile applied. Overlay files themselves are
    /// restored independently from overlay external state; recorded here only so
    /// the rebuilt in-repo profile state can later remove them.
    #[serde(default)]
    overlays: Vec<String>,
    actions: Vec<SnapshotAction>,
    /// Set when the profile was removed; such snapshots are skipped by restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A resolved, self-contained profile action. `WriteFile`/`PlacePluginDir`
/// reference a path under the snapshot's `blobs/` directory; managed-region
/// bodies are inlined.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
enum SnapshotAction {
    WriteFile {
        blob: String,
        target: PathBuf,
    },
    MergeJson {
        target: PathBuf,
        value: Value,
        owned_paths: Vec<String>,
    },
    ManagedRegion {
        bodies: Vec<String>,
        target: PathBuf,
        marker_id: String,
    },
    PlacePluginDir {
        blob: String,
        target: PathBuf,
    },
}

const PROFILE_SNAPSHOT_MANIFEST: &str = "snapshot.json";
const PROFILE_SNAPSHOT_BLOBS: &str = "blobs";

/// External directory holding profile snapshots for a target repository.
fn external_profile_root(target: &Path) -> Result<PathBuf> {
    Ok(crate::state::external_state_dir_for_target(target)?.join("profiles"))
}

/// External directory for a single profile snapshot.
fn external_profile_dir(target: &Path, name: &str, harness: AgentHarness) -> Result<PathBuf> {
    crate::profile::validate_profile_state_component(name)?;
    Ok(external_profile_root(target)?.join(format!("{name}.{harness}")))
}

/// Write a self-contained external snapshot of a freshly applied profile.
fn save_external_profile_snapshot(
    target: &Path,
    state: &ProfileState,
    asset_dir: &Path,
    actions: &[ProfileAction],
) -> Result<()> {
    let dir = external_profile_dir(target, &state.name, state.harness)?;
    // Start fresh so a re-apply never inherits stale blobs or a `removed_at`.
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .with_context(|| format!("Failed to clear old profile snapshot: {}", dir.display()))?;
    }
    let blobs_dir = dir.join(PROFILE_SNAPSHOT_BLOBS);
    fs::create_dir_all(&blobs_dir).with_context(|| {
        format!(
            "Failed to create profile snapshot directory: {}",
            blobs_dir.display()
        )
    })?;

    let mut snapshot_actions = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        match action {
            // Overlays are restored from overlay external state; only the names
            // (already in `state.overlays`) are needed here.
            ProfileAction::ApplyOverlay { .. } | ProfileAction::SkipCapability { .. } => {}
            ProfileAction::WriteFile { source, target } => {
                let blob = format!("write-file-{index}");
                ensure_regular_profile_source(source, asset_dir)?;
                fs::copy(source, blobs_dir.join(&blob)).with_context(|| {
                    format!("Failed to snapshot profile file {}", source.display())
                })?;
                snapshot_actions.push(SnapshotAction::WriteFile {
                    blob,
                    target: target.clone(),
                });
            }
            ProfileAction::MergeJson {
                target,
                value,
                owned_paths,
            } => snapshot_actions.push(SnapshotAction::MergeJson {
                target: target.clone(),
                value: value.clone(),
                owned_paths: owned_paths.clone(),
            }),
            ProfileAction::WriteManagedRegion {
                bodies,
                target,
                marker_id,
            } => {
                let mut resolved = Vec::with_capacity(bodies.len());
                for body in bodies {
                    let content = match body {
                        InstructionBody::File { path, .. } => fs::read_to_string(path)
                            .with_context(|| {
                                format!("Failed to snapshot instruction source {}", path.display())
                            })?,
                        InstructionBody::Inline(text) => text.clone(),
                    };
                    resolved.push(content);
                }
                snapshot_actions.push(SnapshotAction::ManagedRegion {
                    bodies: resolved,
                    target: target.clone(),
                    marker_id: marker_id.clone(),
                });
            }
            ProfileAction::PlacePluginDir { source, target } => {
                let blob = format!("plugin-dir-{index}");
                copy_tree_no_symlinks(source, &blobs_dir.join(&blob)).with_context(|| {
                    format!("Failed to snapshot plugin directory {}", source.display())
                })?;
                snapshot_actions.push(SnapshotAction::PlacePluginDir {
                    blob,
                    target: target.clone(),
                });
            }
        }
    }

    let snapshot = ProfileSnapshot {
        name: state.name.clone(),
        harness: state.harness,
        mode: state.mode,
        session_id: state.session_id.clone(),
        applied_at: state.applied_at,
        profile_fingerprint: state.profile_fingerprint.clone(),
        plugins: state.plugins.clone(),
        skipped: state.skipped.clone(),
        overlays: state.overlays.clone(),
        actions: snapshot_actions,
        removed_at: None,
    };
    let manifest = dir.join(PROFILE_SNAPSHOT_MANIFEST);
    let content =
        serde_json::to_string_pretty(&snapshot).context("Failed to serialize profile snapshot")?;
    crate::fs_util::atomic_write(&manifest, &content)
}

/// Mark a profile's external snapshot as removed so `restore` skips it.
pub(crate) fn remove_external_profile_snapshot(
    target: &Path,
    name: &str,
    harness: AgentHarness,
) -> Result<()> {
    let manifest = external_profile_dir(target, name, harness)?.join(PROFILE_SNAPSHOT_MANIFEST);
    if !manifest.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(&manifest)?;
    match serde_json::from_str::<ProfileSnapshot>(&content) {
        Ok(mut snapshot) => {
            snapshot.removed_at = Some(chrono::Utc::now());
            let updated = serde_json::to_string_pretty(&snapshot)
                .context("Failed to serialize profile snapshot")?;
            crate::fs_util::atomic_write(&manifest, &updated)
        }
        Err(err) => {
            warn!(
                "Failed to parse profile snapshot {}, deleting it: {err}",
                manifest.display()
            );
            if let Some(parent) = manifest.parent() {
                let _ = fs::remove_dir_all(parent);
            }
            Ok(())
        }
    }
}

/// Load every restorable (not-removed) external profile snapshot for a target.
fn load_external_profile_snapshots(target: &Path) -> Result<Vec<ProfileSnapshot>> {
    let root = external_profile_root(target)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut snapshots = Vec::new();
    for entry in fs::read_dir(&root)? {
        let manifest = entry?.path().join(PROFILE_SNAPSHOT_MANIFEST);
        if !manifest.is_file() {
            continue;
        }
        let content = fs::read_to_string(&manifest)?;
        match serde_json::from_str::<ProfileSnapshot>(&content) {
            Ok(snapshot) if snapshot.removed_at.is_none() => snapshots.push(snapshot),
            Ok(_) => {}
            Err(err) => warn!(
                "Failed to parse profile snapshot {}: {err}",
                manifest.display()
            ),
        }
    }
    snapshots.sort_by(|a, b| {
        (a.name.as_str(), a.harness.as_str()).cmp(&(b.name.as_str(), b.harness.as_str()))
    });
    Ok(snapshots)
}

/// Summary of restorable profiles for the `restore` command's preview.
pub(crate) struct RestorableProfile {
    pub(crate) name: String,
    pub(crate) harness: AgentHarness,
}

/// List profiles that `restore_profiles` would re-establish for `target`.
pub(crate) fn list_restorable_profiles(target: &Path) -> Result<Vec<RestorableProfile>> {
    let target = crate::resolve::canonicalize_path(target, "Target directory")?;
    Ok(load_external_profile_snapshots(&target)?
        .into_iter()
        .filter(|snapshot| {
            crate::profile::profile_state_path(&target, &snapshot.name, snapshot.harness)
                .map(|path| !path.exists())
                .unwrap_or(false)
        })
        .map(|snapshot| RestorableProfile {
            name: snapshot.name,
            harness: snapshot.harness,
        })
        .collect())
}

/// Re-establish every profile recorded in the external snapshots for `target`.
///
/// Profiles whose in-repo state still exists are left untouched. Returns the
/// number of profiles restored.
pub(crate) fn restore_profiles(target: &Path) -> Result<usize> {
    let target = crate::resolve::canonicalize_path(target, "Target directory")?;
    let snapshots = load_external_profile_snapshots(&target)?;
    let mut restored = 0;
    for snapshot in snapshots {
        if restore_profile_from_snapshot(&target, &snapshot)? {
            restored += 1;
        }
    }
    Ok(restored)
}

/// Rebuild a single profile from its external snapshot.
///
/// Returns `Ok(false)` (and does nothing) when the profile's in-repo state still
/// exists. Sources are redirected at the snapshot's `blobs/` directory and the
/// shared apply executor is re-run, regenerating in-repo state and backups.
fn restore_profile_from_snapshot(target: &Path, snapshot: &ProfileSnapshot) -> Result<bool> {
    let name = snapshot.name.as_str();
    let harness = snapshot.harness;
    let state_path = crate::profile::profile_state_path(target, name, harness)?;
    if state_path.exists() {
        return Ok(false);
    }

    let blobs_dir = external_profile_dir(target, name, harness)?.join(PROFILE_SNAPSHOT_BLOBS);
    let mut actions = Vec::with_capacity(snapshot.actions.len());
    for action in &snapshot.actions {
        match action {
            SnapshotAction::WriteFile { blob, target } => actions.push(ProfileAction::WriteFile {
                source: blobs_dir.join(blob),
                target: target.clone(),
            }),
            SnapshotAction::MergeJson {
                target,
                value,
                owned_paths,
            } => actions.push(ProfileAction::MergeJson {
                target: target.clone(),
                value: value.clone(),
                owned_paths: owned_paths.clone(),
            }),
            SnapshotAction::ManagedRegion {
                bodies,
                target,
                marker_id,
            } => actions.push(ProfileAction::WriteManagedRegion {
                bodies: bodies
                    .iter()
                    .cloned()
                    .map(InstructionBody::Inline)
                    .collect(),
                target: target.clone(),
                marker_id: marker_id.clone(),
            }),
            SnapshotAction::PlacePluginDir { blob, target } => {
                actions.push(ProfileAction::PlacePluginDir {
                    source: blobs_dir.join(blob),
                    target: target.clone(),
                });
            }
        }
    }

    let mut state = ProfileState {
        name: name.to_string(),
        harness,
        mode: snapshot.mode,
        session_id: snapshot.session_id.clone(),
        applied_at: snapshot.applied_at,
        profile_fingerprint: snapshot.profile_fingerprint.clone(),
        // Overlay files are restored separately; record the names so a later
        // `remove` tears the overlays down too.
        overlays: snapshot.overlays.clone(),
        files: Vec::new(),
        skipped: snapshot.skipped.clone(),
        plugins: snapshot.plugins.clone(),
    };

    let restore_result = (|| -> Result<()> {
        for (action_index, action) in actions.into_iter().enumerate() {
            execute_profile_action(
                action,
                action_index,
                name,
                harness,
                target,
                &blobs_dir,
                &mut state,
            )?;
        }
        save_profile_state(target, &state)
    })();

    if let Err(err) = restore_result {
        restore_profile_files(name, harness, target, &state.files).with_context(|| {
            format!("Profile restore failed ({err}); failed to roll back profile file changes")
        })?;
        return Err(err);
    }

    let exclude_entries = profile_exclude_entries(target, &state);
    if !exclude_entries.is_empty() {
        let section = profile_exclude_section(name, harness);
        if let Err(err) = crate::update_git_exclude(target, &section, &exclude_entries, true) {
            eprintln!(
                "Warning: could not update git exclude for restored profile \
                 (files may show as untracked): {err}"
            );
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{InstructionConfig, ProfileConfig};

    #[test]
    fn upsert_managed_region_into_empty_creates_block() {
        let out = upsert_managed_region("", "rust-dev", "Be concise.");
        assert_eq!(
            out,
            "<!-- repoverlay:profile:rust-dev:begin -->\nBe concise.\n<!-- repoverlay:profile:rust-dev:end -->\n"
        );
    }

    #[test]
    fn upsert_managed_region_appends_after_user_content() {
        let out = upsert_managed_region("User notes\n", "rust-dev", "Body");
        assert_eq!(
            out,
            "User notes\n\n<!-- repoverlay:profile:rust-dev:begin -->\nBody\n<!-- repoverlay:profile:rust-dev:end -->\n"
        );
    }

    #[test]
    fn upsert_managed_region_replaces_existing_block_in_place() {
        let initial = upsert_managed_region("Top\n", "rust-dev", "Old");
        let updated = upsert_managed_region(&initial, "rust-dev", "New");
        assert!(updated.contains("New"));
        assert!(!updated.contains("Old"));
        assert!(updated.starts_with("Top\n"));
        assert_eq!(updated.matches("rust-dev:begin").count(), 1);
    }

    #[test]
    fn upsert_managed_region_preserves_other_profiles() {
        let mut content = upsert_managed_region("", "alpha", "A body");
        content = upsert_managed_region(&content, "beta", "B body");
        assert!(content.contains("alpha:begin"));
        assert!(content.contains("beta:begin"));
        let replaced = upsert_managed_region(&content, "alpha", "A2");
        assert!(replaced.contains("A2"));
        assert!(replaced.contains("B body"));
    }

    #[test]
    fn strip_managed_region_removes_only_target_block() {
        let mut content = upsert_managed_region("Header\n", "alpha", "A");
        content = upsert_managed_region(&content, "beta", "B");
        let stripped = strip_managed_region(&content, "alpha");
        assert!(!stripped.contains("alpha:begin"));
        assert!(stripped.contains("beta:begin"));
        assert!(stripped.starts_with("Header\n"));
    }

    #[test]
    fn strip_managed_region_of_created_only_file_is_empty() {
        let content = upsert_managed_region("", "rust-dev", "Body");
        assert_eq!(strip_managed_region(&content, "rust-dev"), "");
    }

    #[test]
    fn strip_managed_region_without_block_is_unchanged() {
        assert_eq!(
            strip_managed_region("Just user text\n", "rust-dev"),
            "Just user text\n"
        );
    }

    #[test]
    fn is_reserved_marker_line_detects_markers() {
        assert!(is_reserved_marker_line(
            "<!-- repoverlay:profile:beta:begin -->"
        ));
        assert!(is_reserved_marker_line(
            "  <!-- repoverlay:profile:beta:end -->  "
        ));
        assert!(!is_reserved_marker_line("just some text"));
        assert!(!is_reserved_marker_line("<!-- repoverlay:other -->"));
    }

    #[test]
    fn strip_ignores_end_marker_appearing_before_begin() {
        // A stray end marker before the real block must not derail stripping.
        let content = format!(
            "<!-- repoverlay:profile:rust-dev:end -->\n{}",
            upsert_managed_region("", "rust-dev", "Body")
        );
        let stripped = strip_managed_region(&content, "rust-dev");
        assert!(!stripped.contains("Body"));
        assert!(!stripped.contains("rust-dev:begin"));
        // The stray pre-begin end marker is left untouched.
        assert!(stripped.contains("rust-dev:end"));
    }

    fn write_config(target: &Path, content: &str) {
        let config_dir = target.join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("config.ccl"), content).unwrap();
    }

    #[test]
    fn apply_profile_rejects_invalid_profile_name_before_side_effects() {
        let temp = tempfile::TempDir::new().unwrap();
        write_config(
            temp.path(),
            r"
profiles =
  rust-dev =
    description = Rust development
",
        );

        let err = apply_profile(
            "bad/name",
            AgentHarness::Copilot,
            temp.path(),
            ProfileMode::Persistent,
            None,
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("profile state component"),
            "unexpected error: {err}"
        );
        assert!(!temp.path().join("mcp.json").exists());
        assert!(!temp.path().join(".repoverlay/profiles").exists());
    }

    #[test]
    fn remove_profile_rejects_write_file_target_outside_harness_home() {
        let temp = tempfile::TempDir::new().unwrap();
        let outside_dir = tempfile::TempDir::new().unwrap();
        let outside = outside_dir.path().join("outside.md");
        fs::write(&outside, "keep me").unwrap();
        crate::profile::save_profile_state(
            temp.path(),
            &ProfileState {
                name: "rust-dev".to_string(),
                harness: AgentHarness::Copilot,
                mode: ProfileMode::Persistent,
                session_id: None,
                applied_at: Utc::now(),
                profile_fingerprint: "test".to_string(),
                overlays: Vec::new(),
                files: vec![ProfileFileEntry {
                    source: PathBuf::from("<test>"),
                    target: outside.clone(),
                    action: "write-file".to_string(),
                    backup: None,
                    existed: false,
                }],
                skipped: Vec::new(),
                plugins: Vec::new(),
            },
        )
        .unwrap();

        let err = remove_profile("rust-dev", AgentHarness::Copilot, temp.path()).unwrap_err();

        assert!(
            err.to_string().contains("Refusing to remove profile file"),
            "unexpected error: {err}"
        );
        assert_eq!(fs::read_to_string(outside).unwrap(), "keep me");
    }

    #[test]
    fn preflight_rejects_missing_source_before_json_write() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("mcp.json");
        let plan = ProfilePlan {
            profile_name: "rust-dev".to_string(),
            harness: AgentHarness::Copilot,
            actions: vec![
                ProfileAction::MergeJson {
                    target: target.clone(),
                    value: serde_json::json!({ "servers": { "rust": { "command": "uvx" } } }),
                    owned_paths: vec!["/servers/rust".to_string()],
                },
                ProfileAction::WriteFile {
                    source: temp.path().join("missing.md"),
                    target: temp.path().join("instructions/rust-dev/missing.md"),
                },
            ],
            plugins: Vec::new(),
        };

        let err = preflight_plan(&plan, temp.path()).unwrap_err();

        assert!(
            err.to_string().contains("Profile source file not found"),
            "unexpected error: {err}"
        );
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn preflight_rejects_symlink_instruction_source() {
        let temp = tempfile::TempDir::new().unwrap();
        let outside = temp.path().join("outside.md");
        let symlink = temp.path().join("copilot-instructions.md");
        fs::write(&outside, "Do not copy through symlink.").unwrap();
        std::os::unix::fs::symlink(&outside, &symlink).unwrap();
        let plan = ProfilePlan {
            profile_name: "rust-dev".to_string(),
            harness: AgentHarness::Copilot,
            actions: vec![ProfileAction::WriteFile {
                source: symlink,
                target: temp
                    .path()
                    .join("instructions/rust-dev/copilot-instructions.md"),
            }],
            plugins: Vec::new(),
        };

        let err = preflight_plan(&plan, temp.path()).unwrap_err();

        assert!(
            err.to_string().contains("profile source symlink"),
            "unexpected error: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn apply_profile_rejects_instruction_source_beneath_symlinked_directory() {
        let temp = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let copilot_home = tempfile::TempDir::new().unwrap();
        fs::write(
            outside.path().join("copilot-instructions.md"),
            "Do not copy from outside profile assets.",
        )
        .unwrap();
        write_config(
            temp.path(),
            r"
profiles =
  rust-dev =
    instructions =
      =
        source = assets/copilot-instructions.md
",
        );
        std::os::unix::fs::symlink(outside.path(), temp.path().join(".repoverlay/assets")).unwrap();

        let err = apply_profile_with_harness_home(
            "rust-dev",
            AgentHarness::Copilot,
            temp.path(),
            ProfileMode::Persistent,
            None,
            copilot_home.path().to_path_buf(),
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("profile source escaping the profile asset directory"),
            "unexpected error: {err}"
        );
        assert!(
            !copilot_home
                .path()
                .join("instructions/rust-dev/copilot-instructions.md")
                .exists()
        );
        assert!(
            !temp
                .path()
                .join(".repoverlay/profiles/rust-dev.copilot.ccl")
                .exists()
        );
    }

    #[test]
    fn apply_profile_uses_honest_fingerprint_prefix() {
        let temp = tempfile::TempDir::new().unwrap();
        write_config(
            temp.path(),
            r"
profiles =
  rust-dev =
    description = Rust development
",
        );

        let state = apply_profile(
            "rust-dev",
            AgentHarness::Copilot,
            temp.path(),
            ProfileMode::Persistent,
            None,
        )
        .unwrap();

        assert!(
            state.profile_fingerprint.starts_with("sickle-hash:"),
            "unexpected fingerprint: {}",
            state.profile_fingerprint
        );
    }

    #[test]
    fn apply_profile_merges_multiple_instructions_into_one_region() {
        let temp = tempfile::TempDir::new().unwrap();
        let copilot_home = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".repoverlay/a")).unwrap();
        fs::create_dir_all(temp.path().join(".repoverlay/b")).unwrap();
        fs::write(temp.path().join(".repoverlay/a/instructions.md"), "first").unwrap();
        fs::write(temp.path().join(".repoverlay/b/instructions.md"), "second").unwrap();
        write_config(
            temp.path(),
            r"
profiles =
  rust-dev =
    instructions =
      =
        source = a/instructions.md
      =
        source = b/instructions.md
",
        );

        apply_profile_with_harness_home(
            "rust-dev",
            AgentHarness::Copilot,
            temp.path(),
            ProfileMode::Persistent,
            None,
            copilot_home.path().to_path_buf(),
        )
        .unwrap();

        // Both same-basename instructions land in the single AGENTS.md region.
        let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("first"));
        assert!(agents.contains("second"));
        assert_eq!(agents.matches("rust-dev:begin").count(), 1);
    }

    #[test]
    fn preflight_rejects_empty_action_inputs() {
        let temp = tempfile::TempDir::new().unwrap();
        let plan = ProfilePlan {
            profile_name: "rust-dev".to_string(),
            harness: AgentHarness::Copilot,
            actions: vec![
                ProfileAction::ApplyOverlay {
                    reference: " ".to_string(),
                },
                ProfileAction::MergeJson {
                    target: PathBuf::new(),
                    value: serde_json::json!({}),
                    owned_paths: Vec::new(),
                },
            ],
            plugins: Vec::new(),
        };

        let err = preflight_plan(&plan, temp.path()).unwrap_err();

        assert!(
            err.to_string()
                .contains("Profile overlay reference must not be empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn merge_json_value_preserves_existing_json() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("mcp.json");
        fs::write(&target, r#"{"servers":{"existing":{"command":"old"}}}"#).unwrap();

        merge_json_value(
            &target,
            &serde_json::json!({ "servers": { "rust": { "command": "uvx" } } }),
            &["/servers/rust".to_string()],
        )
        .unwrap();

        let merged: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target).unwrap()).unwrap();
        assert_eq!(merged["servers"]["existing"]["command"], "old");
        assert_eq!(merged["servers"]["rust"]["command"], "uvx");
    }

    #[test]
    fn merge_json_value_replaces_managed_mcp_server_object() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("mcp.json");
        fs::write(
            &target,
            r#"{"servers":{"rust":{"command":"old","env":{"SECRET":"stale"}},"other":{"command":"keep"}}}"#,
        )
        .unwrap();

        merge_json_value(
            &target,
            &serde_json::json!({
                "servers": {
                    "rust": {
                        "command": "uvx",
                        "env": {}
                    }
                }
            }),
            &["/servers/rust".to_string()],
        )
        .unwrap();

        let merged: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target).unwrap()).unwrap();
        assert_eq!(merged["servers"]["rust"]["command"], "uvx");
        assert_eq!(merged["servers"]["rust"]["env"], serde_json::json!({}));
        assert!(merged["servers"]["rust"]["env"]["SECRET"].is_null());
        assert_eq!(merged["servers"]["other"]["command"], "keep");
    }

    #[test]
    fn merge_json_value_supports_arbitrary_pointer_container() {
        // Claude's `.mcp.json` uses `mcpServers` rather than `servers`.
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join(".mcp.json");
        fs::write(&target, r#"{"mcpServers":{"keep":{"command":"x"}}}"#).unwrap();

        merge_json_value(
            &target,
            &serde_json::json!({ "mcpServers": { "rust": { "command": "uvx" } } }),
            &["/mcpServers/rust".to_string()],
        )
        .unwrap();

        let merged: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target).unwrap()).unwrap();
        assert_eq!(merged["mcpServers"]["rust"]["command"], "uvx");
        assert_eq!(merged["mcpServers"]["keep"]["command"], "x");
    }

    #[test]
    fn backup_records_pointer_prior_and_written_values() {
        let value = serde_json::json!({ "servers": { "rust": { "command": "new" } } });
        let existing = serde_json::json!({ "servers": { "rust": { "command": "old" } } });
        let backup =
            MergeJsonBackup::from_existing(&value, Some(&existing), &["/servers/rust".to_string()]);
        let entry = backup.paths.get("/servers/rust").unwrap();
        assert!(entry.existed);
        assert_eq!(entry.prior, Some(serde_json::json!({ "command": "old" })));
        assert_eq!(entry.written, serde_json::json!({ "command": "new" }));
    }

    #[test]
    fn unmerge_restores_prior_value_when_key_preexisted() {
        let value = serde_json::json!({ "servers": { "rust": { "command": "new" } } });
        let existing = serde_json::json!({ "servers": { "rust": { "command": "old" } } });
        let backup =
            MergeJsonBackup::from_existing(&value, Some(&existing), &["/servers/rust".to_string()]);
        let mut current = serde_json::json!({ "servers": { "rust": { "command": "new" } } });
        restore_owned_json_paths(
            &mut current,
            &backup,
            Path::new("mcp.json"),
            &BTreeSet::new(),
        );
        prune_empty_objects(&mut current);
        assert_eq!(current["servers"]["rust"]["command"], "old");
    }

    #[test]
    fn unmerge_removes_added_key_when_value_unchanged() {
        let value = serde_json::json!({ "servers": { "rust": { "command": "new" } } });
        let backup = MergeJsonBackup::from_existing(&value, None, &["/servers/rust".to_string()]);
        let mut current = serde_json::json!({ "servers": { "rust": { "command": "new" } } });
        restore_owned_json_paths(
            &mut current,
            &backup,
            Path::new("mcp.json"),
            &BTreeSet::new(),
        );
        prune_empty_objects(&mut current);
        // The whole empty container is pruned away.
        assert!(current.get("servers").is_none());
        assert!(is_empty_profile_json(&current));
    }

    #[test]
    fn unmerge_leaves_added_key_when_value_changed_by_user() {
        let value = serde_json::json!({ "servers": { "rust": { "command": "new" } } });
        let backup = MergeJsonBackup::from_existing(&value, None, &["/servers/rust".to_string()]);
        let mut current =
            serde_json::json!({ "servers": { "rust": { "command": "user-edited" } } });
        restore_owned_json_paths(
            &mut current,
            &backup,
            Path::new("mcp.json"),
            &BTreeSet::new(),
        );
        prune_empty_objects(&mut current);
        assert_eq!(current["servers"]["rust"]["command"], "user-edited");
    }

    #[test]
    fn apply_profile_rejects_state_metadata_errors_before_side_effects() {
        let temp = tempfile::TempDir::new().unwrap();
        let copilot_home = temp.path().join("copilot-home");
        write_config(
            temp.path(),
            r"
profiles =
  rust-dev =
    mcps =
      servers =
        rust =
          command = uvx
",
        );
        fs::write(temp.path().join(".repoverlay/profiles"), "not a directory").unwrap();
        let target = copilot_home.join("mcp.json");
        fs::create_dir_all(&copilot_home).unwrap();
        fs::write(&target, r#"{"servers":{"other":{"command":"keep"}}}"#).unwrap();

        let err = apply_profile_with_harness_home(
            "rust-dev",
            AgentHarness::Copilot,
            temp.path(),
            ProfileMode::Persistent,
            None,
            copilot_home,
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("Failed to inspect profile state"),
            "unexpected error: {err}"
        );
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            r#"{"servers":{"other":{"command":"keep"}}}"#
        );
    }

    #[test]
    fn simple_profile_fingerprint_is_stable_for_same_profile() {
        let profile = ProfileConfig {
            instructions: vec![InstructionConfig {
                source: Some("copilot-instructions.md".to_string()),
                content: None,
                base_dir: None,
            }],
            plugins: vec![crate::plugin::PluginRef::Marketplace {
                marketplace: "playground".to_string(),
                name: "rust-dev".to_string(),
                r#ref: None,
                install: crate::plugin::InstallMode::Managed,
                scope: None,
            }],
            ..ProfileConfig::default()
        };

        assert_eq!(
            simple_profile_fingerprint(&profile),
            simple_profile_fingerprint(&profile)
        );
    }

    #[test]
    fn validate_plugin_dir_target_rejects_control_characters() {
        // A skill/agent dir name is later written into `.git/info/exclude`; a
        // newline (legal in Unix filenames) would let an untrusted plugin bundle
        // inject extra ignore patterns or corrupt the managed section markers.
        let skills_root = AgentHarness::Copilot.skills_root(Path::new("/repo"));
        let malicious = skills_root.join("ok\nmalicious-pattern");
        let err = validate_plugin_dir_target(&malicious).unwrap_err();
        assert!(
            err.to_string().contains("control characters"),
            "unexpected error: {err}"
        );

        // A normal name still passes.
        validate_plugin_dir_target(&skills_root.join("rust")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn preflight_rejects_symlinked_merge_json_target() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().to_path_buf();
        let secret = temp.path().join("secret");
        fs::write(&secret, "sensitive").unwrap();
        let json_target = target.join(".mcp.json");
        std::os::unix::fs::symlink(&secret, &json_target).unwrap();

        let plan = ProfilePlan {
            profile_name: "rust-dev".to_string(),
            harness: AgentHarness::Copilot,
            actions: vec![ProfileAction::MergeJson {
                target: json_target,
                value: serde_json::json!({ "mcpServers": {} }),
                owned_paths: Vec::new(),
            }],
            plugins: Vec::new(),
        };

        let err = preflight_plan(&plan, &target).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("symlink"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn restore_profile_files_reverses_a_merge_json_mutation() {
        // A partial apply must be undone exactly as removal would: a merge-json
        // mutation (the class that was previously left orphaned on failure) is
        // reversed via the shared restore helper.
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path();
        let json_target = target.join(".mcp.json");
        fs::write(
            &json_target,
            r#"{"mcpServers":{"existing":{"command":"old"}}}"#,
        )
        .unwrap();

        let value = serde_json::json!({ "mcpServers": { "rust": { "command": "uvx" } } });
        let owned = vec!["/mcpServers/rust".to_string()];

        let backup = capture_merge_json_backup(
            "rust-dev",
            AgentHarness::Copilot,
            target,
            &json_target,
            0,
            &value,
            &owned,
        )
        .unwrap();
        merge_json_value(&json_target, &value, &owned).unwrap();

        let merged: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&json_target).unwrap()).unwrap();
        assert_eq!(merged["mcpServers"]["rust"]["command"], "uvx");

        let entry = ProfileFileEntry {
            source: backup,
            target: json_target.clone(),
            action: "merge-json".to_string(),
            backup: None,
            existed: false,
        };
        restore_profile_files("rust-dev", AgentHarness::Copilot, target, &[entry]).unwrap();

        let restored: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&json_target).unwrap()).unwrap();
        assert!(
            restored["mcpServers"]["rust"].is_null(),
            "leaked merge-json server should be removed on rollback: {restored}"
        );
        assert_eq!(restored["mcpServers"]["existing"]["command"], "old");
    }
}
