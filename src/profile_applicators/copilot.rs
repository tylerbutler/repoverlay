#![allow(dead_code)]

use anyhow::{Context, Result};
use serde_json::json;
use std::process::Command;

use crate::profile::{ProfileConfig, ProfileScope};
use crate::profile_applicators::{AgentHarness, ProfileApplicator, ProfileContext};
use crate::profile_plan::{ProfileAction, ProfilePlan};

pub(crate) struct CopilotApplicator;

impl CopilotApplicator {
    pub(crate) fn harness_home_from_env() -> Result<std::path::PathBuf> {
        if let Some(home) = std::env::var_os("REPOVERLAY_COPILOT_HOME") {
            return Ok(home.into());
        }
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".config").join("github-copilot"))
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

        if !profile.mcps.servers.is_empty() {
            let mut servers = serde_json::Map::new();
            for (name, server) in &profile.mcps.servers {
                servers.insert(
                    name.clone(),
                    json!({
                        "command": server.command,
                        "args": server.args,
                        "env": server.env,
                    }),
                );
            }
            actions.push(ProfileAction::MergeJson {
                target: context.harness_home.join("mcp.json"),
                value: json!({ "servers": servers }),
                scope: ProfileScope::User,
            });
        }

        for instruction in &profile.instructions {
            let source = context.profile_asset_dir.join(&instruction.source);
            let file_name = source
                .file_name()
                .map(std::ffi::OsStr::to_owned)
                .context("Instruction source has no file name")?;
            actions.push(ProfileAction::WriteFile {
                source,
                target: context
                    .harness_home
                    .join("instructions")
                    .join(&context.profile_name)
                    .join(file_name),
                scope: ProfileScope::User,
            });
        }

        if !profile.skills.is_empty() {
            actions.push(ProfileAction::SkipCapability {
                capability: "skills".to_string(),
                reason: "GitHub Copilot skill placement is not defined in v1".to_string(),
            });
        }
        if !profile.plugins.is_empty() {
            actions.push(ProfileAction::SkipCapability {
                capability: "plugins".to_string(),
                reason: "GitHub Copilot plugin placement is not defined in v1".to_string(),
            });
        }

        Ok(ProfilePlan {
            profile_name: context.profile_name.clone(),
            harness: "copilot".to_string(),
            actions,
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
    use crate::profile::{InstructionConfig, McpConfig, McpServerConfig, ProfileConfig};
    use crate::profile_applicators::{ProfileApplicator, ProfileContext};
    use std::collections::BTreeMap;

    #[test]
    fn copilot_plans_mcp_merge_and_instruction_write() {
        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("profile-assets");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("copilot-instructions.md"), "Be concise.").unwrap();

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
        let context = ProfileContext {
            profile_name: "rust-dev".to_string(),
            target: temp.path().to_path_buf(),
            profile_asset_dir: source_dir,
            harness_home: temp.path().join("copilot-home"),
            mode: crate::profile::ProfileMode::Persistent,
            session_id: None,
        };

        let plan = CopilotApplicator.plan(&profile, &context).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            crate::profile_plan::ProfileAction::MergeJson { target, .. }
                if target.ends_with("mcp.json")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            crate::profile_plan::ProfileAction::WriteFile { target, .. }
                if target.ends_with("instructions/rust-dev/copilot-instructions.md")
        )));
    }

    #[test]
    #[allow(unsafe_code)]
    fn copilot_harness_home_defaults_to_github_copilot_config_dir() {
        let temp = tempfile::TempDir::new().unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", temp.path());
            std::env::remove_var("REPOVERLAY_COPILOT_HOME");
        }
        assert!(std::env::var_os("REPOVERLAY_COPILOT_HOME").is_none());

        let expected = dirs::home_dir().unwrap().join(".config/github-copilot");

        assert_eq!(
            CopilotApplicator::harness_home_from_env().unwrap(),
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
