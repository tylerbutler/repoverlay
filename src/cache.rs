//! Cache management for GitHub repository overlays.
//!
//! Handles downloading, caching, and updating GitHub repositories for use as overlays.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::github::{GitHubSource, GitRef};

/// Execute a git command in a directory and return the output.
fn git_in_dir(repo_path: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .with_context(|| format!("Failed to execute git {}", args.first().unwrap_or(&"")))
}

/// Execute a git command in a directory and check for success.
fn git_run(repo_path: &Path, args: &[&str]) -> Result<()> {
    let output = git_in_dir(repo_path, args)?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr.trim()
        )
    }
}

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
        git_run(repo_path, &["fetch", "--depth", "1", "origin"]).context("Failed to fetch updates")
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
        let output = git_in_dir(repo_path, &["rev-parse", "--verify", ref_spec])?;
        Ok(output.status.success())
    }

    /// Perform the actual checkout.
    fn do_checkout(&self, repo_path: &Path, ref_spec: &str) -> Result<()> {
        git_run(repo_path, &["checkout", ref_spec])
            .with_context(|| format!("Failed to checkout {}", ref_spec))
    }

    /// Fetch a specific commit.
    fn fetch_commit(&self, repo_path: &Path, sha: &str) -> Result<()> {
        // First, unshallow if needed to access the commit (ignore errors - might already be complete)
        let _ = git_in_dir(repo_path, &["fetch", "--unshallow", "origin"]);

        // Fetch the specific commit
        git_run(repo_path, &["fetch", "origin", sha])
            .with_context(|| format!("Failed to fetch commit {}", &sha[..12.min(sha.len())]))?;

        // Checkout the commit
        git_run(repo_path, &["checkout", sha])
            .with_context(|| format!("Failed to checkout commit {}", &sha[..12.min(sha.len())]))
    }

    /// Get the current commit SHA.
    fn get_current_commit(&self, repo_path: &Path) -> Result<String> {
        let output = git_in_dir(repo_path, &["rev-parse", "HEAD"])?;
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

        let meta_path = repo_path.join(".repoverlay-cache-meta.ccl");
        fs::write(&meta_path, sickle::to_string(&meta)?)?;

        Ok(())
    }

    /// Load cache metadata.
    fn load_meta(&self, repo_path: &Path) -> Option<CacheMeta> {
        let meta_path = repo_path.join(".repoverlay-cache-meta.ccl");
        if meta_path.exists()
            && let Ok(content) = fs::read_to_string(&meta_path)
        {
            return sickle::from_str(&content).ok();
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
        let output = git_in_dir(&repo_path, &["fetch", "--depth", "1", "origin"])?;
        if !output.status.success() {
            return Ok(None);
        }

        // Get the remote HEAD commit
        let ref_spec = match &source.git_ref {
            GitRef::Default => "origin/HEAD".to_string(),
            GitRef::Branch(b) => format!("origin/{}", b),
            GitRef::Tag(_) | GitRef::Commit(_) => {
                // Tags and commits don't have "updates"
                return Ok(None);
            }
        };

        let output = git_in_dir(&repo_path, &["rev-parse", &ref_spec])?;
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
    use tempfile::TempDir;

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

    #[test]
    fn test_cache_dir_function() {
        let dir = cache_dir();
        assert!(dir.is_ok());
        let dir = dir.unwrap();
        assert!(dir.to_string_lossy().contains("repoverlay"));
    }

    #[test]
    fn test_cache_manager_cache_dir_accessor() {
        let manager = CacheManager::new().unwrap();
        let dir = manager.cache_dir();
        assert!(dir.to_string_lossy().contains("repoverlay"));
    }

    #[test]
    fn test_cache_meta_roundtrip() {
        let meta = CacheMeta {
            clone_url: "https://github.com/owner/repo.git".to_string(),
            last_fetched: Utc::now(),
            requested_ref: "main".to_string(),
            commit: "abc123def456789012345678901234567890abcdef".to_string(),
        };

        let serialized = sickle::to_string(&meta).unwrap();
        let deserialized: CacheMeta = sickle::from_str(&serialized).unwrap();

        assert_eq!(deserialized.clone_url, meta.clone_url);
        assert_eq!(deserialized.requested_ref, meta.requested_ref);
        assert_eq!(deserialized.commit, meta.commit);
    }

    #[test]
    fn test_list_cached_empty_cache() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        let repos = manager.list_cached().unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn test_list_cached_with_repos() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create fake cached repos
        let repo1 = temp.path().join("github/owner1/repo1");
        let repo2 = temp.path().join("github/owner2/repo2");
        fs::create_dir_all(&repo1).unwrap();
        fs::create_dir_all(&repo2).unwrap();

        let repos = manager.list_cached().unwrap();
        assert_eq!(repos.len(), 2);

        // Should be sorted by owner/repo
        assert_eq!(repos[0].owner, "owner1");
        assert_eq!(repos[0].repo, "repo1");
        assert_eq!(repos[1].owner, "owner2");
        assert_eq!(repos[1].repo, "repo2");
    }

    #[test]
    fn test_list_cached_with_metadata() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create fake cached repo with metadata
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();

        let meta = CacheMeta {
            clone_url: "https://github.com/owner/repo.git".to_string(),
            last_fetched: Utc::now(),
            requested_ref: "main".to_string(),
            commit: "abc123".to_string(),
        };
        let meta_path = repo_path.join(".repoverlay-cache-meta.ccl");
        fs::write(&meta_path, sickle::to_string(&meta).unwrap()).unwrap();

        let repos = manager.list_cached().unwrap();
        assert_eq!(repos.len(), 1);
        assert!(repos[0].meta.is_some());
        assert_eq!(repos[0].meta.as_ref().unwrap().commit, "abc123");
    }

    #[test]
    fn test_clear_cache_empty() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        let count = manager.clear_cache().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_clear_cache_with_repos() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create fake cached repos
        let repo1 = temp.path().join("github/owner1/repo1");
        let repo2 = temp.path().join("github/owner1/repo2");
        fs::create_dir_all(&repo1).unwrap();
        fs::create_dir_all(&repo2).unwrap();

        let count = manager.clear_cache().unwrap();
        assert_eq!(count, 2);

        // Verify directory is removed
        assert!(!temp.path().join("github").exists());
    }

    #[test]
    fn test_remove_cached_nonexistent() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        let removed = manager.remove_cached("owner", "repo").unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_remove_cached_existing() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create fake cached repo
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();

        let removed = manager.remove_cached("owner", "repo").unwrap();
        assert!(removed);
        assert!(!repo_path.exists());
    }

    #[test]
    fn test_remove_cached_cleans_empty_owner_dir() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create single repo for owner
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();

        manager.remove_cached("owner", "repo").unwrap();

        // Owner directory should be removed since it's now empty
        assert!(!temp.path().join("github/owner").exists());
    }

    #[test]
    fn test_remove_cached_preserves_sibling_repos() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create two repos for same owner
        let repo1 = temp.path().join("github/owner/repo1");
        let repo2 = temp.path().join("github/owner/repo2");
        fs::create_dir_all(&repo1).unwrap();
        fs::create_dir_all(&repo2).unwrap();

        manager.remove_cached("owner", "repo1").unwrap();

        // Owner directory should still exist with repo2
        assert!(temp.path().join("github/owner").exists());
        assert!(repo2.exists());
    }

    #[test]
    fn test_list_cached_skips_non_directories() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create github directory with a file (not a directory)
        let github_dir = temp.path().join("github");
        fs::create_dir_all(&github_dir).unwrap();
        fs::write(github_dir.join("some_file.txt"), "content").unwrap();

        // Create actual repo directory
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();

        let repos = manager.list_cached().unwrap();
        // Should only find the actual repo, not the file
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "owner");
    }

    #[test]
    fn test_check_for_updates_nonexistent_repo() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        let source = GitHubSource::parse("https://github.com/owner/repo").unwrap();
        let result = manager.check_for_updates(&source).unwrap();

        // Should return None for non-existent repo
        assert!(result.is_none());
    }

    #[test]
    fn test_check_for_updates_tag_returns_none() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a fake cached repo
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();

        // Initialize as git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Configure git user for commit
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Create an initial commit
        fs::write(repo_path.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Parse as tag source - tags don't have "updates"
        let source = GitHubSource {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            git_ref: GitRef::Tag("v1.0.0".to_string()),
            subpath: None,
        };

        let result = manager.check_for_updates(&source).unwrap();
        // Tags don't have updates, should return None
        assert!(result.is_none());
    }

    #[test]
    fn test_check_for_updates_commit_returns_none() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a fake cached repo
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();

        // Initialize as git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Configure git user for commit
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Create an initial commit
        fs::write(repo_path.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Parse as commit source - commits don't have "updates"
        let source = GitHubSource {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            git_ref: GitRef::Commit("abc123def456".to_string()),
            subpath: None,
        };

        let result = manager.check_for_updates(&source).unwrap();
        // Commits don't have updates, should return None
        assert!(result.is_none());
    }

    #[test]
    fn test_load_meta_returns_none_for_missing_file() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a repo directory without metadata
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();

        let meta = manager.load_meta(&repo_path);
        assert!(meta.is_none());
    }

    #[test]
    fn test_load_meta_returns_none_for_invalid_content() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a repo directory with invalid metadata
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();
        fs::write(
            repo_path.join(".repoverlay-cache-meta.ccl"),
            "invalid { not valid ccl",
        )
        .unwrap();

        let meta = manager.load_meta(&repo_path);
        assert!(meta.is_none());
    }

    #[test]
    fn test_save_and_load_meta_roundtrip() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a repo directory
        let repo_path = temp.path().join("github/owner/repo");
        fs::create_dir_all(&repo_path).unwrap();

        let source = GitHubSource {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            git_ref: GitRef::Branch("main".to_string()),
            subpath: None,
        };

        // Save metadata
        manager
            .save_meta(&repo_path, &source, "abc123def456")
            .unwrap();

        // Load and verify
        let meta = manager.load_meta(&repo_path).unwrap();
        assert_eq!(meta.commit, "abc123def456");
        assert_eq!(meta.requested_ref, "main");
        assert!(meta.clone_url.contains("github.com"));
    }

    #[test]
    fn test_get_current_commit_fails_on_non_git_dir() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a directory that is not a git repo
        let non_git_path = temp.path().join("not-a-repo");
        fs::create_dir_all(&non_git_path).unwrap();

        let result = manager.get_current_commit(&non_git_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_current_commit_succeeds_on_git_repo() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a git repo with a commit
        let repo_path = temp.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        fs::write(repo_path.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let result = manager.get_current_commit(&repo_path);
        assert!(result.is_ok());
        let commit = result.unwrap();
        assert_eq!(commit.len(), 40); // SHA-1 hash is 40 hex chars
    }

    #[test]
    fn test_ref_exists_returns_true_for_existing_ref() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a git repo with a commit
        let repo_path = temp.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        fs::write(repo_path.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // HEAD should exist
        let result = manager.ref_exists(&repo_path, "HEAD").unwrap();
        assert!(result);
    }

    #[test]
    fn test_ref_exists_returns_false_for_nonexistent_ref() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create a git repo with a commit
        let repo_path = temp.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        fs::write(repo_path.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // This branch doesn't exist
        let result = manager
            .ref_exists(&repo_path, "origin/nonexistent-branch")
            .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_list_cached_skips_files_in_owner_directory() {
        let temp = TempDir::new().unwrap();
        let manager = CacheManager {
            cache_dir: temp.path().to_path_buf(),
        };

        // Create owner directory with a file instead of repo dir
        let owner_dir = temp.path().join("github/owner");
        fs::create_dir_all(&owner_dir).unwrap();
        fs::write(owner_dir.join("not-a-repo.txt"), "content").unwrap();

        // Also create a real repo
        let repo_path = temp.path().join("github/owner/real-repo");
        fs::create_dir_all(&repo_path).unwrap();

        let repos = manager.list_cached().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].repo, "real-repo");
    }
}
