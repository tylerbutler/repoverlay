#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::profile::{
    ProfileFileEntry, ProfileMode, ProfileScope, ProfileState, SkippedCapability,
    save_profile_state,
};
use crate::profile_applicators::copilot::CopilotApplicator;
use crate::profile_applicators::{ProfileApplicator, ProfileContext};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProfilePlan {
    pub(crate) profile_name: String,
    pub(crate) harness: String,
    pub(crate) actions: Vec<ProfileAction>,
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
    },
    SkipCapability {
        capability: String,
        reason: String,
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
        CopilotApplicator::harness_home_from_env()?,
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

    let config = crate::config::load_config(Some(target))?;
    let profile = config
        .profiles
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Profile '{name}' not found"))?;
    let applicator = copilot_applicator(harness)?;
    let context = ProfileContext {
        profile_name: name.to_string(),
        target: target.to_path_buf(),
        profile_asset_dir: target.to_path_buf(),
        harness_home,
        mode,
        session_id: session_id.clone(),
    };
    let plan = applicator.plan(profile, &context)?;
    preflight_plan(&plan, &context.profile_asset_dir)?;
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
    };

    let mut rollbacks = Vec::new();
    let apply_result = (|| -> Result<()> {
        for action in plan.actions {
            match action {
                ProfileAction::ApplyOverlay { reference } => {
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
                    state.overlays.push(reference);
                }
                ProfileAction::WriteFile {
                    source,
                    target,
                    scope,
                } => {
                    if scope == ProfileScope::User {
                        rollbacks.push(capture_file_rollback(&target)?);
                    }
                    copy_profile_file(&source, &target, &context.profile_asset_dir)?;
                    state.files.push(ProfileFileEntry {
                        source,
                        target,
                        scope,
                        action: "write-file".to_string(),
                    });
                }
                ProfileAction::MergeJson {
                    target,
                    value,
                    scope,
                } => {
                    if scope == ProfileScope::User {
                        rollbacks.push(capture_file_rollback(&target)?);
                    }
                    merge_json_value(&target, &value)?;
                    state.files.push(ProfileFileEntry {
                        source: PathBuf::from("<generated>"),
                        target,
                        scope,
                        action: "merge-json".to_string(),
                    });
                }
                ProfileAction::SkipCapability { capability, reason } => {
                    eprintln!("Warning: skipped {capability}: {reason}");
                    state.skipped.push(SkippedCapability { capability, reason });
                }
            }
        }

        save_profile_state(target, &state)
    })();

    if let Err(err) = apply_result {
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

    let state = crate::profile::load_profile_state(target, name, harness)?;
    for file in &state.files {
        if file.action == "write-file" && file.target.exists() {
            ensure_removable_profile_file(name, harness, target, file)?;
            fs::remove_file(&file.target)
                .with_context(|| format!("Failed to remove {}", file.target.display()))?;
        }
    }
    for overlay in &state.overlays {
        if crate::state::load_overlay_state(target, overlay).is_ok() {
            crate::remove_overlay(target, Some(overlay.clone()), false, false)?;
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
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read profile state: {}", path.display()))?;
        states.push(
            sickle::from_str(&content)
                .with_context(|| format!("Failed to parse profile state: {}", path.display()))?,
        );
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
        ("copilot", ProfileScope::User) => CopilotApplicator::harness_home_from_env()?
            .join("instructions")
            .join(name),
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
    for action in &plan.actions {
        match action {
            ProfileAction::WriteFile { source, target, .. } => {
                ensure_regular_profile_source(source, profile_asset_dir)?;
                if target.as_os_str().is_empty() {
                    bail!("Profile target path must not be empty");
                }
            }
            ProfileAction::MergeJson { target, .. } => {
                if target.as_os_str().is_empty() {
                    bail!("Profile JSON target path must not be empty");
                }
            }
            ProfileAction::ApplyOverlay { reference } => {
                if reference.trim().is_empty() {
                    bail!("Profile overlay reference must not be empty");
                }
            }
            ProfileAction::SkipCapability { .. } => {}
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

fn copilot_applicator(harness: &str) -> Result<CopilotApplicator> {
    if harness == "copilot" {
        Ok(CopilotApplicator)
    } else {
        bail!("Unsupported harness '{harness}'")
    }
}

fn copy_profile_file(source: &Path, target: &Path, profile_asset_dir: &Path) -> Result<()> {
    ensure_regular_profile_source(source, profile_asset_dir)?;
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

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let dir = path
        .parent()
        .context("Profile JSON target has no parent directory")?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.persist(path)
        .context("Failed to atomically persist profile JSON")?;
    Ok(())
}

fn merge_json_value(target: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut merged = if target.exists() {
        let content = fs::read_to_string(target)?;
        serde_json::from_str::<Value>(&content)?
    } else {
        Value::Object(serde_json::Map::new())
    };
    merge_json_objects(&mut merged, value);
    atomic_write(target, &serde_json::to_string_pretty(&merged)?)?;
    Ok(())
}

fn merge_json_objects(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                let base_value = base_map.entry(key.clone()).or_insert(Value::Null);
                if key == "servers" {
                    merge_json_servers(base_value, value);
                } else {
                    merge_json_objects(base_value, value);
                }
            }
        }
        (base_value, overlay_value) => *base_value = overlay_value.clone(),
    }
}

fn merge_json_servers(base: &mut Value, overlay: &Value) {
    let Value::Object(overlay_servers) = overlay else {
        *base = overlay.clone();
        return;
    };

    if !base.is_object() {
        *base = Value::Object(serde_json::Map::new());
    }
    let Value::Object(base_servers) = base else {
        unreachable!("base was just initialized as an object");
    };

    for (server_name, server_value) in overlay_servers {
        base_servers.insert(server_name.clone(), server_value.clone());
    }
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
    use crate::profile::{InstructionConfig, McpConfig, McpServerConfig, ProfileConfig};
    use std::collections::BTreeMap;

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
                }],
                skipped: Vec::new(),
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
                },
                ProfileAction::WriteFile {
                    source: temp.path().join("missing.md"),
                    target: temp.path().join("instructions/rust-dev/missing.md"),
                    scope: ProfileScope::User,
                },
            ],
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
                },
            ],
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
    fn apply_profile_rolls_back_user_files_when_state_save_fails() {
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
            err.to_string().contains("File exists"),
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
            mcps: McpConfig {
                servers: BTreeMap::from([(
                    "rust".to_string(),
                    McpServerConfig {
                        command: "uvx".to_string(),
                        args: vec!["mcp-rust".to_string()],
                        env: BTreeMap::new(),
                    },
                )]),
            },
            ..ProfileConfig::default()
        };

        assert_eq!(
            simple_profile_fingerprint(&profile),
            simple_profile_fingerprint(&profile)
        );
    }
}
