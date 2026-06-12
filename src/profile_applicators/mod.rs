//! Profile applicators provide profile-specific integration behavior.

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
use crate::plugin::PluginBundle;
use crate::profile::{ProfileConfig, ProfileMode};
use crate::profile_plan::{ProfileAction, ProfilePlan, json_pointer};

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
    /// into (`<repo>/AGENTS.md` for Copilot, `<repo>/CLAUDE.md` for Claude).
    pub(crate) fn managed_region_path(self, repo_target: &Path) -> PathBuf {
        match self {
            Self::Copilot => repo_target.join("AGENTS.md"),
            Self::Claude => repo_target.join("CLAUDE.md"),
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

/// Build the `WriteManagedRegion` action for a profile's instructions, if it
/// has any.
///
/// Shared by every applicator: file `source` entries resolve against the
/// entry's `base_dir` (the directory of the config file that defined it,
/// falling back to the profile asset dir) and the region targets the harness's
/// [`AgentHarness::managed_region_path`], keyed by the profile name.
fn plan_instruction_region(
    profile: &ProfileConfig,
    context: &ProfileContext,
) -> Result<Option<crate::profile_plan::ProfileAction>> {
    let mut bodies = Vec::new();
    for instruction in &profile.instructions {
        instruction.validate_exactly_one()?;
        if let Some(source) = &instruction.source {
            let source_rel = Path::new(source);
            validate_instruction_source(source_rel)?;
            let base_dir = instruction
                .base_dir
                .clone()
                .unwrap_or_else(|| context.profile_asset_dir.clone());
            bodies.push(crate::profile_plan::InstructionBody::File {
                path: base_dir.join(source_rel),
                base_dir,
            });
        } else if let Some(content) = instruction.normalized_content() {
            bodies.push(crate::profile_plan::InstructionBody::Inline(content));
        }
    }
    if bodies.is_empty() {
        return Ok(None);
    }
    let target = context.harness.managed_region_path(&context.target);
    Ok(Some(
        crate::profile_plan::ProfileAction::WriteManagedRegion {
            bodies,
            target,
            marker_id: context.profile_name.clone(),
        },
    ))
}

/// Decompose a resolved plugin bundle into repo-local placements for the
/// context's harness, appending skill/agent placement and skip actions and
/// accumulating MCP servers for the harness's single project `.mcp.json`
/// merge.
///
/// Shared by every applicator: placement roots come from
/// [`AgentHarness::skills_root`]/[`AgentHarness::agents_root`], and
/// `skip_reason` renders the harness-specific reason for a capability that
/// cannot be decomposed.
fn decompose_bundle(
    context: &ProfileContext,
    bundle_dir: &Path,
    plugin_name: &str,
    actions: &mut Vec<ProfileAction>,
    mcp_servers: &mut serde_json::Map<String, serde_json::Value>,
    owned_paths: &mut Vec<String>,
    skip_reason: impl Fn(&str) -> String,
) -> Result<()> {
    let bundle = PluginBundle::read(bundle_dir)?;
    let harness = context.harness;
    let repo_target = &context.target;

    for skill in &bundle.skills {
        actions.push(ProfileAction::PlacePluginDir {
            source: bundle_dir.join("skills").join(skill),
            target: harness.skills_root(repo_target).join(skill),
        });
    }

    for agent in &bundle.agents {
        actions.push(ProfileAction::PlacePluginDir {
            source: bundle_dir.join("agents").join(agent),
            target: harness.agents_root(repo_target).join(agent),
        });
    }

    for (server_name, server) in &bundle.mcp_servers {
        let pointer = json_pointer(&["mcpServers", server_name]);
        if owned_paths.contains(&pointer) {
            anyhow::bail!(
                "MCP server '{server_name}' is provided by more than one plugin; \
                 resolve the conflict before applying"
            );
        }
        let resolved = crate::plugin::substitute_plugin_root(server, bundle_dir)?;
        mcp_servers.insert(server_name.clone(), resolved);
        owned_paths.push(pointer);
    }

    for capability in &bundle.unsupported_capabilities {
        actions.push(ProfileAction::SkipCapability {
            capability: format!("plugin:{plugin_name}:{capability}"),
            reason: skip_reason(capability),
        });
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileContext {
    pub(crate) profile_name: String,
    pub(crate) harness: AgentHarness,
    pub(crate) target: PathBuf,
    pub(crate) profile_asset_dir: PathBuf,
    /// Resolved harness config home (honors `REPOVERLAY_*_HOME`). No applicator
    /// reads it yet, but it is the sandboxed conduit for future actions that
    /// place files in the harness home, and integration tests already rely on
    /// the override env vars.
    #[allow(dead_code)]
    pub(crate) harness_home: PathBuf,
    pub(crate) mode: ProfileMode,
    /// Ephemeral session identifier, persisted in `ProfileState`; available to
    /// applicators that need to brand session-scoped placements.
    #[allow(dead_code)]
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
    use crate::profile_plan::ProfileAction;
    use std::fs;

    /// Both applicators must decompose the same plugin bundle into structurally
    /// identical plans: the same placements (relative to each harness's
    /// skills/agents roots), the same MCP-server merge, and the same skipped
    /// capabilities (the human-readable skip reason may differ).
    #[test]
    fn applicators_decompose_identical_bundles_equivalently() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("repo");
        fs::create_dir_all(&target).unwrap();

        let bundle = target.join("rust-dev");
        fs::create_dir_all(bundle.join(".claude-plugin")).unwrap();
        fs::write(
            bundle.join(".claude-plugin/plugin.json"),
            r#"{"name":"rust-dev"}"#,
        )
        .unwrap();
        fs::create_dir_all(bundle.join("skills/fmt")).unwrap();
        fs::write(bundle.join("skills/fmt/SKILL.md"), "# fmt").unwrap();
        fs::create_dir_all(bundle.join("agents")).unwrap();
        fs::write(
            bundle.join("agents/reviewer.md"),
            "---\nname: reviewer\n---\n",
        )
        .unwrap();
        fs::write(
            bundle.join(".mcp.json"),
            r#"{"mcpServers":{"rust":{"command":"${CLAUDE_PLUGIN_ROOT}/bin/server"}}}"#,
        )
        .unwrap();
        fs::create_dir_all(bundle.join("hooks")).unwrap();
        fs::write(bundle.join("hooks/pre.sh"), "echo hi").unwrap();

        let profile = ProfileConfig {
            plugins: vec![crate::plugin::PluginRef::Local {
                source: PathBuf::from("./rust-dev"),
            }],
            ..ProfileConfig::default()
        };

        let plan_for = |harness: AgentHarness| {
            let context = ProfileContext {
                profile_name: "rust-dev".to_string(),
                harness,
                target: target.clone(),
                profile_asset_dir: target.clone(),
                harness_home: temp.path().join("harness-home"),
                mode: ProfileMode::Persistent,
                session_id: None,
                marketplaces: Vec::new(),
                cache: crate::cache::CacheManager::new().unwrap(),
            };
            harness.applicator().plan(&profile, &context).unwrap()
        };

        let claude = plan_for(AgentHarness::Claude);
        let copilot = plan_for(AgentHarness::Copilot);

        // Placements: identical sources and identical targets relative to each
        // harness's placement roots.
        let placements = |plan: &crate::profile_plan::ProfilePlan, harness: AgentHarness| {
            plan.actions
                .iter()
                .filter_map(|a| match a {
                    ProfileAction::PlacePluginDir { source, target: t } => {
                        let rel = t
                            .strip_prefix(harness.skills_root(&target))
                            .or_else(|_| t.strip_prefix(harness.agents_root(&target)))
                            .expect("placement target must be under a harness root");
                        Some((source.clone(), rel.to_path_buf()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            placements(&claude, AgentHarness::Claude),
            placements(&copilot, AgentHarness::Copilot)
        );

        // MCP merge: identical resolved value and owned paths.
        let merge = |plan: &crate::profile_plan::ProfilePlan| {
            plan.actions
                .iter()
                .find_map(|a| match a {
                    ProfileAction::MergeJson {
                        value, owned_paths, ..
                    } => Some((value.clone(), owned_paths.clone())),
                    _ => None,
                })
                .expect("expected an mcp merge action")
        };
        assert_eq!(merge(&claude), merge(&copilot));

        // Skips: identical capability identifiers.
        let skips = |plan: &crate::profile_plan::ProfilePlan| {
            plan.actions
                .iter()
                .filter_map(|a| match a {
                    ProfileAction::SkipCapability { capability, .. } => Some(capability.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(skips(&claude), skips(&copilot));
        assert_eq!(skips(&claude), vec!["plugin:rust-dev:hooks".to_string()]);
    }

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
