#![allow(dead_code)]

use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::plugin::{InstallMode, PluginBundle, PluginRef, ResolvedPlugin, resolve_plugin};
use crate::profile::ProfileConfig;
use crate::profile_applicators::{AgentHarness, ProfileApplicator, ProfileContext};
use crate::profile_plan::{ProfileAction, ProfilePlan, json_pointer};

pub(crate) struct CopilotApplicator;

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

impl CopilotApplicator {
    fn harness_home_from_override(override_home: Option<std::ffi::OsString>) -> Result<PathBuf> {
        if let Some(home) = override_home {
            return Ok(home.into());
        }
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".config").join("github-copilot"))
    }

    pub(crate) fn harness_home_from_env() -> Result<PathBuf> {
        Self::harness_home_from_override(std::env::var_os("REPOVERLAY_COPILOT_HOME"))
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

    /// Decompose a resolved plugin bundle into Copilot placements, appending
    /// skill-placement and skip actions and accumulating MCP servers for the
    /// single Copilot `mcp.json` merge (Copilot keys servers under `servers`).
    fn decompose_bundle(
        bundle_dir: &Path,
        plugin_name: &str,
        actions: &mut Vec<ProfileAction>,
        servers: &mut serde_json::Map<String, serde_json::Value>,
        owned_paths: &mut Vec<String>,
        repo_target: &Path,
    ) -> Result<()> {
        let bundle = PluginBundle::read(bundle_dir)?;

        for skill in &bundle.skills {
            actions.push(ProfileAction::PlacePluginDir {
                source: bundle_dir.join("skills").join(skill),
                target: repo_target.join(".agents").join("skills").join(skill),
            });
        }

        for (server_name, server) in &bundle.mcp_servers {
            let pointer = json_pointer(&["servers", server_name]);
            if owned_paths.contains(&pointer) {
                anyhow::bail!(
                    "MCP server '{server_name}' is provided by more than one plugin; \
                     resolve the conflict before applying"
                );
            }
            let resolved = crate::plugin::substitute_plugin_root(server, bundle_dir)?;
            servers.insert(server_name.clone(), resolved);
            owned_paths.push(pointer);
        }

        for capability in &bundle.unsupported_capabilities {
            actions.push(ProfileAction::SkipCapability {
                capability: format!("plugin:{plugin_name}:{capability}"),
                reason: format!(
                    "Copilot does not support '{capability}' from plugin '{plugin_name}'"
                ),
            });
        }

        Ok(())
    }
}

impl ProfileApplicator for CopilotApplicator {
    fn harness(&self) -> AgentHarness {
        AgentHarness::Copilot
    }

    fn plan(&self, profile: &ProfileConfig, context: &ProfileContext) -> Result<ProfilePlan> {
        let mut actions = Vec::new();

        for overlay in &profile.overlays {
            actions.push(ProfileAction::ApplyOverlay {
                reference: overlay.clone(),
            });
        }

        let mut instruction_sources = Vec::new();
        for instruction in &profile.instructions {
            let source_rel = Path::new(&instruction.source);
            validate_instruction_source(source_rel)?;
            instruction_sources.push(context.profile_asset_dir.join(source_rel));
        }
        if !instruction_sources.is_empty() {
            actions.push(ProfileAction::WriteManagedRegion {
                sources: instruction_sources,
                target: context.target.join("AGENTS.md"),
                marker_id: context.profile_name.clone(),
            });
        }

        // Plugin introspection: resolve each plugin, decompose its bundle into
        // Copilot's native `mcp.json` servers + skills, and skip what cannot map.
        let mut servers = serde_json::Map::new();
        let mut owned_paths = Vec::new();
        let mut plugins = Vec::new();

        for plugin in &profile.plugins {
            if let PluginRef::Marketplace {
                install: InstallMode::Delegate,
                name,
                ..
            } = plugin
            {
                actions.push(ProfileAction::SkipCapability {
                    capability: format!("plugin:{name}:delegate"),
                    reason: "delegate plugins are a Claude-only feature; not applied for Copilot"
                        .to_string(),
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
                        &mut servers,
                        &mut owned_paths,
                        &context.target,
                    )?;
                }
                ResolvedPlugin::Delegate { name, .. } => {
                    actions.push(ProfileAction::SkipCapability {
                        capability: format!("plugin:{name}:delegate"),
                        reason: "plugin source cannot be cached/introspected for Copilot"
                            .to_string(),
                    });
                }
            }
        }

        if !servers.is_empty() {
            actions.push(ProfileAction::MergeJson {
                target: context.target.join(".mcp.json"),
                value: serde_json::json!({ "servers": servers }),
                owned_paths,
            });
        }

        Ok(ProfilePlan {
            profile_name: context.profile_name.clone(),
            harness: "copilot".to_string(),
            actions,
            plugins,
        })
    }

