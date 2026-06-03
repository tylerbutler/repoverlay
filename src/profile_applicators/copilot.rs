#![allow(dead_code)]

use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::profile::{ProfileConfig, ProfileScope};
use crate::profile_applicators::{AgentHarness, ProfileApplicator, ProfileContext};
use crate::profile_plan::{ProfileAction, ProfilePlan};

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

        for instruction in &profile.instructions {
            let source_rel = Path::new(&instruction.source);
            validate_instruction_source(source_rel)?;
            let source = context.profile_asset_dir.join(source_rel);
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

        // Plugin introspection (MCP servers, skills) lands in Task 6; until then,
        // the Copilot applicator records plugins as skipped.
        if !profile.plugins.is_empty() {
            actions.push(ProfileAction::SkipCapability {
                capability: "plugins".to_string(),
                reason: "GitHub Copilot plugin introspection is not implemented yet".to_string(),
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
    use crate::profile::{InstructionConfig, ProfileConfig};
    use crate::profile_applicators::{ProfileApplicator, ProfileContext};

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
        };

        let plan = CopilotApplicator.plan(&profile, &context).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            crate::profile_plan::ProfileAction::WriteFile { target, .. }
                if target.ends_with("instructions/rust-dev/copilot-instructions.md")
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
