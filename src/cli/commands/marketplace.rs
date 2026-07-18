use anyhow::{Result, bail};
use colored::Colorize;

use super::super::MarketplaceCommand;
use super::find_repo_root;
use crate::config::{self, Marketplace};
use crate::selection::is_interactive;

/// Handle marketplace subcommands.
pub(crate) fn handle_marketplace_command(command: MarketplaceCommand) -> Result<()> {
    match command {
        MarketplaceCommand::Add { name, url, yes } => add_marketplace(&name, &url, yes),
        MarketplaceCommand::List => list_marketplaces(),
        MarketplaceCommand::Remove { name } => remove_marketplace(&name),
    }
}

/// Validate that a marketplace name is usable in a `marketplace/plugin` reference.
fn validate_marketplace_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Marketplace name cannot be empty");
    }
    if name.contains('/') || name.chars().any(char::is_whitespace) {
        bail!("Marketplace name '{name}' must not contain '/' or whitespace");
    }
    Ok(())
}

fn add_marketplace(name: &str, url: &str, yes: bool) -> Result<()> {
    validate_marketplace_name(name)?;

    let expanded = config::validate_marketplace_url(url).map_err(|e| anyhow::anyhow!(e))?;

    let mut global = config::load_config(None)?;

    // Reject re-registration of the same name with a different URL; allow an
    // idempotent re-add with the same URL.
    if let Some(existing) = global.marketplaces.iter().find(|m| m.name == name) {
        if existing.url.as_deref() == Some(expanded.as_str()) {
            println!(
                "{} marketplace '{}' is already registered with this URL",
                "Note:".yellow(),
                name
            );
            return Ok(());
        }
        bail!(
            "Marketplace '{name}' is already registered with a different URL ({}). \
             Remove it first to change the URL.",
            existing.url.as_deref().unwrap_or("<none>")
        );
    }

    // Marketplaces introduce executable behavior; confirm before registering.
    if !yes && is_interactive() {
        let confirmed = crate::prompt::confirm(
            &format!("Register marketplace '{name}' from {expanded}?"),
            true,
        )?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    global.marketplaces.push(Marketplace {
        name: name.to_string(),
        url: Some(expanded.clone()),
    });
    config::save_config(&global)?;

    println!("{} marketplace '{}'", "Added".green().bold(), name);
    println!("       {expanded}");
    Ok(())
}

fn list_marketplaces() -> Result<()> {
    let repo_root = find_repo_root().ok();

    let global = config::load_config(None)?;
    let repo_config = repo_root
        .as_ref()
        .map(|r| config::load_repo_config(r))
        .transpose()?
        .flatten()
        .unwrap_or_default();

    if global.marketplaces.is_empty() && repo_config.marketplaces.is_empty() {
        println!("No marketplaces registered.");
        println!("Add one with: repoverlay marketplace add <name> <url>");
        return Ok(());
    }

    if !repo_config.marketplaces.is_empty() {
        println!(
            "{} ({})",
            "Repository marketplaces".bold(),
            repo_root
                .as_ref()
                .map(|r| config::repo_config_path(r).display().to_string())
                .unwrap_or_default()
        );
        for m in &repo_config.marketplaces {
            print_marketplace(m);
        }
    }

    if !global.marketplaces.is_empty() {
        if !repo_config.marketplaces.is_empty() {
            println!();
        }
        println!(
            "{} (~/.config/repoverlay/config.ccl)",
            "Global marketplaces".bold()
        );
        for m in &global.marketplaces {
            print_marketplace(m);
        }
    }

    Ok(())
}

fn print_marketplace(m: &Marketplace) {
    match &m.url {
        Some(url) => println!("  {} ({url})", m.name.bold()),
        None => println!("  {}", m.name.bold()),
    }
}

fn remove_marketplace(name: &str) -> Result<()> {
    let repo_root = find_repo_root().ok();

    // Check repo config first.
    if let Some(ref root) = repo_root
        && let Some(mut repo_config) = config::load_repo_config(root)?
        && let Some(pos) = repo_config.marketplaces.iter().position(|m| m.name == name)
    {
        repo_config.marketplaces.remove(pos);
        config::save_repo_config(root, &repo_config)?;
        println!(
            "{} marketplace '{}' from repository config",
            "Removed".red().bold(),
            name
        );
        return Ok(());
    }

    let mut global = config::load_config(None)?;
    if let Some(pos) = global.marketplaces.iter().position(|m| m.name == name) {
        global.marketplaces.remove(pos);
        config::save_config(&global)?;
        println!(
            "{} marketplace '{}' from global config",
            "Removed".red().bold(),
            name
        );
        Ok(())
    } else {
        bail!("Marketplace '{name}' not found in any config");
    }
}
