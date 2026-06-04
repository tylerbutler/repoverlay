//! Profile configuration, merge rules, and state metadata.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) plugins: Vec<crate::plugin::PluginRef>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct InstructionConfig {
    /// Relative path to an instruction file, resolved against `base_dir` (the
    /// directory of the config file that defined this entry). Mutually exclusive
    /// with `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    /// Inline instruction text written directly into the managed region.
    /// Mutually exclusive with `source`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<String>,
    /// Directory the relative `source` path is rooted at — the originating config
    /// file's directory, stamped at load time. Runtime-only; never serialized and
    /// excluded from equality (it is provenance, not identity).
    #[serde(skip)]
    pub(crate) base_dir: Option<PathBuf>,
}

impl PartialEq for InstructionConfig {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.content == other.content
    }
}

impl Eq for InstructionConfig {}

impl InstructionConfig {
    /// Require exactly one of `source`/`content` to be set.
    pub(crate) fn validate_exactly_one(&self) -> Result<()> {
        match (&self.source, &self.content) {
            (Some(_), Some(_)) => {
                bail!("instruction entry sets both `source` and `content`; set exactly one")
            }
            (None, None) => {
                bail!("instruction entry sets neither `source` nor `content`; set exactly one")
            }
            _ => Ok(()),
        }
    }

    /// A short label for diagnostics: the source path, or `<inline>` for content.
    pub(crate) fn label(&self) -> String {
        self.source
            .clone()
            .unwrap_or_else(|| "<inline>".to_string())
    }

    /// Inline `content` normalized for embedding: a CCL multiline block arrives
    /// with a leading newline and uniform indentation, so we drop one leading
    /// newline and strip the common leading indentation (textwrap-style dedent).
    pub(crate) fn normalized_content(&self) -> Option<String> {
        self.content.as_deref().map(dedent_block)
    }
}

