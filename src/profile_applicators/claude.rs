#![allow(dead_code)]

use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::plugin::{InstallMode, PluginBundle, PluginRef, ResolvedPlugin, resolve_plugin};
use crate::profile::{DelegateScope, ProfileConfig, ProfileMode};
use crate::profile_applicators::{AgentHarness, ProfileApplicator, ProfileContext};
use crate::profile_plan::{ProfileAction, ProfilePlan, json_pointer};

pub(crate) struct ClaudeApplicator;

/// Accumulated `settings.json` entries for one delegate scope/file.
#[derive(Default)]
struct DelegateSettings {
    /// `"<plugin>@<marketplace>" = true` enablement keys.
    enabled: serde_json::Map<String, serde_json::Value>,
    /// `<marketplace>` registration entries.
    marketplaces: serde_json::Map<String, serde_json::Value>,
    /// RFC 6901 pointers this merge owns (for conflict-aware removal).
    owned_paths: Vec<String>,
}

impl ClaudeApplicator {
    /// Default delegate settings scope when a plugin does not set one.
    ///
    /// Persistent profiles default to `project` (team-shareable, committed);
    /// ephemeral sessions default to `local` so a transient session never
    /// dirties the tracked `.claude/settings.json`.
    const fn default_delegate_scope(mode: ProfileMode) -> DelegateScope {
        match mode {
            ProfileMode::Persistent => DelegateScope::Project,
            ProfileMode::Ephemeral => DelegateScope::Local,
        }
    }

    /// Resolve a delegate scope to its settings file.
    fn delegate_settings_target(scope: DelegateScope, context: &ProfileContext) -> PathBuf {
        match scope {
            DelegateScope::Project => context.target.join(".claude").join("settings.json"),
            DelegateScope::Local => context.target.join(".claude").join("settings.local.json"),
        }
    }

    /// Accumulate a delegate plugin's `enabledPlugins` + `extraKnownMarketplaces`
    /// entries into the per-scope settings map, deduplicating a shared
    /// marketplace and rejecting a marketplace name that maps to two sources.
    fn accumulate_delegate(
        delegates: &mut BTreeMap<PathBuf, DelegateSettings>,
        marketplace: &str,
        name: &str,
        scope: Option<DelegateScope>,
        context: &ProfileContext,
    ) -> Result<()> {
        let url = context
            .marketplaces
            .iter()
            .find(|m| m.name == marketplace)
            .and_then(|m| m.url.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "delegate plugin '{name}' references marketplace '{marketplace}' \
                     with no registered URL"
                )
            })?;

        let scope = scope.unwrap_or_else(|| Self::default_delegate_scope(context.mode));
        let settings_path = Self::delegate_settings_target(scope, context);
        let entry = delegates.entry(settings_path).or_default();

        let enabled_key = format!("{name}@{marketplace}");
        let enabled_pointer = json_pointer(&["enabledPlugins", &enabled_key]);
        if entry
            .enabled
            .insert(enabled_key, serde_json::Value::Bool(true))
            .is_none()
        {
            entry.owned_paths.push(enabled_pointer);
        }

        let marketplace_value = serde_json::json!({
            "source": { "source": "git", "url": url }
        });
        let marketplace_pointer = json_pointer(&["extraKnownMarketplaces", marketplace]);
        match entry.marketplaces.get(marketplace) {
            Some(existing) if *existing != marketplace_value => {
                anyhow::bail!(
                    "marketplace '{marketplace}' maps to conflicting sources within the \
                     profile; resolve the conflict before applying"
                );
            }
            Some(_) => {}
            None => {
                entry
                    .marketplaces
                    .insert(marketplace.to_string(), marketplace_value);
                entry.owned_paths.push(marketplace_pointer);
            }
        }
        Ok(())
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
        repo_target: &Path,
    ) -> Result<()> {
        let bundle = PluginBundle::read(bundle_dir)?;

        for skill in &bundle.skills {
            actions.push(ProfileAction::PlacePluginDir {
                source: bundle_dir.join("skills").join(skill),
                target: AgentHarness::Claude.skills_root(repo_target).join(skill),
            });
        }

        for agent in &bundle.agents {
            actions.push(ProfileAction::PlacePluginDir {
                source: bundle_dir.join("agents").join(agent),
                target: AgentHarness::Claude.agents_root(repo_target).join(agent),
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
                     into a repo-local placement"
                ),
            });
        }

        Ok(())
    }
}

