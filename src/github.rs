//! GitHub URL parsing for repoverlay.
//!
//! Parses GitHub repository URLs into structured components for cloning and caching.

use anyhow::{Context, Result, anyhow, bail};
use std::path::PathBuf;
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

                (GitRef::from_str(ref_str), subpath)
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
            self.git_ref = GitRef::from_str(ref_str);
        }
        self
    }
}

impl GitRef {
    /// Parse a ref string into the appropriate type.
    ///
    /// Heuristics:
    /// - 40 hex chars = commit SHA
    /// - Otherwise = branch name (can't distinguish branch from tag at parse time)
    pub fn from_str(s: &str) -> Self {
        if s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            GitRef::Commit(s.to_string())
        } else {
            // Cannot distinguish branch from tag at parse time
            // Git will resolve it during clone/checkout
            GitRef::Branch(s.to_string())
        }
    }

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
}
