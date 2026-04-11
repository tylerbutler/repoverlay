//! Upstream repository detection for repoverlay.
//!
//! Detects the upstream (parent) repository from git remotes to enable
//! fork inheritance of overlays.

use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::github::parse_remote_url;

/// Information about an upstream repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpstreamInfo {
    /// GitHub organization/owner
    pub(crate) org: String,
    /// Repository name
    pub(crate) repo: String,
    /// Name of the remote (e.g., "upstream" or "origin")
    pub(crate) remote_name: String,
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

    let url = String::from_utf8(output.stdout)?.trim().to_string();
    Ok((!url.is_empty()).then_some(url))
}

/// Detect the upstream repository from git remotes.
///
/// Detection strategy:
/// 1. Check for a remote named "upstream" - if exists, parse its URL
/// 2. If no "upstream" remote, returns None (origin fallback requires knowing current org)
///
/// Returns `None` if no upstream can be detected.
pub(crate) fn detect_upstream(repo_path: &Path) -> Result<Option<UpstreamInfo>> {
    // First, try the "upstream" remote
    if let Some(url) = get_remote_url(repo_path, "upstream")?
        && let Some((org, repo)) = parse_remote_url(&url)
    {
        return Ok(Some(UpstreamInfo {
            org,
            repo,
            remote_name: "upstream".to_string(),
        }));
    }

    // No upstream detected
    Ok(None)
}

/// Identity of the repository the user is operating in.
///
/// Contains org/repo pairs from git remotes, used to auto-filter overlays
/// to those targeting the current repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoIdentity {
    /// org/repo from the "origin" remote
    pub(crate) origin: Option<(String, String)>,
    /// org/repo from the "upstream" remote (fork parent)
    pub(crate) upstream: Option<(String, String)>,
}

impl RepoIdentity {
    /// Check if an overlay's target org/repo matches this repository.
    ///
    /// Matches against both origin and upstream, case-insensitively.
    pub(crate) fn matches(&self, org: &str, repo: &str) -> bool {
        let matches_pair = |pair: &(String, String)| {
            pair.0.eq_ignore_ascii_case(org) && pair.1.eq_ignore_ascii_case(repo)
        };
        self.origin.as_ref().is_some_and(matches_pair)
            || self.upstream.as_ref().is_some_and(matches_pair)
    }
}

/// Detect the identity of the repository at the given path from git remotes.
///
/// Parses "origin" and "upstream" remote URLs to extract org/repo pairs.
/// Returns `None` if no GitHub remotes can be parsed.
pub(crate) fn detect_repo_identity(repo_path: &Path) -> Result<Option<RepoIdentity>> {
    let origin = get_remote_url(repo_path, "origin")?
        .as_deref()
        .and_then(parse_remote_url);
    let upstream = get_remote_url(repo_path, "upstream")?
        .as_deref()
        .and_then(parse_remote_url);

    if origin.is_none() && upstream.is_none() {
        return Ok(None);
    }

    Ok(Some(RepoIdentity { origin, upstream }))
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
        let repo = create_git_repo_with_remote(
            "upstream",
            "https://github.com/microsoft/FluidFramework.git",
        );

        let result = detect_upstream(repo.path()).unwrap();

        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.org, "microsoft");
        assert_eq!(info.repo, "FluidFramework");
        assert_eq!(info.remote_name, "upstream");
    }

    #[test]
    fn returns_none_when_no_upstream() {
        let repo = create_git_repo_with_remote(
            "origin",
            "https://github.com/tylerbutler/FluidFramework.git",
        );

        let result = detect_upstream(repo.path()).unwrap();

        // No upstream remote, so no upstream detected
        assert!(result.is_none());
    }

    #[test]
    fn handles_ssh_remote_url() {
        let repo =
            create_git_repo_with_remote("upstream", "git@github.com:microsoft/FluidFramework.git");

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

    // detect_repo_identity tests

    fn create_git_repo_with_remotes(remotes: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        for (name, url) in remotes {
            Command::new("git")
                .args(["remote", "add", name, url])
                .current_dir(dir.path())
                .output()
                .unwrap();
        }
        dir
    }

    #[test]
    fn detect_identity_from_origin() {
        let repo = create_git_repo_with_remotes(&[(
            "origin",
            "https://github.com/tylerbutler/FluidFramework.git",
        )]);

        let identity = detect_repo_identity(repo.path()).unwrap().unwrap();
        assert_eq!(
            identity.origin,
            Some(("tylerbutler".to_string(), "FluidFramework".to_string()))
        );
        assert!(identity.upstream.is_none());
    }

    #[test]
    fn detect_identity_from_both_remotes() {
        let repo = create_git_repo_with_remotes(&[
            (
                "origin",
                "https://github.com/tylerbutler/FluidFramework.git",
            ),
            (
                "upstream",
                "https://github.com/microsoft/FluidFramework.git",
            ),
        ]);

        let identity = detect_repo_identity(repo.path()).unwrap().unwrap();
        assert_eq!(
            identity.origin,
            Some(("tylerbutler".to_string(), "FluidFramework".to_string()))
        );
        assert_eq!(
            identity.upstream,
            Some(("microsoft".to_string(), "FluidFramework".to_string()))
        );
    }

    #[test]
    fn detect_identity_returns_none_for_no_remotes() {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let result = detect_repo_identity(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn repo_identity_matches_origin() {
        let id = RepoIdentity {
            origin: Some(("tylerbutler".to_string(), "FluidFramework".to_string())),
            upstream: None,
        };
        assert!(id.matches("tylerbutler", "FluidFramework"));
        assert!(!id.matches("microsoft", "FluidFramework"));
    }

    #[test]
    fn repo_identity_matches_upstream() {
        let id = RepoIdentity {
            origin: Some(("tylerbutler".to_string(), "FluidFramework".to_string())),
            upstream: Some(("microsoft".to_string(), "FluidFramework".to_string())),
        };
        assert!(id.matches("tylerbutler", "FluidFramework"));
        assert!(id.matches("microsoft", "FluidFramework"));
        assert!(!id.matches("other-org", "FluidFramework"));
    }

    #[test]
    fn repo_identity_matches_case_insensitive() {
        let id = RepoIdentity {
            origin: Some(("TylerButler".to_string(), "FluidFramework".to_string())),
            upstream: None,
        };
        assert!(id.matches("tylerbutler", "fluidframework"));
        assert!(id.matches("TYLERBUTLER", "FLUIDFRAMEWORK"));
    }
}
