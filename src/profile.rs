//! Profile configuration, merge rules, and state metadata.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
const PROFILES_DIR: &str = "profiles";

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct ProfileConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) overlays: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) instructions: Vec<InstructionConfig>,
    #[serde(skip_serializing_if = "McpConfig::is_empty")]
    pub(crate) mcps: McpConfig,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) skills: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) plugins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct InstructionConfig {
    pub(crate) source: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct McpConfig {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) servers: BTreeMap<String, McpServerConfig>,
}

impl McpConfig {
    pub(crate) fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct McpServerConfig {
    pub(crate) command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
}

pub(crate) fn merge_profile_config(
    base: &ProfileConfig,
    override_profile: &ProfileConfig,
) -> ProfileConfig {
    let mut mcps = base.mcps.clone();
    for (name, server) in &override_profile.mcps.servers {
        mcps.servers.insert(name.clone(), server.clone());
    }

    ProfileConfig {
        description: override_profile
            .description
            .clone()
            .or_else(|| base.description.clone()),
        overlays: merge_list(&base.overlays, &override_profile.overlays),
        instructions: merge_list(&base.instructions, &override_profile.instructions),
        mcps,
        skills: merge_list(&base.skills, &override_profile.skills),
        plugins: merge_list(&base.plugins, &override_profile.plugins),
    }
}

fn merge_list<T: Clone>(base: &[T], override_list: &[T]) -> Vec<T> {
    if override_list.is_empty() {
        base.to_vec()
    } else {
        override_list.to_vec()
    }
}

#[allow(dead_code)]
pub(crate) fn profile_state_path(target: &Path, name: &str, harness: &str) -> Result<PathBuf> {
    validate_profile_state_component(name)?;
    validate_profile_state_component(harness)?;

    Ok(target
        .join(crate::state::STATE_DIR)
        .join(PROFILES_DIR)
        .join(format!("{name}.{harness}.ccl")))
}

pub(crate) fn profile_lock_path(target: &Path, name: &str, harness: &str) -> Result<PathBuf> {
    validate_profile_state_component(name)?;
    validate_profile_state_component(harness)?;

    Ok(target
        .join(crate::state::STATE_DIR)
        .join(PROFILES_DIR)
        .join(format!("{name}.{harness}.lock")))
}

/// Liveness of an ephemeral session lock file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockState {
    /// No lock file is present.
    Absent,
    /// The lock file is held by a process that is still alive.
    Live,
    /// The lock file exists but its owner is dead, missing, or unparsable.
    ///
    /// Stale locks are left behind when a session is `SIGKILL`ed or the machine
    /// loses power, and may be safely recovered.
    Stale,
}

/// Inspect an ephemeral session lock file and classify its liveness.
///
/// The lock file stores the PID of the owning process. A lock is considered
/// [`LockState::Stale`] when the file is empty, its contents cannot be parsed as
/// a PID, or the recorded process is no longer alive. This lets callers recover
/// from locks orphaned by `SIGKILL` or power loss instead of blocking forever.
pub(crate) fn inspect_lock(path: &Path) -> Result<LockState> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LockState::Absent);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read profile lock: {}", path.display()));
        }
    };

    match content.trim().parse::<u32>() {
        Ok(pid) if pid_is_alive(pid) => Ok(LockState::Live),
        _ => Ok(LockState::Stale),
    }
}

/// Check whether a process with the given PID is currently alive.
#[cfg(unix)]
pub(crate) fn pid_is_alive(pid: u32) -> bool {
    // PID 0 addresses the caller's whole process group with `kill(2)`, so it can
    // never identify a single lock owner. Treat it as stale.
    if pid == 0 {
        return false;
    }

    #[allow(unsafe_code, clippy::cast_possible_wrap)]
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        // Signal 0 was delivered: the process exists and we may signal it.
        return true;
    }

    // EPERM means the process exists but we lack permission; ESRCH means it is
    // gone. Anything else is treated conservatively as gone.
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

/// Check whether a process with the given PID is currently alive.
///
/// On non-Unix platforms liveness cannot be probed portably, so we conservatively
/// assume the owner is still alive to avoid deleting a valid lock.
#[cfg(not(unix))]
pub(crate) fn pid_is_alive(_pid: u32) -> bool {
    true
}

