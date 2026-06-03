#![allow(dead_code)]

use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::plugin::{InstallMode, PluginBundle, PluginRef, ResolvedPlugin, resolve_plugin};
use crate::profile::{ProfileConfig, ProfileScope};
use crate::profile_applicators::{AgentHarness, ProfileApplicator, ProfileContext};
use crate::profile_plan::{ProfileAction, ProfilePlan, json_pointer};

pub(crate) struct ClaudeApplicator;

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

impl ClaudeApplicator {
    fn harness_home_from_override(override_home: Option<std::ffi::OsString>) -> Result<PathBuf> {
        if let Some(home) = override_home {
            return Ok(home.into());
        }
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".claude"))
    }

    pub(crate) fn harness_home_from_env() -> Result<PathBuf> {
        Self::harness_home_from_override(std::env::var_os("REPOVERLAY_CLAUDE_HOME"))
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub(crate) fn command_with_program(
        &self,
        _context: &ProfileContext,
        program: &str,
        extra_args: &[String],
    ) -> Result<Command> {
        let mut command = Command::new(program);
        command.args(extra_args);
        Ok(command)
    }

    /// Decompose a resolved plugin bundle into native Claude placements,
    /// appending skill-placement and skip actions and accumulating MCP servers
    /// for the single project `.mcp.json` merge.
    fn decompose_bundle(
        bundle_dir: &Path,
        plugin_name: &str,
        actions: &mut Vec<ProfileAction>,
        mcp_servers: &mut serde_json::Map<String, serde_json::Value>,
        owned_paths: &mut Vec<String>,
        harness_home: &Path,
    ) -> Result<()> {
        let bundle = PluginBundle::read(bundle_dir)?;

        for skill in &bundle.skills {
            actions.push(ProfileAction::PlacePluginDir {
                source: bundle_dir.join("skills").join(skill),
                target: harness_home.join("skills").join(skill),
                scope: ProfileScope::User,
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
                reason: format!(
                    "Claude '{capability}' from plugin '{plugin_name}' cannot be decomposed \
                     into a persistent placement; use an ephemeral session for full plugin load"
                ),
            });
        }

        Ok(())
    }
}

impl ProfileApplicator for ClaudeApplicator {
    fn harness(&self) -> AgentHarness {
        AgentHarness::Claude
    }

    fn plan(&self, profile: &ProfileConfig, context: &ProfileContext) -> Result<ProfilePlan> {
        let mut actions = Vec::new();

        for overlay in &profile.overlays {
            actions.push(ProfileAction::ApplyOverlay {
                reference: overlay.clone(),
            });
        }

        // Claude has no persistent profile-instruction convention yet; record
        // each instruction as skipped rather than dropping it silently.
        for instruction in &profile.instructions {
            let source_rel = Path::new(&instruction.source);
            validate_instruction_source(source_rel)?;
            actions.push(ProfileAction::SkipCapability {
                capability: format!("instruction:{}", instruction.source),
                reason: "Claude persistent instruction placement is not implemented yet"
                    .to_string(),
            });
        }

        let mut mcp_servers = serde_json::Map::new();
        let mut owned_paths = Vec::new();
        let mut plugins = Vec::new();

        for plugin in &profile.plugins {
            // Delegate plugins are enabled through Claude settings (Task 8); do
            // not resolve or cache them here.
            if let PluginRef::Marketplace {
                install: InstallMode::Delegate,
                name,
                ..
            } = plugin
            {
                actions.push(ProfileAction::SkipCapability {
                    capability: format!("plugin:{name}:delegate"),
                    reason: "delegate plugin enablement is deferred".to_string(),
                });
                continue;
            }

            let resolved = resolve_plugin(
                plugin,
                &context.marketplaces,
                &context.cache,
                &context.target,
                false,
            )?;

            match resolved {
                ResolvedPlugin::Bundle {
                    name,
                    bundle_dir,
                    resolved_commit,
                    ..
                } => {
                    plugins.push(crate::profile_plan::PluginProvenance {
                        reference: plugin.to_string(),
                        resolved_commit,
                    });
                    Self::decompose_bundle(
                        &bundle_dir,
                        &name,
                        &mut actions,
                        &mut mcp_servers,
                        &mut owned_paths,
                        &context.harness_home,
                    )?;
                }
                ResolvedPlugin::Delegate { name, .. } => {
                    actions.push(ProfileAction::SkipCapability {
                        capability: format!("plugin:{name}:delegate"),
                        reason: "plugin source cannot be cached/introspected; enablement is \
                                 deferred to the harness"
                            .to_string(),
                    });
                }
            }
        }

        if !mcp_servers.is_empty() {
            actions.push(ProfileAction::MergeJson {
                target: context.target.join(".mcp.json"),
                value: serde_json::json!({ "mcpServers": mcp_servers }),
                scope: ProfileScope::Repo,
                owned_paths,
            });
        }

        Ok(ProfilePlan {
            profile_name: context.profile_name.clone(),
            harness: "claude".to_string(),
            actions,
            plugins,
        })
    }