    fn command(&self, context: &ProfileContext, extra_args: &[String]) -> Result<Command> {
        let program =
            std::env::var("REPOVERLAY_COPILOT_COMMAND").unwrap_or_else(|_| "copilot".to_string());
        self.command_with_program(context, &program, extra_args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{InstructionConfig, ProfileConfig};
    use crate::profile_applicators::{ProfileApplicator, ProfileContext};

    #[test]
    fn copilot_introspects_plugin_mcp_into_servers_merge() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("repo");
        std::fs::create_dir_all(&target).unwrap();
        let bundle = target.join("rust-plugin");
        std::fs::create_dir_all(bundle.join(".claude-plugin")).unwrap();
        std::fs::write(
            bundle.join(".claude-plugin/plugin.json"),
            r#"{"name":"rust"}"#,
        )
        .unwrap();
        std::fs::write(
            bundle.join(".mcp.json"),
            r#"{"mcpServers":{"rust":{"command":"uvx","args":["mcp-rust"]}}}"#,
        )
        .unwrap();

        let profile = ProfileConfig {
            plugins: vec![PluginRef::Local {
                source: PathBuf::from("./rust-plugin"),
            }],
            ..ProfileConfig::default()
        };
        let context = ProfileContext {
            profile_name: "rust-dev".to_string(),
            target: target.clone(),
            profile_asset_dir: target,
            harness_home: temp.path().join("copilot-home"),
            mode: crate::profile::ProfileMode::Persistent,
            session_id: None,
            marketplaces: Vec::new(),
            cache: crate::cache::CacheManager::new().unwrap(),
        };

        let plan = CopilotApplicator.plan(&profile, &context).unwrap();
        let (mtarget, value, owned) = plan
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
            .expect("expected an mcp.json merge action");
        assert!(mtarget.ends_with(".mcp.json"));
        assert_eq!(owned, &vec!["/servers/rust".to_string()]);
        assert_eq!(value["servers"]["rust"]["command"], "uvx");
    }

    #[test]
    fn copilot_plans_instruction_write() {
        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("profile-assets");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("copilot-instructions.md"), "Be concise.").unwrap();

        let profile = ProfileConfig {
            instructions: vec![InstructionConfig {
                source: "copilot-instructions.md".to_string(),
            }],
            ..ProfileConfig::default()
        };
        let context = ProfileContext {
            profile_name: "rust-dev".to_string(),
            target: temp.path().to_path_buf(),
            profile_asset_dir: source_dir,
            harness_home: temp.path().join("copilot-home"),
            mode: crate::profile::ProfileMode::Persistent,
            session_id: None,
            marketplaces: Vec::new(),
            cache: crate::cache::CacheManager::new().unwrap(),
        };

        let plan = CopilotApplicator.plan(&profile, &context).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            crate::profile_plan::ProfileAction::WriteManagedRegion { target, marker_id, .. }
                if target.ends_with("AGENTS.md") && marker_id == "rust-dev"
        )));
    }

    #[test]
    fn copilot_rejects_unsafe_instruction_sources() {
        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("profile-assets");
        let context = ProfileContext {
            profile_name: "rust-dev".to_string(),
            target: temp.path().to_path_buf(),
            profile_asset_dir: source_dir,
            harness_home: temp.path().join("copilot-home"),
            mode: crate::profile::ProfileMode::Persistent,
            session_id: None,
            marketplaces: Vec::new(),
            cache: crate::cache::CacheManager::new().unwrap(),
        };

        let unsafe_sources = vec![
            "../secret.md".to_string(),
            temp.path().join("secret.md").display().to_string(),
        ];

        for source in unsafe_sources {
            let profile = ProfileConfig {
                instructions: vec![InstructionConfig {
                    source: source.clone(),
                }],
                ..ProfileConfig::default()
            };

            let err = CopilotApplicator.plan(&profile, &context).unwrap_err();
            assert!(
                err.to_string().contains("Instruction source"),
                "unexpected error for {source}: {err}"
            );
        }
    }

    #[test]
    fn copilot_harness_home_uses_override_when_provided() {
        let temp = tempfile::TempDir::new().unwrap();

        assert_eq!(
            CopilotApplicator::harness_home_from_override(Some(temp.path().into())).unwrap(),
            temp.path()
        );
    }

    #[test]
    fn copilot_harness_home_defaults_to_github_copilot_config_dir() {
        let expected = dirs::home_dir().unwrap().join(".config/github-copilot");

        assert_eq!(
            CopilotApplicator::harness_home_from_override(None).unwrap(),
            expected
        );
    }

    #[test]
    fn copilot_command_uses_env_override_and_extra_args() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = ProfileContext {
            profile_name: "rust-dev".to_string(),
            target: temp.path().to_path_buf(),
            profile_asset_dir: temp.path().to_path_buf(),
            harness_home: temp.path().join("copilot-home"),
            mode: crate::profile::ProfileMode::Ephemeral,
            session_id: Some("session-1".to_string()),
            marketplaces: Vec::new(),
            cache: crate::cache::CacheManager::new().unwrap(),
        };

        let command = CopilotApplicator
            .command_with_program(&context, "echo", &["hello".to_string()])
            .unwrap();
        assert_eq!(command.get_program(), "echo");
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec!["hello"]
        );
    }
}
