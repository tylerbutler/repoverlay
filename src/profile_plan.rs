#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::profile::{
    ProfileFileEntry, ProfileMode, ProfileScope, ProfileState, SkippedCapability,
    save_profile_state,
};
use crate::profile_applicators::claude::ClaudeApplicator;
use crate::profile_applicators::copilot::CopilotApplicator;
use crate::profile_applicators::{ProfileApplicator, ProfileContext};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProfilePlan {
    // `profile_name`/`harness` currently duplicate `ProfileContext`; this is part
    // of the planned typed-harness work — see the design note on `AgentHarness`
    // in `profile_applicators/mod.rs`.
    pub(crate) profile_name: String,
    pub(crate) harness: String,
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
        scope: ProfileScope,
    },
    MergeJson {
        target: PathBuf,
        value: Value,
        scope: ProfileScope,
        /// RFC 6901 JSON-pointer paths this merge owns (e.g. `/servers/rust`).
        /// Ownership is tracked per-pointer for conflict detection and clean
        /// removal; each owned pointer's value is replaced wholesale.
        owned_paths: Vec<String>,
    },
    SkipCapability {
        capability: String,
        reason: String,
    },
    /// Place a plugin's skill directory into a native harness location by
    /// copying/symlinking the whole tree.
    ///
    /// Unlike [`ProfileAction::WriteFile`], the `source` lives in the resolved
    /// plugin bundle (the cache), not the profile asset directory. Execution,
    /// backup, and removal of this action land in the managed-plugin lifecycle
    /// task; until then `preflight_plan` rejects plans that contain it.
    PlacePluginDir {
        source: PathBuf,
        target: PathBuf,
        scope: ProfileScope,
    },
}

pub(crate) fn apply_profile(
    name: &str,
    harness: &str,
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
        harness_home_for(harness)?,
    )
}

fn apply_profile_with_harness_home(
    name: &str,
    harness: &str,
    target: &Path,
    mode: ProfileMode,
    session_id: Option<String>,
    harness_home: PathBuf,
) -> Result<ProfileState> {
    crate::profile::validate_profile_state_component(name)?;
    crate::profile::validate_profile_state_component(harness)?;
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
    let applicator = applicator_for(harness)?;
    let context = ProfileContext {
        profile_name: name.to_string(),
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
        harness: harness.to_string(),
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

    let mut rollbacks = Vec::new();
    let mut dir_rollbacks: Vec<DirRollback> = Vec::new();
    let apply_result = (|| -> Result<()> {
        for (action_index, action) in plan.actions.into_iter().enumerate() {
            match action {
                ProfileAction::ApplyOverlay { reference } => {
                    let before = crate::state::list_applied_overlays(target)?;
                    crate::apply_overlay(
                        &reference,
                        target,
                        false,
                        None,
                        None,
                        true,
                        crate::ConflictStrategy::Fail,
                        false,
                        None,
                        false,
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
                    scope,
                } => {
                    if scope == ProfileScope::User {
                        rollbacks.push(capture_file_rollback(&file_target)?);
                    }
                    let (backup, existed) = capture_write_file_backup(
                        name,
                        harness,
                        target,
                        &file_target,
                        action_index,
                    )?;
                    copy_profile_file(&source, &file_target, &context.profile_asset_dir)?;
                    state.files.push(ProfileFileEntry {
                        source,
                        target: file_target,
                        scope,
                        action: "write-file".to_string(),
                        backup,
                        existed,
                    });
                }
                ProfileAction::MergeJson {
                    target: json_target,
                    value,
                    scope,
                    owned_paths,
                } => {
                    if scope == ProfileScope::User {
                        rollbacks.push(capture_file_rollback(&json_target)?);
                    }
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
                        scope,
                        action: "merge-json".to_string(),
                        backup: None,
                        existed: false,
                    });
                }
                ProfileAction::SkipCapability { capability, reason } => {
                    eprintln!("Warning: skipped {capability}: {reason}");
                    state.skipped.push(SkippedCapability { capability, reason });
                }
                ProfileAction::PlacePluginDir {
                    source,
                    target: dir_target,
                    scope,
                } => {
                    let (backup, existed) = place_plugin_dir(
                        name,
                        harness,
                        target,
                        &source,
                        &dir_target,
                        action_index,
                    )?;
                    dir_rollbacks.push(DirRollback {
                        target: dir_target.clone(),
                        backup: backup.clone(),
                        existed,
                    });
                    state.files.push(ProfileFileEntry {
                        source,
                        target: dir_target,
                        scope,
                        action: "place-plugin-dir".to_string(),
                        backup,
                        existed,
                    });
                }
            }
        }

        save_profile_state(target, &state)
    })();

    if let Err(err) = apply_result {
        rollback_placed_dirs(&dir_rollbacks).with_context(|| {
            format!("Profile apply failed ({err}); failed to roll back plugin placements")
        })?;
        rollback_applied_overlays(target, &state.overlays).with_context(|| {
            format!("Profile apply failed ({err}); failed to roll back applied overlays")
        })?;
        rollback_user_file_changes(&rollbacks).with_context(|| {
            format!("Profile apply failed ({err}); failed to roll back user config changes")
        })?;
        return Err(err);
    }

    println!("Applied profile {name} for {harness}");
    Ok(state)
}

