use anyhow::{Result, bail};
use colored::Colorize;
use std::io::{self, Write};

use crate::CacheManager;
use super::super::CacheCommand;

pub(crate) fn handle_cache_command(command: CacheCommand) -> Result<()> {
    let cache = CacheManager::new()?;

    match command {
        CacheCommand::List => {
            let repos = cache.list_cached()?;

            if repos.is_empty() {
                println!("{} No repositories cached.", "Cache:".bold());
                return Ok(());
            }

            println!("{} {} cached repository(s):", "Cache:".bold(), repos.len());
            println!();

            for repo in repos {
                println!("  {}/{}", repo.owner.cyan(), repo.repo);
                if let Some(meta) = repo.meta {
                    println!("    Ref:     {}", meta.requested_ref);
                    println!("    Commit:  {}", &meta.commit[..12.min(meta.commit.len())]);
                    println!(
                        "    Fetched: {}",
                        meta.last_fetched.format("%Y-%m-%d %H:%M UTC")
                    );
                }
                println!("    Path:    {}", repo.path.display());
                println!();
            }
        }

        CacheCommand::Clear { yes } => {
            eprintln!(
                "{} 'repoverlay cache clear' is deprecated and will be removed in 1.0.",
                "Warning:".yellow().bold()
            );
            eprintln!("         Use 'repoverlay cache remove --all' instead.");
            eprintln!();

            clear_cache(&cache, yes)?;
        }

        CacheCommand::Remove { repo, all, yes } => {
            if all {
                clear_cache(&cache, yes)?;
            } else if let Some(repo) = repo {
                let parts: Vec<&str> = repo.split('/').collect();
                if parts.len() != 2 {
                    bail!("Invalid repository format. Use: owner/repo");
                }

                let (owner, repo_name) = (parts[0], parts[1]);

                if cache.remove_cached(owner, repo_name)? {
                    println!(
                        "{} Removed {}/{} from cache.",
                        "✓".green().bold(),
                        owner,
                        repo_name
                    );
                } else {
                    println!("{owner}/{repo_name} is not cached.");
                }
            } else {
                bail!(
                    "Specify a repository to remove or use --all.\n\n\
                     Usage:\n  \
                     repoverlay cache remove <owner/repo>  # Remove specific repo\n  \
                     repoverlay cache remove --all          # Remove all cached repos"
                );
            }
        }

        CacheCommand::Path => {
            println!("{}", cache.cache_dir().display());
        }
    }

    Ok(())
}

/// Clear the entire cache with optional confirmation prompt.
fn clear_cache(cache: &CacheManager, skip_confirm: bool) -> Result<()> {
    if !skip_confirm {
        print!("Remove all cached repositories? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let count = cache.clear_cache()?;
    println!(
        "{} Removed {} cached repository(s).",
        "✓".green().bold(),
        count
    );
    Ok(())
}
