use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::path::PathBuf;

use super::super::SourceCommand;
use super::find_repo_root;
use crate::config;
use crate::path_safety::validate_path_component;

/// Handle source subcommands.
pub(crate) fn handle_source_command(command: SourceCommand) -> Result<()> {
    let mut config = config::load_config(None)?;

    match command {
        SourceCommand::Add { source, name } => {
            if source.is_local() {
                // Local path source — save to per-repo config
                let raw_path = source.local_path().to_path_buf();
                let repo_root = find_repo_root()?;

                // Canonicalize and validate path exists
                let canonical = raw_path
                    .canonicalize()
                    .with_context(|| format!("Path does not exist: {}", raw_path.display()))?;

                // Validate within repo
                let repo_root_canonical = repo_root.canonicalize()?;
                if !canonical.starts_with(&repo_root_canonical) {
                    bail!(
                        "Path must be within the repository: {}",
                        canonical.display()
                    );
                }

                // Convert to repo-relative
                let relative_path = canonical
                    .strip_prefix(&repo_root_canonical)
                    .context("Failed to compute repo-relative path")?;

                // Extract name
                let source_name = name.unwrap_or_else(|| {
                    relative_path
                        .file_name()
                        .map_or_else(|| "local".to_string(), |n| n.to_string_lossy().to_string())
                });

                if source_name.is_empty() {
                    bail!(
                        "Could not extract source name from path. Please provide a name with --name"
                    );
                }

                validate_path_component(&source_name)?;
                if source_name.starts_with('@') {
                    bail!(
                        "Source names starting with '@' are reserved. '@library' is a built-in source."
                    );
                }

                // Check name conflicts in both global and repo configs
                let global_config = config::load_global_config()?;
                let repo_config = config::load_repo_config(&repo_root)?;

                if global_config.sources.iter().any(|s| s.name == source_name) {
                    bail!("Source '{source_name}' already exists");
                }
                if let Some(ref rc) = repo_config
                    && rc.sources.iter().any(|s| s.name == source_name)
                {
                    bail!("Source '{source_name}' already exists");
                }

                let new_source = config::Source {
                    name: source_name.clone(),
                    url: None,
                    path: Some(PathBuf::from(relative_path)),
                };

                let mut updated_repo_config = repo_config.unwrap_or_default();
                updated_repo_config.sources.push(new_source);
                config::save_repo_config(&repo_root, &updated_repo_config)?;

                println!(
                    "{} local source '{}' at position {}",
                    "Added".green().bold(),
                    source_name,
                    updated_repo_config.sources.len()
                );
                println!("       Path: {}", relative_path.display());
                println!(
                    "       Config: {}",
                    config::repo_config_path(&repo_root).display()
                );
            } else {
                // Git URL source — save to global config
                let validated_url = source.to_url();
                let source_name = name.unwrap_or_else(|| {
                    validated_url
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("source")
                        .trim_end_matches(".git")
                        .to_string()
                });

                if source_name.is_empty() {
                    bail!(
                        "Could not extract source name from URL. Please provide a name with --name"
                    );
                }

                validate_path_component(&source_name)?;
                if source_name.starts_with('@') {
                    bail!(
                        "Source names starting with '@' are reserved. '@library' is a built-in source."
                    );
                }

                // Check if name already exists
                if config.sources.iter().any(|s| s.name == source_name) {
                    bail!("Source '{source_name}' already exists");
                }

                let new_source = config::Source {
                    name: source_name.clone(),
                    url: Some(validated_url.clone()),
                    path: None,
                };
                config.sources.push(new_source);
                config::save_config(&config)?;

                println!(
                    "{} source '{}' at position {}",
                    "Added".green().bold(),
                    source_name,
                    config.sources.len()
                );
                println!("       {validated_url}");
            }
        }
        SourceCommand::List => {
            let repo_root = find_repo_root().ok();

            let global_config = config::load_config(None)?;
            let repo_config = repo_root
                .as_ref()
                .map(|r| config::load_repo_config(r))
                .transpose()?
                .flatten()
                .unwrap_or_default();

            let has_sources = !global_config.sources.is_empty() || !repo_config.sources.is_empty();

            if !has_sources {
                println!("No overlay sources configured.");
                println!("Add one with: repoverlay source add <url-or-path>");
                return Ok(());
            }

            // Show repo sources first (higher priority)
            if !repo_config.sources.is_empty() {
                println!(
                    "{} ({})",
                    "Repository sources".bold(),
                    repo_root
                        .as_ref()
                        .map(|r| config::repo_config_path(r).display().to_string())
                        .unwrap_or_default()
                );
                for (i, source) in repo_config.sources.iter().enumerate() {
                    print!("  {}. {}", i + 1, source.name.bold());
                    if let Some(path) = &source.path {
                        println!(" (path: {})", path.display());
                    } else if let Some(url) = &source.url {
                        println!(" (url: {url})");
                    } else {
                        println!();
                    }
                }
            }

            if !global_config.sources.is_empty() {
                if !repo_config.sources.is_empty() {
                    println!();
                }
                println!(
                    "{} (~/.config/repoverlay/config.ccl)",
                    "Global sources".bold()
                );
                for (i, source) in global_config.sources.iter().enumerate() {
                    let position = repo_config.sources.len() + i + 1;
                    print!("  {}. {}", position, source.name.bold());
                    if let Some(url) = &source.url {
                        println!(" (url: {url})");
                    } else if let Some(path) = &source.path {
                        println!(" (path: {})", path.display());
                    } else {
                        println!();
                    }
                }
            }

            println!("\nSources are checked in priority order (lowest number = highest priority).");
        }
        SourceCommand::Remove { name } => {
            let repo_root = find_repo_root().ok();

            // Check repo config first
            if let Some(ref root) = repo_root
                && let Some(mut repo_config) = config::load_repo_config(root)?
                && let Some(pos) = repo_config.sources.iter().position(|s| s.name == name)
            {
                repo_config.sources.remove(pos);
                config::save_repo_config(root, &repo_config)?;
                println!(
                    "{} source '{}' from repository config",
                    "Removed".red().bold(),
                    name
                );
                return Ok(());
            }

            // Fall back to global config
            if let Some(pos) = config.sources.iter().position(|s| s.name == name) {
                config.sources.remove(pos);
                config::save_config(&config)?;
                println!(
                    "{} source '{}' from global config",
                    "Removed".red().bold(),
                    name
                );
            } else {
                bail!("Source '{name}' not found in any config");
            }
        }
    }

    Ok(())
}