pub(crate) fn remove_profile(name: &str, harness: &str, target: &Path) -> Result<()> {
    crate::profile::validate_profile_state_component(name)?;
    crate::profile::validate_profile_state_component(harness)?;

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
pub(crate) fn remove_profile_for_session(name: &str, harness: &str, target: &Path) -> Result<()> {
    crate::profile::validate_profile_state_component(name)?;
    crate::profile::validate_profile_state_component(harness)?;
    remove_profile_inner(name, harness, target)
}

fn remove_profile_inner(name: &str, harness: &str, target: &Path) -> Result<()> {
    let state = crate::profile::load_profile_state(target, name, harness)?;
    for file in state.files.iter().rev() {
        match file.action.as_str() {
            "write-file" => {
                restore_write_file_backup(name, harness, target, file)?;
            }
            "merge-json" => {
                restore_merge_json_backup(name, harness, target, file)?;
            }
            "place-plugin-dir" => {
                restore_placed_plugin_dir(name, harness, target, file)?;
            }
            _ => {}
        }
    }
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
            .then_with(|| left.harness.cmp(&right.harness))
    });
    Ok(states)
}

fn ensure_removable_profile_file(
    name: &str,
    harness: &str,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    let allowed_root = match (harness, file.scope) {
        ("copilot", ProfileScope::User) => {
            harness_home_for("copilot")?.join("instructions").join(name)
        }
        ("claude", ProfileScope::User) => harness_home_for("claude")?.join("skills"),
        (_, ProfileScope::Repo) => repo_target.to_path_buf(),
        _ => bail!("Unsupported harness '{harness}'"),
    };

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
                // The placed directory must be exactly `<skills-root>/<name>` with a
                // single, traversal-free final component; everything else is rejected
                // before any filesystem mutation runs.
                validate_plugin_dir_target(target)?;
                if !source.exists() {
                    bail!("Plugin skill source does not exist: {}", source.display());
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
    harness: &str,
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
fn check_mcp_ownership_conflicts(plan: &ProfilePlan, target: &Path, harness: &str) -> Result<()> {
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

#[derive(Debug)]
struct FileRollback {
    path: PathBuf,
    prior_bytes: Option<Vec<u8>>,
}

fn capture_file_rollback(path: &Path) -> Result<FileRollback> {
    let prior_bytes = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "Failed to capture existing user config before modifying {}",
                    path.display()
                )
            });
        }
    };

    Ok(FileRollback {
        path: path.to_path_buf(),
        prior_bytes,
    })
}