impl ProfileApplicator for ClaudeApplicator {
    fn plan(&self, profile: &ProfileConfig, context: &ProfileContext) -> Result<ProfilePlan> {
        let mut actions = Vec::new();

        for overlay in &profile.overlays {
            actions.push(ProfileAction::ApplyOverlay {
                reference: overlay.clone(),
            });
        }

        // Instructions land in a profile-keyed managed region of CLAUDE.md,
        // mirroring Copilot's AGENTS.md region.
        if let Some(action) = super::plan_instruction_region(profile, context)? {
            actions.push(action);
        }

        let mut mcp_servers = serde_json::Map::new();
        let mut owned_paths = Vec::new();
        let mut plugins = Vec::new();
        let mut delegates: BTreeMap<PathBuf, DelegateSettings> = BTreeMap::new();

        for plugin in &profile.plugins {
            // Delegate plugins are enabled through Claude settings rather than
            // cached/introspected; do not resolve them.
            if let PluginRef::Marketplace {
                install: InstallMode::Delegate,
                marketplace,
                name,
                scope,
                ..
            } = plugin
            {
                Self::accumulate_delegate(&mut delegates, marketplace, name, *scope, context)?;
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
                    // Decompose the bundle into repo-local skill/agent/MCP
                    // placements for both persistent and ephemeral applies, so
                    // Claude and Copilot behave identically. (Ephemeral
                    // placements are rolled back when the session ends.)
                    Self::decompose_bundle(
                        &bundle_dir,
                        &name,
                        &mut actions,
                        &mut mcp_servers,
                        &mut owned_paths,
                        &context.target,
                    )?;
                }
                // The source could not be cached/introspected, so fall back to
                // Claude's native enablement via scoped settings (same as an
                // explicit `install = delegate`).
                ResolvedPlugin::Delegate { .. } => {
                    if let PluginRef::Marketplace {
                        marketplace,
                        name,
                        scope,
                        ..
                    } = plugin
                    {
                        Self::accumulate_delegate(
                            &mut delegates,
                            marketplace,
                            name,
                            *scope,
                            context,
                        )?;
                    }
                }
            }
        }

        if !mcp_servers.is_empty() {
            actions.push(ProfileAction::MergeJson {
                target: context.target.join(".mcp.json"),
                value: serde_json::json!({ "mcpServers": mcp_servers }),
                owned_paths,
            });
        }