    fn command(&self, context: &ProfileContext, extra_args: &[String]) -> Result<Command> {
        let program =
            std::env::var("REPOVERLAY_CLAUDE_COMMAND").unwrap_or_else(|_| "claude".to_string());
        self.command_with_program(context, &program, extra_args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheManager;
    use crate::config::Marketplace;
    use crate::plugin::PluginRef;
    use std::fs;

    fn local_bundle(dir: &Path, name: &str) -> PathBuf {
        let bundle = dir.join(name);
        fs::create_dir_all(bundle.join(".claude-plugin")).unwrap();
        fs::write(
            bundle.join(".claude-plugin/plugin.json"),
            format!(r#"{{"name":"{name}"}}"#),
        )
        .unwrap();
        bundle
    }

    fn context_for(target: &Path, harness_home: &Path) -> ProfileContext {
        ProfileContext {
            profile_name: "rust-dev".to_string(),
            target: target.to_path_buf(),
            profile_asset_dir: target.to_path_buf(),
            harness_home: harness_home.to_path_buf(),
            mode: crate::profile::ProfileMode::Persistent,
            session_id: None,
            marketplaces: Vec::<Marketplace>::new(),
            cache: CacheManager::new().unwrap(),
        }
    }

    #[test]
    fn claude_home_defaults_to_dot_claude() {
        let expected = dirs::home_dir().unwrap().join(".claude");
        assert_eq!(
            ClaudeApplicator::harness_home_from_override(None).unwrap(),
            expected
        );
    }

    #[test]
    fn claude_home_honors_override() {
        let temp = tempfile::TempDir::new().unwrap();
        assert_eq!(
            ClaudeApplicator::harness_home_from_override(Some(temp.path().into())).unwrap(),
            temp.path()
        );
    }

    #[test]
    fn claude_command_uses_env_program_name() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = context_for(temp.path(), &temp.path().join("claude-home"));
        let command = ClaudeApplicator
            .command_with_program(&context, "echo", &["hi".to_string()])
            .unwrap();
        assert_eq!(command.get_program(), "echo");
    }

    #[test]
    fn claude_decomposes_local_plugin_into_skills_and_mcp() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("repo");
        fs::create_dir_all(&target).unwrap();
        let claude_home = temp.path().join("claude-home");

        let bundle = local_bundle(&target, "rust-dev");
        fs::create_dir_all(bundle.join("skills/fmt")).unwrap();
        fs::write(bundle.join("skills/fmt/SKILL.md"), "# fmt").unwrap();
        fs::write(
            bundle.join(".mcp.json"),
            r#"{"mcpServers":{"rust":{"command":"${CLAUDE_PLUGIN_ROOT}/bin/server"}}}"#,
        )
        .unwrap();

        let profile = ProfileConfig {
            plugins: vec![PluginRef::Local {
                source: PathBuf::from("./rust-dev"),
            }],
            ..ProfileConfig::default()
        };
        let context = context_for(&target, &claude_home);

        let plan = ClaudeApplicator.plan(&profile, &context).unwrap();

        // Skill placement targets the Claude skills dir.
        assert!(plan.actions.iter().any(|a| matches!(
            a,
            ProfileAction::PlacePluginDir { target, .. }
                if target.ends_with("skills/fmt")
        )));

        // MCP merge into the project `.mcp.json` with the plugin root resolved.
        let merge = plan
            .actions
            .iter()
            .find_map(|a| match a {
                ProfileAction::MergeJson {
                    target,
                    value,
                    owned_paths,
                    ..
                } => Some((target, value, owned_paths)),
                _ => None,
            })
            .expect("expected an mcp merge action");
        assert!(merge.0.ends_with(".mcp.json"));
        assert_eq!(merge.2, &vec!["/mcpServers/rust".to_string()]);
        let resolved_cmd = merge.1["mcpServers"]["rust"]["command"].as_str().unwrap();
        assert!(!resolved_cmd.contains("${CLAUDE_PLUGIN_ROOT}"));
        assert!(resolved_cmd.ends_with("/bin/server"));
    }

    #[test]
    fn claude_skips_unsupported_capabilities() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("repo");
        fs::create_dir_all(&target).unwrap();

        let bundle = local_bundle(&target, "hooky");
        fs::create_dir_all(bundle.join("hooks")).unwrap();
        fs::write(bundle.join("hooks/pre.sh"), "echo hi").unwrap();

        let profile = ProfileConfig {
            plugins: vec![PluginRef::Local {
                source: PathBuf::from("./hooky"),
            }],
            ..ProfileConfig::default()
        };
        let context = context_for(&target, &temp.path().join("claude-home"));

        let plan = ClaudeApplicator.plan(&profile, &context).unwrap();
        assert!(plan.actions.iter().any(|a| matches!(
            a,
            ProfileAction::SkipCapability { capability, .. }
                if capability == "plugin:hooky:hooks"
        )));
    }
}
