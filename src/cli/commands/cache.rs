use anyhow::{Result, bail};
use colored::Colorize;
use std::io::{self, Write};

use super::super::CacheCommand;
use crate::CacheManager;
use crate::cache::{CachedRepoInfo, CachedSourceInfo};

pub(crate) fn handle_cache_command(command: CacheCommand) -> Result<()> {
    let cache = CacheManager::new()?;

    match command {
        CacheCommand::List => {
            let repos = cache.list_cached()?;
            let sources = cache.list_cached_sources()?;

            if repos.is_empty() && sources.is_empty() {
                println!(
                    "{} No cached GitHub repositories or configured source clones.",
                    "Cache:".bold()
                );
                return Ok(());
            }

            println!("{}", "Cache:".bold());
            println!();

            print_cached_github_repositories(&repos);
            println!();
            print_cached_source_clones(&sources);
        }

        CacheCommand::Remove {
            repo,
            source,
            all,
            yes,
        } => {
            if all {
                clear_cache(&cache, yes)?;
            } else if let Some(repo) = repo {
                let parts: Vec<&str> = repo.split('/').collect();
                if parts.len() != 2 {
                    bail!("Invalid GitHub repository format. Use: owner/repo");
                }

                let (owner, repo_name) = (parts[0], parts[1]);

                if cache.remove_cached(owner, repo_name)? {
                    println!(
                        "{} Removed GitHub repository {}/{} from cache.",
                        "✓".green().bold(),
                        owner,
                        repo_name
                    );
                } else {
                    println!("GitHub repository {owner}/{repo_name} is not cached.");
                }
            } else if let Some(source) = source {
                if cache.remove_cached_source(&source)? {
                    println!(
                        "{} Removed configured source clone {} from cache.",
                        "✓".green().bold(),
                        source
                    );
                } else {
                    println!("Configured source clone {source} is not cached.");
                }
            } else {
                bail!(
                    "Specify a GitHub repository, --source <name>, or --all.\n\n\
                     Usage:\n  \
                     repoverlay cache remove <owner/repo>  # Remove a cached GitHub repository\n  \
                     repoverlay cache remove --source <name>  # Remove a cached configured source clone\n  \
                     repoverlay cache remove --all          # Remove all cached GitHub repositories and configured source clones"
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
        print!("Remove all cached GitHub repositories and configured source clones? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled. No GitHub repositories or configured source clones were removed.");
            return Ok(());
        }
    }

    let counts = cache.clear_cache()?;
    println!(
        "{} Removed {} GitHub repository(s) and {} configured source clone(s).",
        "✓".green().bold(),
        counts.github,
        counts.sources
    );
    Ok(())
}

fn print_cached_github_repositories(repos: &[CachedRepoInfo]) {
    println!("{}", "GitHub repositories".bold());

    if repos.is_empty() {
        println!("  (none cached)");
        return;
    }

    for repo in repos {
        println!("  {}/{}", repo.owner.cyan(), repo.repo);
        if let Some(meta) = &repo.meta {
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

fn print_cached_source_clones(sources: &[CachedSourceInfo]) {
    println!("{}", "Configured source clones".bold());

    if sources.is_empty() {
        println!("  (none cached)");
        return;
    }

    for source in sources {
        println!("  {}", source.name.cyan());
        println!("    Path:    {}", source.path.display());
        println!();
    }
}