fn rollback_user_file_changes(rollbacks: &[FileRollback]) -> Result<()> {
    for rollback in rollbacks.iter().rev() {
        match &rollback.prior_bytes {
            Some(bytes) => {
                if let Some(parent) = rollback.path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "Failed to recreate parent directory for {}",
                            rollback.path.display()
                        )
                    })?;
                }
                fs::write(&rollback.path, bytes)
                    .with_context(|| format!("Failed to restore {}", rollback.path.display()))?;
            }
            None => {
                if rollback.path.exists() {
                    fs::remove_file(&rollback.path)
                        .with_context(|| format!("Failed to remove {}", rollback.path.display()))?;
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

fn profile_backup_dir(repo_target: &Path, name: &str, harness: &str) -> Result<PathBuf> {
    crate::profile::validate_profile_state_component(name)?;
    crate::profile::validate_profile_state_component(harness)?;

    Ok(repo_target
        .join(crate::state::STATE_DIR)
        .join("profiles")
        .join("backups")
        .join(format!("{name}.{harness}")))
}

fn merge_json_backup_path(
    repo_target: &Path,
    name: &str,
    harness: &str,
    action_index: usize,
) -> Result<PathBuf> {
    Ok(profile_backup_dir(repo_target, name, harness)?
        .join(format!("merge-json-{action_index}.bak")))
}

fn write_file_backup_path(
    repo_target: &Path,
    name: &str,
    harness: &str,
    action_index: usize,
) -> Result<PathBuf> {
    Ok(profile_backup_dir(repo_target, name, harness)?
        .join(format!("write-file-{action_index}.bak")))
}

fn capture_write_file_backup(
    name: &str,
    harness: &str,
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
    harness: &str,
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
    harness: &str,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    ensure_removable_profile_json(harness, repo_target, file)?;

    ensure_valid_merge_json_backup_source(name, harness, repo_target, &file.source)?;
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
        crate::state::atomic_write(&file.target, &serde_json::to_string(&current)?)?;
    }
    if let Err(err) = fs::remove_file(&file.source) {
        eprintln!(
            "Warning: failed to remove profile JSON backup {}: {err}",
            file.source.display()
        );
    }
    Ok(())
}

/// Reverse a `merge-json` action using its pointer-keyed backup.
///
/// For each owned pointer: restore the prior value if one existed; otherwise
/// remove the key, but only if its current value still equals what we wrote
/// (a user edit since apply is left in place, with a warning).
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
    harness: &str,
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

/// Rollback record for a plugin directory placed during apply, used to reverse
/// the placement if a later action fails before state is persisted.
#[derive(Debug)]
struct DirRollback {
    target: PathBuf,
    backup: Option<PathBuf>,
    existed: bool,
}

/// The managed root under which a harness's plugin skill directories live.
///
/// Repo-local: Claude uses `<repo>/.claude/skills`, Copilot uses
/// `<repo>/.agents/skills`.
fn managed_skills_root(harness: &str, repo_target: &Path) -> Result<PathBuf> {
    match harness {
        "claude" => Ok(repo_target.join(".claude").join("skills")),
        "copilot" => Ok(repo_target.join(".agents").join("skills")),
        _ => bail!("Unsupported harness '{harness}'"),
    }
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

/// Verify a plugin-directory target is exactly `<skills-root>/<single-component>`
/// for the given harness, rejecting anything outside the managed skills root.
fn ensure_plugin_dir_under_skills_root(
    harness: &str,
    repo_target: &Path,
    target: &Path,
) -> Result<()> {
    validate_plugin_dir_target(target)?;
    let skills_root = managed_skills_root(harness, repo_target)?;
    let parent = target
        .parent()
        .context("Plugin directory target has no parent")?;
    if parent != skills_root {
        bail!(
            "Refusing plugin directory placement outside managed skills root: {}",
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
        std::os::unix::fs::symlink(&link_target, dst)
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
    harness: &str,
    repo_target: &Path,
    source: &Path,
    dir_target: &Path,
    action_index: usize,
) -> Result<(Option<PathBuf>, bool)> {
    ensure_plugin_dir_under_skills_root(harness, repo_target, dir_target)?;
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

/// Reverse plugin directory placements after a failed apply, in reverse order.
fn rollback_placed_dirs(dirs: &[DirRollback]) -> Result<()> {
    for placement in dirs.iter().rev() {
        remove_path(&placement.target).with_context(|| {
            format!(
                "Failed to roll back plugin placement {}",
                placement.target.display()
            )
        })?;
        if placement.existed
            && let Some(backup) = &placement.backup
        {
            move_path(backup, &placement.target).with_context(|| {
                format!(
                    "Failed to restore displaced content for {}",
                    placement.target.display()
                )
            })?;
        }
    }
    Ok(())
}

/// Remove a placed plugin directory and restore any backed-up prior content.
fn restore_placed_plugin_dir(
    _name: &str,
    harness: &str,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    ensure_plugin_dir_under_skills_root(harness, repo_target, &file.target)?;
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
    harness: &str,
    repo_target: &Path,
    file: &ProfileFileEntry,
) -> Result<()> {
    // Plugin MCP servers and Claude delegate settings are decomposed into
    // repo-local files; only those exact paths may be restored.
    let allowed: &[PathBuf] = &match harness {
        "claude" => vec![
            repo_target.join(".mcp.json"),
            repo_target.join(".claude").join("settings.json"),
            repo_target.join(".claude").join("settings.local.json"),
        ],
        "copilot" => vec![repo_target.join(".mcp.json")],
        _ => bail!("Unsupported profile JSON target for harness '{harness}'"),
    };

    if file.scope != ProfileScope::Repo || !allowed.contains(&file.target) {
        bail!(
            "Refusing to restore profile JSON outside managed location: {}",
            file.target.display()
        );
    }
    Ok(())
}

fn ensure_valid_merge_json_backup_source(
    name: &str,
    harness: &str,
    repo_target: &Path,
    source: &Path,
) -> Result<()> {
    if source.as_os_str().is_empty() {
        bail!("Profile JSON backup source must not be empty");
    }
    if source
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!(
            "Refusing profile JSON backup source with parent directory traversal: {}",
            source.display()
        );
    }

    let backup_root = profile_backup_dir(repo_target, name, harness)?;
    if !source.starts_with(&backup_root) {
        bail!(
            "Refusing profile JSON backup outside managed location: {}",
            source.display()
        );
    }
    Ok(())
}

fn ensure_valid_profile_backup_source(
    name: &str,
    harness: &str,
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

// String-based harness dispatch is an intentional placeholder; see the design
// note on `AgentHarness` in `profile_applicators/mod.rs`.
fn applicator_for(harness: &str) -> Result<Box<dyn ProfileApplicator>> {
    match harness {
        "copilot" => Ok(Box::new(CopilotApplicator)),
        "claude" => Ok(Box::new(ClaudeApplicator)),
        _ => bail!("Unsupported harness '{harness}'"),
    }
}

/// Resolve the harness home directory for a harness identity, honoring the
/// per-harness `REPOVERLAY_*_HOME` override.
pub(crate) fn harness_home_for(harness: &str) -> Result<PathBuf> {
    match harness {
        "copilot" => CopilotApplicator::harness_home_from_env(),
        "claude" => ClaudeApplicator::harness_home_from_env(),
        _ => bail!("Unsupported harness '{harness}'"),
    }
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
    crate::state::atomic_write(target, &serde_json::to_string_pretty(&merged)?)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{InstructionConfig, ProfileConfig};

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
            "copilot",
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
        let outside = temp.path().join("outside.md");
        fs::write(&outside, "keep me").unwrap();
        crate::profile::save_profile_state(
            temp.path(),
            &ProfileState {
                name: "rust-dev".to_string(),
                harness: "copilot".to_string(),
                mode: ProfileMode::Persistent,
                session_id: None,
                applied_at: Utc::now(),
                profile_fingerprint: "test".to_string(),
                overlays: Vec::new(),
                files: vec![ProfileFileEntry {
                    source: PathBuf::from("<test>"),
                    target: outside.clone(),
                    scope: ProfileScope::User,
                    action: "write-file".to_string(),
                    backup: None,
                    existed: false,
                }],
                skipped: Vec::new(),
                plugins: Vec::new(),
            },
        )
        .unwrap();

        let err = remove_profile("rust-dev", "copilot", temp.path()).unwrap_err();

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
            harness: "copilot".to_string(),
            actions: vec![
                ProfileAction::MergeJson {
                    target: target.clone(),
                    value: serde_json::json!({ "servers": { "rust": { "command": "uvx" } } }),
                    scope: ProfileScope::User,
                    owned_paths: vec!["/servers/rust".to_string()],
                },
                ProfileAction::WriteFile {
                    source: temp.path().join("missing.md"),
                    target: temp.path().join("instructions/rust-dev/missing.md"),
                    scope: ProfileScope::User,
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
            harness: "copilot".to_string(),
            actions: vec![ProfileAction::WriteFile {
                source: symlink,
                target: temp
                    .path()
                    .join("instructions/rust-dev/copilot-instructions.md"),
                scope: ProfileScope::User,
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
        std::os::unix::fs::symlink(outside.path(), temp.path().join("assets")).unwrap();
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

        let err = apply_profile_with_harness_home(
            "rust-dev",
            "copilot",
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
            "copilot",
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
    fn apply_profile_rejects_duplicate_instruction_targets() {
        let temp = tempfile::TempDir::new().unwrap();
        let copilot_home = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("a")).unwrap();
        fs::create_dir_all(temp.path().join("b")).unwrap();
        fs::write(temp.path().join("a/instructions.md"), "first").unwrap();
        fs::write(temp.path().join("b/instructions.md"), "second").unwrap();
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

        let err = apply_profile_with_harness_home(
            "rust-dev",
            "copilot",
            temp.path(),
            ProfileMode::Persistent,
            None,
            copilot_home.path().to_path_buf(),
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("duplicate profile target"),
            "unexpected error: {err}"
        );
        assert!(
            !copilot_home
                .path()
                .join("instructions/rust-dev/instructions.md")
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
    fn preflight_rejects_empty_action_inputs() {
        let temp = tempfile::TempDir::new().unwrap();
        let plan = ProfilePlan {
            profile_name: "rust-dev".to_string(),
            harness: "copilot".to_string(),
            actions: vec![
                ProfileAction::ApplyOverlay {
                    reference: " ".to_string(),
                },
                ProfileAction::MergeJson {
                    target: PathBuf::new(),
                    value: serde_json::json!({}),
                    scope: ProfileScope::User,
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
            "copilot",
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
                source: "copilot-instructions.md".to_string(),
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
}
