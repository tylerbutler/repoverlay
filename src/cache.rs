//! Cache management for GitHub repository overlays.
//!
//! Handles downloading, caching, and updating GitHub repositories for use as overlays.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::github::{GitHubSource, GitRef};

/// Metadata about a cached repository.
#[derive(Debug, Deserialize, Serialize)]
pub struct CacheMeta {
    /// The clone URL
    pub clone_url: String,
    /// When the cache was last fetched
    pub last_fetched: DateTime<Utc>,
    /// The git ref that was requested
    pub requested_ref: String,
    /// The resolved commit SHA
    pub commit: String,
}

/// Result of caching a GitHub repository.
#[derive(Debug)]
pub struct CachedOverlay {
    /// Path to the overlay files (may include subpath)
    pub path: PathBuf,
    /// The resolved commit SHA
    pub commit: String,
    /// When the cache was created/updated
    #[allow(dead_code)]
    pub cached_at: DateTime<Utc>,
}

/// Information about a cached repository.
#[derive(Debug)]
pub struct CachedRepoInfo {
    /// Owner name
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Path to the cached repo
    pub path: PathBuf,
    /// Cache metadata (if available)
    pub meta: Option<CacheMeta>,
}

/// Manager for the overlay cache.
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    /// Create a new cache manager.
    pub fn new() -> Result<Self> {
        let cache_dir = cache_dir()?;
        Ok(Self { cache_dir })
    }

    /// Get the cache directory path.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Ensure a GitHub repository is cached and at the correct ref.
    ///
    /// Returns the path to the overlay files.
    pub fn ensure_cached(&self, source: &GitHubSource, update: bool) -> Result<CachedOverlay> {
        let repo_path = self.repo_path(source);

        if repo_path.exists() {
            if update {
                self.update_repo(&repo_path)?;
            }
            self.checkout_ref(&repo_path, source)?;
        } else {
            self.clone_repo(source, &repo_path)?;
        }

        let overlay_path = match &source.subpath {
            Some(subpath) => {
                let path = repo_path.join(subpath);
                if !path.exists() {
                    bail!(
                        "Subpath '{}' not found in repository {}/{}",
                        subpath.display(),
                        source.owner,
                        source.repo
                    );
                }
                path
            }
            None => repo_path.clone(),
        };

        let commit = self.get_current_commit(&repo_path)?;
        let cached_at = Utc::now();

        // Save cache metadata
        self.save_meta(&repo_path, source, &commit)?;

        Ok(CachedOverlay {
            path: overlay_path,
            commit,
            cached_at,
        })
    }

    /// Get the path where a repository would be cached.
    pub fn repo_path(&self, source: &GitHubSource) -> PathBuf {
        self.cache_dir
            .join("github")
            .join(&source.owner)
            .join(&source.repo)
    }

    /// Clone a repository.
    fn clone_repo(&self, source: &GitHubSource, target: &Path) -> Result<()> {
        // Create parent directories
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut cmd = Command::new("git");
        cmd.args(["clone", "--depth", "1"]);

        // If specific branch/tag, clone that
        match &source.git_ref {
            GitRef::Branch(branch) | GitRef::Tag(branch) => {
                cmd.args(["--branch", branch]);
            }
            GitRef::Default | GitRef::Commit(_) => {
                // Clone default branch, will checkout specific commit after
            }
        }

        cmd.arg(source.clone_url());
        cmd.arg(target);

        let output = cmd.output().context("Failed to execute git clone")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not found") || stderr.contains("Repository not found") {
                bail!("Repository not found: {}/{}", source.owner, source.repo);
            }
            if stderr.contains("could not find remote branch") {
                bail!(
                    "Branch or tag not found: {} in {}/{}",
                    source.git_ref.as_str(),
                    source.owner,
                    source.repo
                );
            }
            bail!("Failed to clone repository: {}", stderr.trim());
        }

        // If a specific commit was requested, we need to fetch and checkout
        if let GitRef::Commit(sha) = &source.git_ref {
            self.fetch_commit(target, sha)?;
        }

        Ok(())
    }

    /// Update an existing cached repository.
    fn update_repo(&self, repo_path: &Path) -> Result<()> {
        let output = Command::new("git")
            .args(["fetch", "--depth", "1", "origin"])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git fetch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to fetch updates: {}", stderr.trim());
        }

        Ok(())
    }

    /// Checkout a specific ref.
    fn checkout_ref(&self, repo_path: &Path, source: &GitHubSource) -> Result<()> {
        let ref_spec = match &source.git_ref {
            GitRef::Default => "origin/HEAD",
            GitRef::Branch(b) => {
                // For branches, try origin/branch first
                let origin_ref = format!("origin/{}", b);
                if self.ref_exists(repo_path, &origin_ref)? {
                    return self.do_checkout(repo_path, &origin_ref);
                }
                // Fall back to local ref
                b.as_str()
            }
            GitRef::Tag(t) => t.as_str(),
            GitRef::Commit(c) => c.as_str(),
        };

        self.do_checkout(repo_path, ref_spec)
    }

    /// Check if a ref exists in the repository.
    fn ref_exists(&self, repo_path: &Path, ref_spec: &str) -> Result<bool> {
        let output = Command::new("git")
            .args(["rev-parse", "--verify", ref_spec])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git rev-parse")?;

        Ok(output.status.success())
    }

    /// Perform the actual checkout.
    fn do_checkout(&self, repo_path: &Path, ref_spec: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["checkout", ref_spec])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git checkout")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to checkout {}: {}", ref_spec, stderr.trim());
        }

        Ok(())
    }

    /// Fetch a specific commit.
    fn fetch_commit(&self, repo_path: &Path, sha: &str) -> Result<()> {
        // First, unshallow if needed to access the commit
        let output = Command::new("git")
            .args(["fetch", "--unshallow", "origin"])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git fetch --unshallow")?;

        // Ignore errors from unshallow (might already be complete)
        let _ = output;

        // Fetch the specific commit
        let output = Command::new("git")
            .args(["fetch", "origin", sha])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git fetch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Failed to fetch commit {}: {}",
                &sha[..12.min(sha.len())],
                stderr.trim()
            );
        }

        // Checkout the commit
        let output = Command::new("git")
            .args(["checkout", sha])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git checkout")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Failed to checkout commit {}: {}",
                &sha[..12.min(sha.len())],
                stderr.trim()
            );
        }

        Ok(())
    }

    /// Get the current commit SHA.
    fn get_current_commit(&self, repo_path: &Path) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_path)
            .output()
            .context("Failed to execute git rev-parse")?;

        if !output.status.success() {
            bail!("Failed to get current commit");
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    /// Save cache metadata.
    fn save_meta(&self, repo_path: &Path, source: &GitHubSource, commit: &str) -> Result<()> {
        let meta = CacheMeta {
            clone_url: source.clone_url(),
            last_fetched: Utc::now(),
            requested_ref: source.git_ref.as_str().to_string(),
            commit: commit.to_string(),
        };

        let meta_path = repo_path.join(".repoverlay-cache-meta.toml");
        fs::write(&meta_path, toml::to_string_pretty(&meta)?)?;

        Ok(())
    }

    /// Load cache metadata.
    fn load_meta(&self, repo_path: &Path) -> Option<CacheMeta> {
        let meta_path = repo_path.join(".repoverlay-cache-meta.toml");
        if meta_path.exists()
            && let Ok(content) = fs::read_to_string(&meta_path)
        {
            return toml::from_str(&content).ok();
        }
        None
    }

    /// List all cached repositories.
    pub fn list_cached(&self) -> Result<Vec<CachedRepoInfo>> {
        let github_dir = self.cache_dir.join("github");

        if !github_dir.exists() {
            return Ok(Vec::new());
        }

        let mut repos = Vec::new();

        for owner_entry in fs::read_dir(&github_dir)? {
            let owner_entry = owner_entry?;
            if !owner_entry.file_type()?.is_dir() {
                continue;
            }

            let owner = owner_entry.file_name().to_string_lossy().to_string();

            for repo_entry in fs::read_dir(owner_entry.path())? {
                let repo_entry = repo_entry?;
                if !repo_entry.file_type()?.is_dir() {
                    continue;
                }

                let repo = repo_entry.file_name().to_string_lossy().to_string();
                let path = repo_entry.path();
                let meta = self.load_meta(&path);

                repos.push(CachedRepoInfo {
                    owner: owner.clone(),
                    repo,
                    path,
                    meta,
                });
            }
        }

        repos.sort_by(|a, b| (&a.owner, &a.repo).cmp(&(&b.owner, &b.repo)));

        Ok(repos)
    }

    /// Remove a specific cached repository.
    pub fn remove_cached(&self, owner: &str, repo: &str) -> Result<bool> {
        let path = self.cache_dir.join("github").join(owner).join(repo);

        if path.exists() {
            fs::remove_dir_all(&path)?;

            // Clean up empty parent directories
            let owner_dir = self.cache_dir.join("github").join(owner);
            if owner_dir.exists() && owner_dir.read_dir()?.next().is_none() {
                fs::remove_dir(&owner_dir)?;
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Clear the entire cache.
    pub fn clear_cache(&self) -> Result<usize> {
        let github_dir = self.cache_dir.join("github");

        if !github_dir.exists() {
            return Ok(0);
        }

        let repos = self.list_cached()?;
        let count = repos.len();

        fs::remove_dir_all(&github_dir)?;

        Ok(count)
    }

    /// Check for updates to a cached repository.
    ///
    /// Returns the latest commit on the default branch if different from current.
    pub fn check_for_updates(&self, source: &GitHubSource) -> Result<Option<String>> {
        let repo_path = self.repo_path(source);

        if !repo_path.exists() {
            return Ok(None);
        }

        let current_commit = self.get_current_commit(&repo_path)?;

        // Fetch latest
        let output = Command::new("git")
            .args(["fetch", "--depth", "1", "origin"])
            .current_dir(&repo_path)
            .output()
            .context("Failed to fetch")?;

        if !output.status.success() {
            return Ok(None);
        }

        // Get the remote HEAD commit
        let ref_spec = match &source.git_ref {
            GitRef::Default => "origin/HEAD",
            GitRef::Branch(b) => &format!("origin/{}", b),
            GitRef::Tag(_) | GitRef::Commit(_) => {
                // Tags and commits don't have "updates"
                return Ok(None);
            }
        };

        let output = Command::new("git")
            .args(["rev-parse", ref_spec])
            .current_dir(&repo_path)
            .output()
            .context("Failed to get remote commit")?;

        if !output.status.success() {
            return Ok(None);
        }

        let remote_commit = String::from_utf8(output.stdout)?.trim().to_string();

        if remote_commit != current_commit {
            Ok(Some(remote_commit))
        } else {
            Ok(None)
        }
    }
}

/// Get the cache directory.
pub fn cache_dir() -> Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("", "", "repoverlay")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;

    Ok(proj_dirs.cache_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_manager_creation() {
        let manager = CacheManager::new();
        assert!(manager.is_ok());
    }

    #[test]
    fn test_repo_path() {
        let manager = CacheManager::new().unwrap();
        let source = GitHubSource::parse("https://github.com/owner/repo").unwrap();
        let path = manager.repo_path(&source);

        assert!(path.ends_with("github/owner/repo"));
    }

    #[test]
    fn test_repo_path_with_subpath() {
        let manager = CacheManager::new().unwrap();
        let source = GitHubSource::parse("https://github.com/owner/repo/tree/main/subdir").unwrap();
        let path = manager.repo_path(&source);

        // Subpath should not affect cache path (repo is cached, subpath is used at read time)
        assert!(path.ends_with("github/owner/repo"));
    }
}
