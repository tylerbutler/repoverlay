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
        harness_home: CopilotApplicator::harness_home_from_env()?,
        mode,
        session_id: session_id.clone(),
    };
    let plan = applicator.plan(profile, &context)?;
    preflight_plan(&plan)?;
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
                copy_profile_file(&source, &target)?;
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

    save_profile_state(target, &state)?;
    println!("Applied profile {name} for {harness}");
    Ok(state)
}

fn preflight_plan(plan: &ProfilePlan) -> Result<()> {
    for action in &plan.actions {
        match action {
            ProfileAction::WriteFile { source, target, .. } => {
                if !source.is_file() {
                    bail!("Profile source file not found: {}", source.display());
                }
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

fn copilot_applicator(harness: &str) -> Result<CopilotApplicator> {
    if harness == "copilot" {
        Ok(CopilotApplicator)
    } else {
        bail!("Unsupported harness '{harness}'")
    }
}

fn copy_profile_file(source: &Path, target: &Path) -> Result<()> {
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
                merge_json_objects(base_map.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base_value, overlay_value) => *base_value = overlay_value.clone(),
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

        let err = preflight_plan(&plan).unwrap_err();

        assert!(
            err.to_string().contains("Profile source file not found"),
            "unexpected error: {err}"
        );
        assert!(!target.exists());
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

        let err = preflight_plan(&plan).unwrap_err();

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
