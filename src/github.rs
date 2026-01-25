//! GitHub URL parsing for repoverlay.
//!
//! Parses GitHub repository URLs into structured components for cloning and caching.

use anyhow::{Context, Result, anyhow, bail};
use std::path::PathBuf;
use std::str::FromStr;
use url::Url;

/// Parsed GitHub URL components.
#[derive(Debug, Clone, PartialEq)]
pub struct GitHubSource {
    pub owner: String,
    pub repo: String,
    pub git_ref: GitRef,
    pub subpath: Option<PathBuf>,
}

/// Git reference type.
#[derive(Debug, Clone, PartialEq)]
pub enum GitRef {
    /// Use repository's default branch
    Default,
    /// A branch name
    Branch(String),
    /// A tag name (currently parsed as Branch, kept for future use)
    #[allow(dead_code)]
    Tag(String),
    /// A commit SHA (40 hex characters)
    Commit(String),
}

impl GitHubSource {
    /// Parse a GitHub URL into its components.
    ///
    /// Supported formats:
    /// - `https://github.com/owner/repo`
    /// - `https://github.com/owner/repo.git`
    /// - `https://github.com/owner/repo/tree/branch`
    /// - `https://github.com/owner/repo/tree/branch/path/to/subdir`
    /// - `https://github.com/owner/repo/tree/v1.0.0`
    /// - `https://github.com/owner/repo/tree/abc123...` (commit SHA)
    pub fn parse(input: &str) -> Result<Self> {
        let url = Url::parse(input).with_context(|| format!("Invalid URL: {}", input))?;

        if url.host_str() != Some("github.com") {
            bail!("Not a GitHub URL: {}", input);
        }

        // Extract path segments: /owner/repo[/tree/ref/subpath]
        let path = url.path().trim_start_matches('/');
        let segments: Vec<&str> = path.split('/').collect();

        if segments.len() < 2 || segments[0].is_empty() || segments[1].is_empty() {
            bail!("Invalid GitHub URL - missing owner/repo: {}", input);
        }

        let owner = segments[0].to_string();
        let repo = segments[1].trim_end_matches(".git").to_string();

        let (git_ref, subpath) = if segments.len() > 2 {
            if segments[2] == "tree" {
                // Has ref and possibly subpath
                let ref_str = segments
                    .get(3)
                    .ok_or_else(|| anyhow!("Missing ref after /tree/ in URL: {}", input))?;

                let subpath = if segments.len() > 4 {
                    Some(PathBuf::from(segments[4..].join("/")))
                } else {
                    None
                };

                (ref_str.parse().unwrap(), subpath)
            } else if segments[2] == "blob" {
                // User pasted a file URL instead of tree URL
                bail!(
                    "Invalid GitHub URL: use /tree/ URLs for directories, not /blob/ URLs for files"
                );
            } else {
                // Unknown path component, treat as default ref
                (GitRef::Default, None)
            }
        } else {
            (GitRef::Default, None)
        };

        Ok(GitHubSource {
            owner,
            repo,
            git_ref,
            subpath,
        })
    }

    /// Check if a string looks like a GitHub URL.
    pub fn is_github_url(input: &str) -> bool {
        input.starts_with("https://github.com/") || input.starts_with("http://github.com/")
    }

    /// Generate a unique cache directory name.
    #[allow(dead_code)]
    pub fn cache_key(&self) -> String {
        let ref_part = match &self.git_ref {
            GitRef::Default => "default".to_string(),
            GitRef::Branch(b) => format!("branch-{}", sanitize_for_path(b)),
            GitRef::Tag(t) => format!("tag-{}", sanitize_for_path(t)),
            GitRef::Commit(c) => format!("commit-{}", &c[..12.min(c.len())]),
        };
        format!("{}__{}__{}", self.owner, self.repo, ref_part)
    }

    /// Full clone URL for the repository.
    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.repo)
    }

    /// Human-readable display of the source.
    #[allow(dead_code)]
    pub fn display_url(&self) -> String {
        let base = format!("https://github.com/{}/{}", self.owner, self.repo);
        match (&self.git_ref, &self.subpath) {
            (GitRef::Default, None) => base,
            (GitRef::Default, Some(path)) => format!("{}/tree/HEAD/{}", base, path.display()),
            (ref_, None) => format!("{}/tree/{}", base, ref_.as_str()),
            (ref_, Some(path)) => format!("{}/tree/{}/{}", base, ref_.as_str(), path.display()),
        }
    }

    /// Apply a ref override from CLI.
    pub fn with_ref_override(mut self, ref_override: Option<&str>) -> Self {
        if let Some(ref_str) = ref_override {
            self.git_ref = ref_str.parse().unwrap();
        }
        self
    }
}

