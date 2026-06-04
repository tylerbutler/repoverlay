use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;

use crate::cli::ProfileCommand;
use crate::config;

use super::resolve_target;

pub(crate) fn handle_profile_command(command: ProfileCommand) -> Result<()> {
    match command {
        ProfileCommand::List { target } => {
            let target = resolve_target(target)?;
            let config = config::load_config(Some(&target))?;
            if config.profiles.is_empty() {
                println!("No profiles configured.");
                return Ok(());
            }

            for (name, profile) in &config.profiles {
                match &profile.description {
                    Some(description) => println!("{} - {description}", name.bold()),
                    None => println!("{}", name.bold()),
                }
            }
            Ok(())
        }
        ProfileCommand::Show { name, target } => {
            let target = resolve_target(target)?;
            let config = config::load_config(Some(&target))?;
            let profile = config
                .profiles
                .get(&name)
                .ok_or_else(|| anyhow::anyhow!("Profile '{name}' not found"))?;

            println!("{}", name.bold());
            if let Some(description) = &profile.description {
                println!("  Description: {description}");
            }
            print_list("Overlays", &profile.overlays);
            print_list(
                "Instructions",
                &profile
                    .instructions
                    .iter()
                    .map(crate::profile::InstructionConfig::label)
                    .collect::<Vec<_>>(),
            );
            if !profile.plugins.is_empty() {
                print_list(
                    "Plugins",
                    &profile
                        .plugins
                        .iter()
                        .map(format_plugin_for_show)
                        .collect::<Vec<_>>(),
                );
            }
            Ok(())
        }
        ProfileCommand::Apply {
            name,
            harness,
            target,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            let target = crate::canonicalize_path(&target, "Target")?;
            crate::validate_git_repo(&target)?;
            crate::profile_plan::apply_profile(
                &name,
                &harness,
                &target,
                crate::profile::ProfileMode::Persistent,
                None,
            )?;
            Ok(())
        }
        ProfileCommand::Status { target, harness } => {
            let target = resolve_target(target)?;
            let states = crate::profile_plan::list_profile_states(&target)?;
            let states: Vec<_> = states
                .into_iter()
                .filter(|state| harness.as_ref().is_none_or(|h| h == &state.harness))
                .collect();
            if states.is_empty() {
                println!("No profiles applied.");
                return Ok(());
            }
            for state in states {
                println!("{} ({})", state.name.bold(), state.harness);
            }
            Ok(())
        }
        ProfileCommand::Remove {
            name,
            harness,
            target,
        } => {
            let target = resolve_target(target)?;
            crate::profile_plan::remove_profile(&name, &harness, &target)?;
            Ok(())
        }
    }
}

fn print_list(label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    println!("  {label}:");
    for value in values {
        println!("    - {value}");
    }
}

/// Render a plugin reference for `profile show`, annotating the install mode and
/// (for delegate plugins) the enablement scope.
fn format_plugin_for_show(plugin: &crate::plugin::PluginRef) -> String {
    use crate::plugin::{InstallMode, PluginRef};
    use crate::profile::DelegateScope;
    use std::fmt::Write as _;

    match plugin {
        PluginRef::Local { source } => {
            format!("{} (local, managed)", source.display())
        }
        PluginRef::Marketplace {
            marketplace,
            name,
            r#ref,
            install,
            scope,
        } => {
            let mut out = format!("{marketplace}/{name}");
            if let Some(r) = r#ref {
                let _ = write!(out, "@{r}");
            }
            match install {
                InstallMode::Managed => out.push_str(" (managed)"),
                InstallMode::Delegate => {
                    let scope = match scope {
                        Some(DelegateScope::Project) => "project",
                        Some(DelegateScope::Local) => "local",
                        None => "default",
                    };
                    let _ = write!(out, " (delegate, scope: {scope})");
                }
            }
            out
        }
    }
}
