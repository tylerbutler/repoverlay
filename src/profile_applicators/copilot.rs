use anyhow::Result;
use std::path::Path;

use crate::plugin::{InstallMode, PluginBundle, PluginRef, ResolvedPlugin, resolve_plugin};
use crate::profile::ProfileConfig;
use crate::profile_applicators::{AgentHarness, ProfileApplicator, ProfileContext};
use crate::profile_plan::{ProfileAction, ProfilePlan, json_pointer};

pub(crate) struct CopilotApplicator;

impl CopilotApplicator {
    /// Decompose a resolved plugin bundle into Copilot placements, appending
    /// skill-placement and skip actions and accumulating MCP servers for the
    /// single Copilot `.mcp.json` merge (Copilot CLI keys servers under
    /// `mcpServers`, matching `~/.copilot/mcp-config.json`).
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
                target: AgentHarness::Copilot.skills_root(repo_target).join(skill),
            });
        }

        for agent in &bundle.agents {
            actions.push(ProfileAction::PlacePluginDir {
                source: bundle_dir.join("agents").join(agent),
                target: AgentHarness::Copilot.agents_root(repo_target).join(agent),
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
    fn plan(&self, profile: &ProfileConfig, context: &ProfileContext) -> Result<ProfilePlan> {
        let mut actions = Vec::new();

        for overlay in &profile.overlays {
            actions.push(ProfileAction::ApplyOverlay {
                reference: overlay.clone(),
            });
        }

        if let Some(action) = super::plan_instruction_region(profile, context)? {
            actions.push(action);
        }

        // Plugin introspection: resolve each plugin, decompose its bundle into
        // Copilot's native `mcp.json` `mcpServers` + skills, and skip what
        // cannot map.
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
                value: serde_json::json!({ "mcpServers": servers }),
                owned_paths,
            });
        }

        Ok(ProfilePlan {
            profile_name: context.profile_name.clone(),
            harness: context.harness,
            actions,
            plugins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{InstructionConfig, ProfileConfig};
    use crate::profile_applicators::{ProfileApplicator, ProfileContext};
    use std::path::PathBuf;

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
            harness: AgentHarness::Copilot,
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
        assert_eq!(owned, &vec!["/mcpServers/rust".to_string()]);
        assert_eq!(value["mcpServers"]["rust"]["command"], "uvx");
    }

    #[test]
    fn copilot_places_plugin_agents_into_github_agents() {
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
        std::fs::create_dir_all(bundle.join("agents")).unwrap();
        std::fs::write(
            bundle.join("agents/reviewer.agent.md"),
            "---\nname: reviewer\n---\n",
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
            harness: AgentHarness::Copilot,
            target: target.clone(),
            profile_asset_dir: target.clone(),
            harness_home: temp.path().join("copilot-home"),
            mode: crate::profile::ProfileMode::Persistent,
            session_id: None,
            marketplaces: Vec::new(),
            cache: crate::cache::CacheManager::new().unwrap(),
        };

        let plan = CopilotApplicator.plan(&profile, &context).unwrap();
        let placed = plan
            .actions
            .iter()
            .find_map(|a| match a {
                ProfileAction::PlacePluginDir { source, target } => Some((source, target)),
                _ => None,
            })
            .expect("expected an agent placement action");
        assert!(placed.0.ends_with("agents/reviewer.agent.md"));
        assert_eq!(
            placed.1,
            &target
                .join(".github")
                .join("agents")
                .join("reviewer.agent.md")
        );
    }

    #[test]
    fn copilot_plans_instruction_write() {
        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("profile-assets");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("copilot-instructions.md"), "Be concise.").unwrap();

        let profile = ProfileConfig {
            instructions: vec![InstructionConfig {
                source: Some("copilot-instructions.md".to_string()),
                content: None,
                base_dir: None,
            }],
            ..ProfileConfig::default()
        };
        let context = ProfileContext {
            profile_name: "rust-dev".to_string(),
            harness: AgentHarness::Copilot,
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
            harness: AgentHarness::Copilot,
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
                    source: Some(source.clone()),
                    content: None,
                    base_dir: None,
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
}
