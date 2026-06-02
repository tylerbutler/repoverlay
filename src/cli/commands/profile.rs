use anyhow::{Result, bail};
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
                    .map(|entry| entry.source.clone())
                    .collect::<Vec<_>>(),
            );
            if !profile.mcps.servers.is_empty() {
                println!("  MCP servers:");
                for (server, config) in &profile.mcps.servers {
                    println!("    - {} ({})", server, config.command);
                }
            }
            print_list("Skills", &profile.skills);
            print_list("Plugins", &profile.plugins);
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
        ProfileCommand::Status { .. } | ProfileCommand::Remove { .. } => {
            bail!("profile status/remove are added in the profile lifecycle task")
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