/// Remove a single leading newline and the longest common leading whitespace
/// prefix shared by all non-blank lines. Trailing newlines are left for the
/// caller's existing trim step.
fn dedent_block(s: &str) -> String {
    let s = s.strip_prefix('\n').unwrap_or(s);
    let leading_ws = |line: &str| line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let min_indent = s
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(leading_ws)
        .min()
        .unwrap_or(0);
    s.lines()
        .map(|line| {
            if line.len() >= min_indent {
                &line[min_indent..]
            } else {
                line.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn merge_profile_config(
    base: &ProfileConfig,
    override_profile: &ProfileConfig,
) -> ProfileConfig {
    ProfileConfig {
        description: override_profile
            .description
            .clone()
            .or_else(|| base.description.clone()),
        overlays: merge_list(&base.overlays, &override_profile.overlays),
        instructions: merge_list(&base.instructions, &override_profile.instructions),
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
    // PID 0 addresses the caller's whole process group with kill(2), so it can
    // never identify a single lock owner. Treat it as stale.
    if pid == 0 {
        return false;
    }

    let Ok(raw_pid) = i32::try_from(pid) else {
        return false;
    };

    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(raw_pid), None) {
        Ok(()) | Err(nix::errno::Errno::EPERM) => true,
        Err(_) => false,
    }
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

/// Validate that a profile name is safe to embed in `AGENTS.md` managed-region
/// markers. Markers are line-delimited HTML comments, so the name must not
/// contain whitespace or characters that could break out of the comment or the
/// marker grammar. Restricting to `[A-Za-z0-9._-]` keeps markers parseable and
/// injection-safe.
pub(crate) fn validate_profile_marker_component(component: &str) -> Result<()> {
    validate_profile_state_component(component)?;
    if !component
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        bail!(
            "Profile name {component:?} contains characters that cannot be used in an \
             AGENTS.md managed region; allowed characters are letters, digits, '.', '_' and '-'"
        );
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn save_profile_state(target: &Path, state: &ProfileState) -> Result<()> {
    let path = profile_state_path(target, &state.name, &state.harness)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = sickle::to_string(state).context("Failed to serialize profile state")?;
    crate::state::atomic_write(&path, &content)
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

/// Load every applied profile state recorded under `<target>/.repoverlay/profiles/`.
///
/// State files are read and deserialized rather than parsed from their
/// filenames, because profile names may legally contain `.`. A file that fails
/// to parse is skipped (with the error returned alongside its path) so a single
/// corrupt state file does not abort bulk operations such as `update`.
#[allow(dead_code)]
pub(crate) fn list_applied_profile_states(target: &Path) -> Result<Vec<ProfileState>> {
    let dir = target.join(crate::state::STATE_DIR).join(PROFILES_DIR);
    if !dir.try_exists().unwrap_or(false) {
        return Ok(Vec::new());
    }
    let mut states = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ccl") {
            continue;
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read profile state: {}", path.display()))?;
        match sickle::from_str::<ProfileState>(&content) {
            Ok(state) => states.push(state),
            Err(err) => {
                eprintln!(
                    "  ? skipping unreadable profile state {}: {err}",
                    path.display()
                );
            }
        }
    }
    states.sort_by(|a, b| (a.name.as_str(), a.harness.as_str()).cmp(&(&b.name, &b.harness)));
    Ok(states)
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
    #[serde(default)]
    pub(crate) plugins: Vec<ProfilePluginEntry>,
    /// Ephemeral-only: cached bundle directories passed to the harness via
    /// `--plugin-dir`. Recorded for introspection; nothing is placed on disk
    /// for these, so removal is a no-op for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) plugin_dirs: Vec<PathBuf>,
}

/// Provenance for a managed plugin that was resolved, cached, and decomposed
/// into native harness placements during apply.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ProfilePluginEntry {
    pub(crate) reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) resolved_commit: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ProfileFileEntry {
    pub(crate) source: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) backup: Option<PathBuf>,
    #[serde(default)]
    pub(crate) existed: bool,
}

/// Settings scope for the delegate-to-Claude plugin fallback.
///
/// Selects which repo-local Claude `settings.json` file receives the
/// `enabledPlugins` / `extraKnownMarketplaces` entries. All profile artifacts
/// are repo-local; this only distinguishes the project/local settings split.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub(crate) enum DelegateScope {
    /// `.claude/settings.json` (team-shareable, committed)
    Project,
    /// `.claude/settings.local.json` (gitignored)
    Local,
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
    use crate::plugin::{InstallMode, PluginRef};

    #[test]
    fn parses_inline_instruction_content_from_ccl() {
        let ccl = "
profiles =
  rust-dev =
    instructions =
      =
        content =
          Be concise in all responses.
          Prefer composition over inheritance.
";

        let config: crate::config::RepoverlayConfig = sickle::from_str(ccl).unwrap();
        let profile = config.profiles.get("rust-dev").unwrap();
        assert_eq!(profile.instructions[0].source, None);
        assert_eq!(
            profile.instructions[0].normalized_content().as_deref(),
            Some("Be concise in all responses.\nPrefer composition over inheritance.")
        );
    }

    #[test]
    fn instruction_requires_exactly_one_of_source_or_content() {
        let both = InstructionConfig {
            source: Some("a.md".to_string()),
            content: Some("inline".to_string()),
            base_dir: None,
        };
        assert!(both.validate_exactly_one().is_err());

        let neither = InstructionConfig::default();
        assert!(neither.validate_exactly_one().is_err());

        let source_only = InstructionConfig {
            source: Some("a.md".to_string()),
            ..InstructionConfig::default()
        };
        assert!(source_only.validate_exactly_one().is_ok());

        let content_only = InstructionConfig {
            content: Some("inline".to_string()),
            ..InstructionConfig::default()
        };
        assert!(content_only.validate_exactly_one().is_ok());
    }

    #[test]
    fn instruction_equality_ignores_base_dir() {
        let a = InstructionConfig {
            source: Some("a.md".to_string()),
            content: None,
            base_dir: Some(PathBuf::from("/one")),
        };
        let b = InstructionConfig {
            source: Some("a.md".to_string()),
            content: None,
            base_dir: Some(PathBuf::from("/two")),
        };
        assert_eq!(a, b);
    }

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
    plugins =
      = playground/rust-dev
";

        let config: crate::config::RepoverlayConfig = sickle::from_str(ccl).unwrap();
        let profile = config.profiles.get("rust-dev").unwrap();
        assert_eq!(profile.description.as_deref(), Some("Rust profile"));
        assert_eq!(profile.overlays, vec!["rust-base"]);
        assert_eq!(
            profile.instructions[0].source.as_deref(),
            Some("copilot-instructions.md")
        );
        assert_eq!(
            profile.plugins,
            vec![PluginRef::Marketplace {
                marketplace: "playground".to_string(),
                name: "rust-dev".to_string(),
                r#ref: None,
                install: InstallMode::Managed,
                scope: None,
            }]
        );
    }

    #[test]
    fn merge_profile_uses_type_based_rules() {
        let base = ProfileConfig {
            description: Some("Base".to_string()),
            overlays: vec!["base-overlay".to_string()],
            instructions: vec![InstructionConfig {
                source: Some("base.md".to_string()),
                content: None,
                base_dir: None,
            }],
            plugins: vec![PluginRef::Marketplace {
                marketplace: "playground".to_string(),
                name: "base-plugin".to_string(),
                r#ref: None,
                install: InstallMode::Managed,
                scope: None,
            }],
        };
        let overlay = ProfileConfig {
            description: None,
            overlays: vec!["repo-overlay".to_string()],
            instructions: Vec::new(),
            plugins: vec![PluginRef::Marketplace {
                marketplace: "playground".to_string(),
                name: "repo-plugin".to_string(),
                r#ref: None,
                install: InstallMode::Managed,
                scope: None,
            }],
        };

        let merged = merge_profile_config(&base, &overlay);
        assert_eq!(merged.description.as_deref(), Some("Base"));
        assert_eq!(merged.overlays, vec!["repo-overlay"]);
        assert_eq!(merged.instructions[0].source.as_deref(), Some("base.md"));
        // Plugins follow the list-replace rule: a non-empty override wins.
        assert_eq!(
            merged.plugins,
            vec![PluginRef::Marketplace {
                marketplace: "playground".to_string(),
                name: "repo-plugin".to_string(),
                r#ref: None,
                install: InstallMode::Managed,
                scope: None,
            }]
        );
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
            plugins: Vec::new(),
            plugin_dirs: Vec::new(),
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
            plugins: Vec::new(),
            plugin_dirs: Vec::new(),
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
