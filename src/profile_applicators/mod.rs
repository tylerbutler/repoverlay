//! Profile applicators provide profile-specific integration behavior.
#![allow(dead_code)]

pub(crate) mod claude;
pub(crate) mod copilot;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use crate::config::Marketplace;
use crate::profile::{ProfileConfig, ProfileMode};
use crate::profile_plan::ProfilePlan;

/// The agent harness a profile is being applied to.
///
/// This is the single source of truth for harness identity: it serializes to
/// the stable on-disk identifiers (`"claude"`, `"copilot"`) used in profile
/// state files and CLI arguments, and owns every per-harness mapping (config
/// home, launch program, repo-local placement roots, removable JSON targets, and
/// the applicator implementing its planning behavior) so dispatch never threads
/// raw strings around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub(crate) enum AgentHarness {
    Copilot,
    Claude,
}

impl AgentHarness {
    /// Stable on-disk / wire identifier (`"claude"`, `"copilot"`).
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Copilot => "copilot",
            Self::Claude => "claude",
        }
    }

    /// Human-facing label for user messages (`"Claude"`, `"Copilot"`).
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Copilot => "Copilot",
            Self::Claude => "Claude",
        }
    }

    /// The applicator implementing this harness's planning behavior.
    pub(crate) fn applicator(self) -> Box<dyn ProfileApplicator> {
        match self {
            Self::Copilot => Box::new(copilot::CopilotApplicator),
            Self::Claude => Box::new(claude::ClaudeApplicator),
        }
    }

    /// Repo-local root under which decomposed plugin skill directories are placed
    /// (Claude `<repo>/.claude/skills`, Copilot `<repo>/.agents/skills`).
    pub(crate) fn skills_root(self, repo_target: &Path) -> PathBuf {
        match self {
            Self::Claude => repo_target.join(".claude").join("skills"),
            Self::Copilot => repo_target.join(".agents").join("skills"),
        }
    }

    /// Repo-local root for decomposed plugin agent files (Claude
    /// `<repo>/.claude/agents`, Copilot `<repo>/.github/agents`).
    pub(crate) fn agents_root(self, repo_target: &Path) -> PathBuf {
        match self {
            Self::Claude => repo_target.join(".claude").join("agents"),
            Self::Copilot => repo_target.join(".github").join("agents"),
        }
    }

    /// The single shared managed-region file this harness writes instructions
    /// into, if any (`<repo>/AGENTS.md` for Copilot; Claude has none).
    pub(crate) fn managed_region_path(self, repo_target: &Path) -> Option<PathBuf> {
        match self {
            Self::Copilot => Some(repo_target.join("AGENTS.md")),
            Self::Claude => None,
        }
    }

    /// Repo-local JSON files this harness is allowed to merge into and restore.
    ///
    /// Plugin MCP servers and Claude delegate settings are decomposed into these
    /// exact paths; nothing else may be touched.
    pub(crate) fn removable_json_targets(self, repo_target: &Path) -> Vec<PathBuf> {
        match self {
            Self::Claude => vec![
                repo_target.join(".mcp.json"),
                repo_target.join(".claude").join("settings.json"),
                repo_target.join(".claude").join("settings.local.json"),
            ],
            Self::Copilot => vec![repo_target.join(".mcp.json")],
        }
    }

    /// Suffix appended to the user's home directory for this harness's config.
    fn home_suffix(self) -> PathBuf {
        match self {
            Self::Claude => PathBuf::from(".claude"),
            Self::Copilot => Path::new(".config").join("github-copilot"),
        }
    }

    /// Environment variable overriding this harness's config home directory.
    const fn home_env_var(self) -> &'static str {
        match self {
            Self::Claude => "REPOVERLAY_CLAUDE_HOME",
            Self::Copilot => "REPOVERLAY_COPILOT_HOME",
        }
    }

    /// Resolve this harness's config home, honoring the `REPOVERLAY_*_HOME`
    /// override and otherwise appending [`home_suffix`](Self::home_suffix) to the
    /// user's home directory.
    pub(crate) fn home_from_env(self) -> Result<PathBuf> {
        self.home_from_override(std::env::var_os(self.home_env_var()))
    }

    /// Resolve this harness's config home from an explicit override (used by
    /// tests and [`home_from_env`](Self::home_from_env)).
    fn home_from_override(self, override_home: Option<OsString>) -> Result<PathBuf> {
        if let Some(home) = override_home {
            return Ok(home.into());
        }
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(self.home_suffix()))
    }

    /// Environment variable overriding the launch program name.
    const fn command_env_var(self) -> &'static str {
        match self {
            Self::Claude => "REPOVERLAY_CLAUDE_COMMAND",
            Self::Copilot => "REPOVERLAY_COPILOT_COMMAND",
        }
    }

    /// The program to launch for this harness, honoring `REPOVERLAY_*_COMMAND`
    /// and otherwise falling back to the harness identifier.
    pub(crate) fn program(self) -> String {
        std::env::var(self.command_env_var()).unwrap_or_else(|_| self.as_str().to_string())
    }
}

impl fmt::Display for AgentHarness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AgentHarness {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "copilot" => Ok(Self::Copilot),
            "claude" => Ok(Self::Claude),
            other => bail!("Unsupported harness '{other}'"),
        }
    }
}

/// Reject an instruction source that escapes the profile asset directory.
///
/// Shared by every applicator: the source must be relative and must not contain
/// `..`, a root, or a path prefix, so it can only ever resolve inside the
/// profile's own asset tree.
fn validate_instruction_source(source: &Path) -> Result<()> {
    for component in source.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "Instruction source '{}' must stay within the profile asset directory",
                    source.display()
                );
            }
        }
    }
    if source.is_absolute() {
        anyhow::bail!("Instruction source '{}' must be relative", source.display());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileContext {
    pub(crate) profile_name: String,
    pub(crate) harness: AgentHarness,
    pub(crate) target: PathBuf,
    pub(crate) profile_asset_dir: PathBuf,
    pub(crate) harness_home: PathBuf,
    pub(crate) mode: ProfileMode,
    pub(crate) session_id: Option<String>,
    /// Marketplace registry used to resolve `marketplace/plugin` references.
    pub(crate) marketplaces: Vec<Marketplace>,
    /// Cache used to resolve/clone plugin and marketplace git sources.
    pub(crate) cache: crate::cache::CacheManager,
}

pub(crate) trait ProfileApplicator {
    fn plan(&self, profile: &ProfileConfig, context: &ProfileContext) -> Result<ProfilePlan>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_home_defaults_to_dot_claude() {
        let expected = dirs::home_dir().unwrap().join(".claude");
        assert_eq!(
            AgentHarness::Claude.home_from_override(None).unwrap(),
            expected
        );
    }

    #[test]
    fn copilot_home_defaults_to_config_github_copilot() {
        let expected = dirs::home_dir()
            .unwrap()
            .join(".config")
            .join("github-copilot");
        assert_eq!(
            AgentHarness::Copilot.home_from_override(None).unwrap(),
            expected
        );
    }

    #[test]
    fn harness_home_honors_override() {
        let temp = tempfile::TempDir::new().unwrap();
        assert_eq!(
            AgentHarness::Claude
                .home_from_override(Some(temp.path().into()))
                .unwrap(),
            temp.path()
        );
    }

    #[test]
    fn harness_round_trips_through_str() {
        for harness in [AgentHarness::Claude, AgentHarness::Copilot] {
            assert_eq!(harness.as_str().parse::<AgentHarness>().unwrap(), harness);
        }
    }
}