impl FromStr for GitRef {
    type Err = std::convert::Infallible;

    /// Parse a ref string into the appropriate type.
    ///
    /// Heuristics:
    /// - 40 hex chars = commit SHA
    /// - Otherwise = branch name (can't distinguish branch from tag at parse time)
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(GitRef::Commit(s.to_string()))
        } else {
            // Cannot distinguish branch from tag at parse time
            // Git will resolve it during clone/checkout
            Ok(GitRef::Branch(s.to_string()))
        }
    }
}

impl GitRef {
    /// Get the ref as a string for display and storage.
    pub fn as_str(&self) -> &str {
        match self {
            GitRef::Default => "HEAD",
            GitRef::Branch(s) | GitRef::Tag(s) | GitRef::Commit(s) => s,
        }
    }

    /// Check if this is the default ref.
    #[allow(dead_code)]
    pub fn is_default(&self) -> bool {
        matches!(self, GitRef::Default)
    }
}

impl std::fmt::Display for GitRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitRef::Default => write!(f, "(default branch)"),
            GitRef::Branch(s) => write!(f, "branch:{}", s),
            GitRef::Tag(s) => write!(f, "tag:{}", s),
            GitRef::Commit(s) => write!(f, "commit:{}", &s[..12.min(s.len())]),
        }
    }
}

/// Parse owner/repo from a git remote URL (HTTPS or SSH format).
///
/// Returns `None` if the URL is not a GitHub URL or cannot be parsed.
pub fn parse_remote_url(url: &str) -> Option<(String, String)> {
    // Must be a GitHub URL
    if !url.contains("github.com") {
        return None;
    }

    // Handle SSH format: git@github.com:owner/repo.git
    if let Some(path) = url.strip_prefix("git@github.com:") {
        let path = path.trim_end_matches(".git");
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Some((parts[0].to_string(), parts[1].to_string()));
        }
        return None;
    }

    // Handle HTTPS format: https://github.com/owner/repo.git
    let path = url
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/")
        .trim_end_matches(".git");

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        return Some((parts[0].to_string(), parts[1].to_string()));
    }

    None
}