pub(crate) fn validate_profile_state_component(component: &str) -> Result<()> {
    if component.is_empty()
        || matches!(component, "." | "..")
        || component.contains('/')
        || component.contains('\\')
    {
        bail!("Invalid profile state component: {component:?}");
    }
    Ok(())
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let dir = path
        .parent()
        .context("Profile state file has no parent directory")?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.persist(path)
        .context("Failed to atomically persist profile state")?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn save_profile_state(target: &Path, state: &ProfileState) -> Result<()> {
    let path = profile_state_path(target, &state.name, &state.harness)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = sickle::to_string(state).context("Failed to serialize profile state")?;
    atomic_write(&path, &content)
        .with_context(|| format!("Failed to write profile state: {}", path.display()))?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn load_profile_state(target: &Path, name: &str, harness: &str) -> Result<ProfileState> {
    let path = profile_state_path(target, name, harness)?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read profile state: {}", path.display()))?;
    sickle::from_str(&content)
        .with_context(|| format!("Failed to parse profile state: {}", path.display()))
}

#[allow(dead_code)]
pub(crate) fn remove_profile_state(target: &Path, name: &str, harness: &str) -> Result<()> {
    let path = profile_state_path(target, name, harness)?;
    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("Failed to remove profile state: {}", path.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub(crate) enum ProfileMode {
    Persistent,
    Ephemeral,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ProfileState {
    pub(crate) name: String,
    pub(crate) harness: String,
    pub(crate) mode: ProfileMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    pub(crate) applied_at: DateTime<Utc>,
    pub(crate) profile_fingerprint: String,
    #[serde(default)]
    pub(crate) overlays: Vec<String>,
    #[serde(default)]
    pub(crate) files: Vec<ProfileFileEntry>,
    #[serde(default)]
    pub(crate) skipped: Vec<SkippedCapability>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ProfileFileEntry {
    pub(crate) source: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) scope: ProfileScope,
    pub(crate) action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) backup: Option<PathBuf>,
    #[serde(default)]
    pub(crate) existed: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub(crate) enum ProfileScope {
    Repo,
    User,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct SkippedCapability {
    pub(crate) capability: String,
    pub(crate) reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn parses_profile_config_from_ccl() {
        let ccl = r"
profiles =
  rust-dev =
    description = Rust profile
    overlays =
      = rust-base
    instructions =
      =
        source = copilot-instructions.md
    mcps =
      servers =
        rust =
          command = uvx
          args =
            = mcp-rust
    skills =
      = market:rust-reviewer@playground
    plugins =
      = market:rust-dev@playground
";

        let config: crate::config::RepoverlayConfig = sickle::from_str(ccl).unwrap();
        let profile = config.profiles.get("rust-dev").unwrap();
        assert_eq!(profile.description.as_deref(), Some("Rust profile"));
        assert_eq!(profile.overlays, vec!["rust-base"]);
        assert_eq!(profile.instructions[0].source, "copilot-instructions.md");
        assert_eq!(profile.mcps.servers["rust"].command, "uvx");
        assert_eq!(profile.mcps.servers["rust"].args, vec!["mcp-rust"]);
        assert_eq!(profile.skills, vec!["market:rust-reviewer@playground"]);
        assert_eq!(profile.plugins, vec!["market:rust-dev@playground"]);
    }

    #[test]
    fn rejects_mcp_server_without_command() {
        let ccl = r"
profiles =
  bad =
    mcps =
      servers =
        broken =
          args =
            = mcp-broken
";

        assert!(sickle::from_str::<crate::config::RepoverlayConfig>(ccl).is_err());
    }

    #[test]
    fn merge_profile_uses_type_based_rules() {
        let mut base_servers = BTreeMap::new();
        base_servers.insert(
            "shared".to_string(),
            McpServerConfig {
                command: "base-command".to_string(),
                args: vec!["base".to_string()],
                env: BTreeMap::new(),
            },
        );
        base_servers.insert(
            "base-only".to_string(),
            McpServerConfig {
                command: "base-only-command".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
        );

        let mut override_servers = BTreeMap::new();
        override_servers.insert(
            "shared".to_string(),
            McpServerConfig {
                command: "override-command".to_string(),
                args: vec!["override".to_string()],
                env: BTreeMap::new(),
            },
        );

        let base = ProfileConfig {
            description: Some("Base".to_string()),
            overlays: vec!["base-overlay".to_string()],
            instructions: vec![InstructionConfig {
                source: "base.md".to_string(),
            }],
            mcps: McpConfig {
                servers: base_servers,
            },
            skills: vec!["base-skill".to_string()],
            plugins: vec!["base-plugin".to_string()],
        };
        let overlay = ProfileConfig {
            description: None,
            overlays: vec!["repo-overlay".to_string()],
            instructions: Vec::new(),
            mcps: McpConfig {
                servers: override_servers,
            },
            skills: Vec::new(),
            plugins: vec!["repo-plugin".to_string()],
        };

        let merged = merge_profile_config(&base, &overlay);
        assert_eq!(merged.description.as_deref(), Some("Base"));
        assert_eq!(merged.overlays, vec!["repo-overlay"]);
        assert_eq!(merged.instructions[0].source, "base.md");
        assert_eq!(merged.mcps.servers["shared"].command, "override-command");
        assert_eq!(
            merged.mcps.servers["base-only"].command,
            "base-only-command"
        );
        assert_eq!(merged.skills, vec!["base-skill"]);
        assert_eq!(merged.plugins, vec!["repo-plugin"]);
    }

    #[test]
    fn saves_and_loads_profile_state() {
        let temp = tempfile::TempDir::new().unwrap();
        let state = ProfileState {
            name: "rust-dev".to_string(),
            harness: "copilot".to_string(),
            mode: ProfileMode::Persistent,
            session_id: None,
            applied_at: Utc::now(),
            profile_fingerprint: "sickle-hash:test".to_string(),
            overlays: vec!["rust-base".to_string()],
            files: Vec::new(),
            skipped: Vec::new(),
        };

        save_profile_state(temp.path(), &state).unwrap();
        let loaded = load_profile_state(temp.path(), "rust-dev", "copilot").unwrap();
        assert_eq!(loaded.name, "rust-dev");
        assert_eq!(loaded.harness, "copilot");
        assert_eq!(loaded.overlays, vec!["rust-base"]);
    }

    #[test]
    fn removes_profile_state_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let state = ProfileState {
            name: "rust-dev".to_string(),
            harness: "copilot".to_string(),
            mode: ProfileMode::Persistent,
            session_id: None,
            applied_at: Utc::now(),
            profile_fingerprint: "sickle-hash:test".to_string(),
            overlays: Vec::new(),
            files: Vec::new(),
            skipped: Vec::new(),
        };

        save_profile_state(temp.path(), &state).unwrap();
        remove_profile_state(temp.path(), "rust-dev", "copilot").unwrap();
        assert!(load_profile_state(temp.path(), "rust-dev", "copilot").is_err());
    }

    #[test]
    fn profile_state_path_rejects_traversal_components() {
        let temp = tempfile::TempDir::new().unwrap();
        for invalid in ["../evil", "bad/name", "bad\\name", ".", ""] {
            let name_err = profile_state_path(temp.path(), invalid, "copilot").unwrap_err();
            assert!(
                name_err.to_string().contains("profile state component"),
                "unexpected error for invalid profile name {invalid:?}: {name_err}"
            );

            let harness_err = profile_state_path(temp.path(), "rust-dev", invalid).unwrap_err();
            assert!(
                harness_err.to_string().contains("profile state component"),
                "unexpected error for invalid harness {invalid:?}: {harness_err}"
            );
        }
    }

    #[test]
    fn profile_lock_path_uses_validated_profile_components() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = profile_lock_path(temp.path(), "rust-dev", "copilot").unwrap();
        assert_eq!(
            path,
            temp.path()
                .join(crate::state::STATE_DIR)
                .join(PROFILES_DIR)
                .join("rust-dev.copilot.lock")
        );

        let err = profile_lock_path(temp.path(), "../evil", "copilot").unwrap_err();
        assert!(err.to_string().contains("profile state component"));
    }

    #[test]
    fn inspect_lock_reports_absent_when_missing() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("missing.lock");
        assert_eq!(inspect_lock(&path).unwrap(), LockState::Absent);
    }

    #[test]
    fn inspect_lock_reports_live_for_current_process() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("live.lock");
        fs::write(&path, format!("{}\n", std::process::id())).unwrap();
        assert_eq!(inspect_lock(&path).unwrap(), LockState::Live);
    }

    #[cfg(unix)]
    #[test]
    fn inspect_lock_reports_stale_for_dead_pid() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("dead.lock");
        // i32::MAX is above every platform's PID ceiling, so it cannot be alive.
        fs::write(&path, format!("{}\n", i32::MAX)).unwrap();
        assert_eq!(inspect_lock(&path).unwrap(), LockState::Stale);
    }

    #[cfg(unix)]
    #[test]
    fn inspect_lock_reports_stale_for_unparsable_contents() {
        let temp = tempfile::TempDir::new().unwrap();
        for contents in ["", "   ", "not-a-pid", "0"] {
            let path = temp.path().join("garbage.lock");
            fs::write(&path, contents).unwrap();
            assert_eq!(
                inspect_lock(&path).unwrap(),
                LockState::Stale,
                "expected stale lock for contents {contents:?}"
            );
        }
    }

    #[test]
    fn pid_is_alive_detects_current_process() {
        assert!(pid_is_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn pid_is_alive_rejects_zero_pid() {
        assert!(!pid_is_alive(0));
    }
}
