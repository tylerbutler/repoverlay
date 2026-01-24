//! Upstream repository detection for repoverlay.
//!
//! Detects the upstream (parent) repository from git remotes to enable
//! fork inheritance of overlays.

use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::github::parse_remote_url;

/// Information about an upstream repository.
#[derive(Debug, Clone, PartialEq)]
pub struct UpstreamInfo {
    /// GitHub organization/owner
    pub org: String,
    /// Repository name
    pub repo: String,
    /// Name of the remote (e.g., "upstream" or "origin")
    pub remote_name: String,
}

/// Get the URL for a git remote.
fn get_remote_url(repo_path: &Path, remote_name: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["remote", "get-url", remote_name])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Ok(None);
    }

    let url = String::from_utf8(output.stdout)?
        .trim()
        .to_string();

    if url.is_empty() {
        Ok(None)
    } else {
        Ok(Some(url))
    }
}

/// Detect the upstream repository from git remotes.
///
/// Detection strategy:
/// 1. Check for a remote named "upstream" - if exists, parse its URL
/// 2. If no "upstream" remote, returns None (origin fallback requires knowing current org)
///
/// Returns `None` if no upstream can be detected.
pub fn detect_upstream(repo_path: &Path) -> Result<Option<UpstreamInfo>> {
    // First, try the "upstream" remote
    if let Some(url) = get_remote_url(repo_path, "upstream")? {
        if let Some((org, repo)) = parse_remote_url(&url) {
            return Ok(Some(UpstreamInfo {
                org,
                repo,
                remote_name: "upstream".to_string(),
            }));
        }
    }

    // No upstream detected
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_git_repo_with_remote(remote_name: &str, remote_url: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["remote", "add", remote_name, remote_url])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn detects_upstream_remote() {
        let repo = create_git_repo_with_remote("upstream", "https://github.com/microsoft/FluidFramework.git");

        let result = detect_upstream(repo.path()).unwrap();

        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.org, "microsoft");
        assert_eq!(info.repo, "FluidFramework");
        assert_eq!(info.remote_name, "upstream");
    }

    #[test]
    fn returns_none_when_no_upstream() {
        let repo = create_git_repo_with_remote("origin", "https://github.com/tylerbutler/FluidFramework.git");

        let result = detect_upstream(repo.path()).unwrap();

        // No upstream remote, so no upstream detected
        assert!(result.is_none());
    }

    #[test]
    fn handles_ssh_remote_url() {
        let repo = create_git_repo_with_remote("upstream", "git@github.com:microsoft/FluidFramework.git");

        let result = detect_upstream(repo.path()).unwrap();

        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.org, "microsoft");
        assert_eq!(info.repo, "FluidFramework");
    }

    #[test]
    fn returns_none_for_non_github_remote() {
        let repo = create_git_repo_with_remote("upstream", "https://gitlab.com/org/repo.git");

        let result = detect_upstream(repo.path()).unwrap();

        assert!(result.is_none());
    }
}