/// Sanitize a string for use in a filesystem path.
#[allow(dead_code)]
fn sanitize_for_path(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_url() {
        let source = GitHubSource::parse("https://github.com/owner/repo").unwrap();
        assert_eq!(source.owner, "owner");
        assert_eq!(source.repo, "repo");
        assert_eq!(source.git_ref, GitRef::Default);
        assert_eq!(source.subpath, None);
    }

    #[test]
    fn test_parse_url_with_git_suffix() {
        let source = GitHubSource::parse("https://github.com/owner/repo.git").unwrap();
        assert_eq!(source.repo, "repo");
    }

    #[test]
    fn test_parse_url_with_branch() {
        let source = GitHubSource::parse("https://github.com/owner/repo/tree/main").unwrap();
        assert_eq!(source.git_ref, GitRef::Branch("main".to_string()));
        assert_eq!(source.subpath, None);
    }

    #[test]
    fn test_parse_url_with_branch_and_subpath() {
        let source =
            GitHubSource::parse("https://github.com/owner/repo/tree/main/path/to/overlay").unwrap();
        assert_eq!(source.git_ref, GitRef::Branch("main".to_string()));
        assert_eq!(source.subpath, Some(PathBuf::from("path/to/overlay")));
    }

    #[test]
    fn test_parse_url_with_tag() {
        let source = GitHubSource::parse("https://github.com/owner/repo/tree/v1.0.0").unwrap();
        // Note: v1.0.0 is parsed as a branch since we can't distinguish at parse time
        assert_eq!(source.git_ref, GitRef::Branch("v1.0.0".to_string()));
    }

    #[test]
    fn test_parse_url_with_commit() {
        let source = GitHubSource::parse(
            "https://github.com/owner/repo/tree/abc123def456789012345678901234567890abcd",
        )
        .unwrap();
        assert!(matches!(source.git_ref, GitRef::Commit(_)));
    }

    #[test]
    fn test_parse_url_with_subpath_deep() {
        let source =
            GitHubSource::parse("https://github.com/org/monorepo/tree/develop/packages/overlay-a")
                .unwrap();
        assert_eq!(source.owner, "org");
        assert_eq!(source.repo, "monorepo");
        assert_eq!(source.git_ref, GitRef::Branch("develop".to_string()));
        assert_eq!(source.subpath, Some(PathBuf::from("packages/overlay-a")));
    }

    #[test]
    fn test_reject_non_github_url() {
        let result = GitHubSource::parse("https://gitlab.com/owner/repo");
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_blob_url() {
        let result = GitHubSource::parse("https://github.com/owner/repo/blob/main/file.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("blob"));
    }

    #[test]
    fn test_reject_invalid_url() {
        let result = GitHubSource::parse("not a url");
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_missing_repo() {
        let result = GitHubSource::parse("https://github.com/owner");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_github_url() {
        assert!(GitHubSource::is_github_url("https://github.com/owner/repo"));
        assert!(GitHubSource::is_github_url("http://github.com/owner/repo"));
        assert!(!GitHubSource::is_github_url("./local/path"));
        assert!(!GitHubSource::is_github_url(
            "https://gitlab.com/owner/repo"
        ));
    }

    #[test]
    fn test_clone_url() {
        let source = GitHubSource::parse("https://github.com/owner/repo/tree/main/subdir").unwrap();
        assert_eq!(source.clone_url(), "https://github.com/owner/repo.git");
    }

    #[test]
    fn test_cache_key() {
        let source = GitHubSource::parse("https://github.com/owner/repo").unwrap();
        assert_eq!(source.cache_key(), "owner__repo__default");

        let source = GitHubSource::parse("https://github.com/owner/repo/tree/main").unwrap();
        assert_eq!(source.cache_key(), "owner__repo__branch-main");

        let source = GitHubSource::parse(
            "https://github.com/owner/repo/tree/abc123def456789012345678901234567890abcd",
        )
        .unwrap();
        assert_eq!(source.cache_key(), "owner__repo__commit-abc123def456");
    }

    #[test]
    fn test_with_ref_override() {
        let source = GitHubSource::parse("https://github.com/owner/repo")
            .unwrap()
            .with_ref_override(Some("develop"));
        assert_eq!(source.git_ref, GitRef::Branch("develop".to_string()));
    }

    #[test]
    fn test_display_url() {
        let source = GitHubSource::parse("https://github.com/owner/repo").unwrap();
        assert_eq!(source.display_url(), "https://github.com/owner/repo");

        let source = GitHubSource::parse("https://github.com/owner/repo/tree/main").unwrap();
        assert_eq!(
            source.display_url(),
            "https://github.com/owner/repo/tree/main"
        );

        let source = GitHubSource::parse("https://github.com/owner/repo/tree/main/subdir").unwrap();
        assert_eq!(
            source.display_url(),
            "https://github.com/owner/repo/tree/main/subdir"
        );
    }

    #[test]
    fn test_display_url_default_with_subpath() {
        // Create source with default ref but with subpath
        let mut source = GitHubSource::parse("https://github.com/owner/repo").unwrap();
        source.subpath = Some(PathBuf::from("some/path"));

        assert_eq!(
            source.display_url(),
            "https://github.com/owner/repo/tree/HEAD/some/path"
        );
    }

    #[test]
    fn test_git_ref_is_default() {
        assert!(GitRef::Default.is_default());
        assert!(!GitRef::Branch("main".to_string()).is_default());
        assert!(!GitRef::Tag("v1.0".to_string()).is_default());
        assert!(!GitRef::Commit("abc123".to_string()).is_default());
    }

    #[test]
    fn test_git_ref_display() {
        assert_eq!(format!("{}", GitRef::Default), "(default branch)");
        assert_eq!(
            format!("{}", GitRef::Branch("main".to_string())),
            "branch:main"
        );
        assert_eq!(
            format!("{}", GitRef::Tag("v1.0.0".to_string())),
            "tag:v1.0.0"
        );
        assert_eq!(
            format!(
                "{}",
                GitRef::Commit("abc123def456789012345678901234567890abcdef".to_string())
            ),
            "commit:abc123def456"
        );
    }

    #[test]
    fn test_git_ref_display_short_commit() {
        // Commit shorter than 12 chars should display in full
        assert_eq!(
            format!("{}", GitRef::Commit("abc123".to_string())),
            "commit:abc123"
        );
    }

    #[test]
    fn test_cache_key_tag() {
        // Create a source and manually set it to a tag
        let mut source = GitHubSource::parse("https://github.com/owner/repo").unwrap();
        source.git_ref = GitRef::Tag("v1.0.0".to_string());

        assert_eq!(source.cache_key(), "owner__repo__tag-v1.0.0");
    }

    #[test]
    fn test_sanitize_for_path() {
        assert_eq!(sanitize_for_path("main"), "main");
        assert_eq!(sanitize_for_path("feature/branch"), "feature_branch");
        assert_eq!(sanitize_for_path("v1.0.0"), "v1.0.0");
        assert_eq!(sanitize_for_path("branch-name_123"), "branch-name_123");
        assert_eq!(sanitize_for_path("special!@#chars"), "special___chars");
    }

    #[test]
    fn test_with_ref_override_none() {
        let source = GitHubSource::parse("https://github.com/owner/repo/tree/main")
            .unwrap()
            .with_ref_override(None);

        // Should keep original ref
        assert_eq!(source.git_ref, GitRef::Branch("main".to_string()));
    }

    #[test]
    fn test_with_ref_override_commit() {
        // Use exactly 40 hex characters for a valid commit SHA
        let source = GitHubSource::parse("https://github.com/owner/repo")
            .unwrap()
            .with_ref_override(Some("abcd1234abcd1234abcd1234abcd1234abcd1234"));

        // 40 hex chars is detected as commit
        if let GitRef::Commit(sha) = &source.git_ref {
            assert_eq!(sha, "abcd1234abcd1234abcd1234abcd1234abcd1234");
        } else {
            panic!("Expected GitRef::Commit");
        }
    }

    #[test]
    fn test_git_ref_from_str_branch() {
        // Non-40 hex char string should be parsed as branch
        let git_ref: GitRef = "main".parse().unwrap();
        assert_eq!(git_ref, GitRef::Branch("main".to_string()));

        let git_ref: GitRef = "feature-branch".parse().unwrap();
        assert_eq!(git_ref, GitRef::Branch("feature-branch".to_string()));
    }

    #[test]
    fn test_git_ref_from_str_commit() {
        // Exactly 40 hex chars should be parsed as commit
        let git_ref: GitRef = "abcd1234abcd1234abcd1234abcd1234abcd1234".parse().unwrap();
        assert!(matches!(git_ref, GitRef::Commit(_)));
    }

    #[test]
    fn test_git_ref_from_str_not_commit_wrong_length() {
        // Less than 40 chars should not be commit
        let git_ref: GitRef = "abc123".parse().unwrap();
        assert!(matches!(git_ref, GitRef::Branch(_)));
    }

    #[test]
    fn test_git_ref_from_str_not_commit_non_hex() {
        // 40 chars but not all hex should not be commit
        let git_ref: GitRef = "ghijklmnopqrstuvwxyz1234567890abcdefghij".parse().unwrap();
        assert!(matches!(git_ref, GitRef::Branch(_)));
    }

    #[test]
    fn test_parse_unknown_path_component() {
        // URLs with unknown path components after repo should use default ref
        let source = GitHubSource::parse("https://github.com/owner/repo/issues");
        // This will succeed but with default ref (unknown path component is ignored)
        assert!(source.is_ok());
        let source = source.unwrap();
        assert_eq!(source.git_ref, GitRef::Default);
    }

    #[test]
    fn test_git_ref_as_str() {
        assert_eq!(GitRef::Default.as_str(), "HEAD");
        assert_eq!(GitRef::Branch("main".to_string()).as_str(), "main");
        assert_eq!(GitRef::Tag("v1.0".to_string()).as_str(), "v1.0");
        assert_eq!(GitRef::Commit("abc123".to_string()).as_str(), "abc123");
    }

    #[test]
    fn test_parse_remote_url_ssh() {
        let result = parse_remote_url("git@github.com:owner/repo.git");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_remote_url_ssh_no_suffix() {
        let result = parse_remote_url("git@github.com:owner/repo");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_remote_url_https() {
        let result = parse_remote_url("https://github.com/owner/repo.git");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_remote_url_non_github() {
        let result = parse_remote_url("git@gitlab.com:owner/repo.git");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_remote_url_ssh_invalid_format() {
        // SSH URL with empty owner
        let result = parse_remote_url("git@github.com:/repo");
        assert_eq!(result, None);

        // SSH URL with empty repo
        let result = parse_remote_url("git@github.com:owner/");
        assert_eq!(result, None);

        // SSH URL with just owner, no repo
        let result = parse_remote_url("git@github.com:owner");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_remote_url_https_invalid_format() {
        // HTTPS URL with empty owner
        let result = parse_remote_url("https://github.com//repo");
        assert_eq!(result, None);

        // HTTPS URL with empty repo
        let result = parse_remote_url("https://github.com/owner/");
        assert_eq!(result, None);
    }
}
