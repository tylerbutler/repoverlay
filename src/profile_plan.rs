#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::Value;
use std::fs;
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
    let mut state = ProfileState {
        name: name.to_string(),
        harness: harness.to_string(),
        mode,
        session_id,
        applied_at: Utc::now(),
        profile_fingerprint: format!("sha256:{}", simple_profile_fingerprint(profile)),
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
    fs::write(target, serde_json::to_string_pretty(&merged)?)?;
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

    let serialized = sickle::to_string(profile).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    hasher.finish()
}