        for (settings_path, settings) in delegates {
            actions.push(ProfileAction::MergeJson {
                target: settings_path,
                value: serde_json::json!({
                    "enabledPlugins": settings.enabled,
                    "extraKnownMarketplaces": settings.marketplaces,
                }),
                owned_paths: settings.owned_paths,
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
            harness: AgentHarness::Claude,
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

    #[test]
    fn claude_places_plugin_agents_into_claude_agents() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("repo");
        fs::create_dir_all(&target).unwrap();

        let bundle = local_bundle(&target, "rust-dev");
        fs::create_dir_all(bundle.join("agents")).unwrap();
        fs::write(
            bundle.join("agents/reviewer.md"),
            "---\nname: reviewer\n---\n",
        )
        .unwrap();

        let profile = ProfileConfig {
            plugins: vec![PluginRef::Local {
                source: PathBuf::from("./rust-dev"),
            }],
            ..ProfileConfig::default()
        };
        let context = context_for(&target, &temp.path().join("claude-home"));

        let plan = ClaudeApplicator.plan(&profile, &context).unwrap();
        assert!(plan.actions.iter().any(|a| matches!(
            a,
            ProfileAction::PlacePluginDir { target: t, .. }
                if t.ends_with("agents/reviewer.md")
        )));
    }

    #[test]
    fn claude_plans_instruction_write_into_claude_md() {
        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("profile-assets");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("claude-instructions.md"), "Be concise.").unwrap();

        let profile = ProfileConfig {
            instructions: vec![crate::profile::InstructionConfig {
                source: Some("claude-instructions.md".to_string()),
                content: None,
                base_dir: None,
            }],
            ..ProfileConfig::default()
        };
        let mut context = context_for(temp.path(), &temp.path().join("claude-home"));
        context.profile_asset_dir = source_dir;

        let plan = ClaudeApplicator.plan(&profile, &context).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            ProfileAction::WriteManagedRegion { target, marker_id, .. }
                if target.ends_with("CLAUDE.md") && marker_id == "rust-dev"
        )));
        assert!(
            !plan
                .actions
                .iter()
                .any(|action| matches!(action, ProfileAction::SkipCapability { .. })),
            "instructions must be placed, not skipped"
        );
    }

    #[test]
    fn claude_rejects_unsafe_instruction_sources() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = context_for(temp.path(), &temp.path().join("claude-home"));

        for source in [
            "../secret.md".to_string(),
            temp.path().join("secret.md").display().to_string(),
        ] {
            let profile = ProfileConfig {
                instructions: vec![crate::profile::InstructionConfig {
                    source: Some(source.clone()),
                    content: None,
                    base_dir: None,
                }],
                ..ProfileConfig::default()
            };

            let err = ClaudeApplicator.plan(&profile, &context).unwrap_err();
            assert!(
                err.to_string().contains("Instruction source"),
                "unexpected error for {source}: {err}"
            );
        }
    }

    fn delegate_plugin(scope: Option<DelegateScope>) -> PluginRef {
        PluginRef::Marketplace {
            marketplace: "vendor".to_string(),
            name: "cool".to_string(),
            r#ref: None,
            install: InstallMode::Delegate,
            scope,
        }
    }

    fn context_with_marketplace(target: &Path, mode: ProfileMode) -> ProfileContext {
        ProfileContext {
            profile_name: "rust-dev".to_string(),
            harness: AgentHarness::Claude,
            target: target.to_path_buf(),
            profile_asset_dir: target.to_path_buf(),
            harness_home: target.join("claude-home"),
            mode,
            session_id: None,
            marketplaces: vec![Marketplace {
                name: "vendor".to_string(),
                url: Some("https://github.com/vendor/market".to_string()),
            }],
            cache: CacheManager::new().unwrap(),
        }
    }

    fn find_merge<'a>(
        plan: &'a ProfilePlan,
        file_name: &str,
    ) -> (&'a Path, &'a serde_json::Value, &'a Vec<String>) {
        plan.actions
            .iter()
            .find_map(|a| match a {
                ProfileAction::MergeJson {
                    target,
                    value,
                    owned_paths,
                    ..
                } if target.ends_with(file_name) => Some((target.as_path(), value, owned_paths)),
                _ => None,
            })
            .expect("expected a settings merge action")
    }

    #[test]
    fn delegate_plugin_writes_project_settings_by_default_when_persistent() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path();
        let profile = ProfileConfig {
            plugins: vec![delegate_plugin(None)],
            ..ProfileConfig::default()
        };
        let context = context_with_marketplace(target, ProfileMode::Persistent);

        let plan = ClaudeApplicator.plan(&profile, &context).unwrap();
        let (path, value, owned) = find_merge(&plan, ".claude/settings.json");
        assert!(path.starts_with(target));
        assert_eq!(value["enabledPlugins"]["cool@vendor"], true);
        assert_eq!(
            value["extraKnownMarketplaces"]["vendor"]["source"]["url"],
            "https://github.com/vendor/market"
        );
        assert!(owned.contains(&"/enabledPlugins/cool@vendor".to_string()));
        assert!(owned.contains(&"/extraKnownMarketplaces/vendor".to_string()));
    }

    #[test]
    fn delegate_plugin_writes_local_settings_when_ephemeral() {
        let temp = tempfile::TempDir::new().unwrap();
        let profile = ProfileConfig {
            plugins: vec![delegate_plugin(None)],
            ..ProfileConfig::default()
        };
        let context = context_with_marketplace(temp.path(), ProfileMode::Ephemeral);

        let plan = ClaudeApplicator.plan(&profile, &context).unwrap();
        find_merge(&plan, ".claude/settings.local.json");
    }

    #[test]
    fn delegate_plugins_sharing_marketplace_dedupe_registration() {
        let temp = tempfile::TempDir::new().unwrap();
        let other = PluginRef::Marketplace {
            marketplace: "vendor".to_string(),
            name: "other".to_string(),
            r#ref: None,
            install: InstallMode::Delegate,
            scope: None,
        };
        let profile = ProfileConfig {
            plugins: vec![delegate_plugin(None), other],
            ..ProfileConfig::default()
        };
        let context = context_with_marketplace(temp.path(), ProfileMode::Persistent);

        let plan = ClaudeApplicator.plan(&profile, &context).unwrap();
        let (_, value, owned) = find_merge(&plan, ".claude/settings.json");
        assert_eq!(value["enabledPlugins"]["cool@vendor"], true);
        assert_eq!(value["enabledPlugins"]["other@vendor"], true);
        assert_eq!(
            owned
                .iter()
                .filter(|p| p.as_str() == "/extraKnownMarketplaces/vendor")
                .count(),
            1
        );
    }

    #[test]
    fn delegate_plugin_without_registered_marketplace_fails() {
        let temp = tempfile::TempDir::new().unwrap();
        let profile = ProfileConfig {
            plugins: vec![delegate_plugin(None)],
            ..ProfileConfig::default()
        };
        let mut context = context_with_marketplace(temp.path(), ProfileMode::Persistent);
        context.marketplaces.clear();

        let err = ClaudeApplicator.plan(&profile, &context).unwrap_err();
        assert!(format!("{err}").contains("no registered URL"));
    }
}
