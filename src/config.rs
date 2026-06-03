//! Configuration management for repoverlay.
//!
//! Handles global and per-repo configuration using CCL format.
//! Global config: `~/.config/repoverlay/config.ccl`
//! Per-repo config: `.repoverlay/config.ccl`

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Global repoverlay configuration.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub(crate) struct RepoverlayConfig {
    /// Configured overlay sources (checked in order for resolution).
    #[serde(default)]
    pub(crate) sources: Vec<Source>,
    /// Custom library path (per-repo only, relative to repo root).
    /// When not set, defaults to `.repoverlay/library/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) library_path: Option<String>,
    /// Named profile definitions.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(crate) profiles: std::collections::BTreeMap<String, crate::profile::ProfileConfig>,
}

impl RepoverlayConfig {
    /// Get an `OverlayRepoConfig` from the first configured source.
    ///
    /// Commands that need a single overlay repo (create, inspect, sync, etc.)
    /// should use this method.
    pub(crate) fn get_default_overlay_repo_config(&self) -> Result<OverlayRepoConfig> {
        let source = self.sources.first().ok_or_else(|| {
            anyhow::anyhow!(
                "Overlay repository not configured.\n\n\
                 Run 'repoverlay source add <url>' to set up an overlay source."
            )
        })?;

        let cache_dir = directories::ProjectDirs::from("", "", "repoverlay")
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        let local_path = cache_dir.cache_dir().join("sources").join(&source.name);
        Ok(OverlayRepoConfig {
            url: source.url()?.to_string(),
            local_path: Some(local_path),
        })
    }

    /// Get an `OverlayRepoConfig` for a specific named source.
    ///
    /// Looks up the source by name in the configured sources list.
    /// Falls back to `get_default_overlay_repo_config` if `source_name` is `None`.
    pub(crate) fn get_overlay_repo_config_by_name(
        &self,
        source_name: Option<&str>,
    ) -> Result<OverlayRepoConfig> {
        let Some(name) = source_name else {
            return self.get_default_overlay_repo_config();
        };

        let source = self
            .sources
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Source '{name}' not found in configuration.\n\n\
                 Available sources: {}",
                    self.sources
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;

        let cache_dir = directories::ProjectDirs::from("", "", "repoverlay")
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        let local_path = cache_dir.cache_dir().join("sources").join(&source.name);

        Ok(OverlayRepoConfig {
            url: source.url()?.to_string(),
            local_path: Some(local_path),
        })
    }
}

/// An overlay source repository.
///
/// Sources are checked in order when resolving overlay references.
/// Earlier sources have higher priority.
///
/// Each source must have exactly one of `url` (git-backed) or `path` (local directory).
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub(crate) struct Source {
    /// Name for this source (used in CLI output and `--source` flag).
    pub(crate) name: String,
    /// Git URL of the overlay repository.
    /// Accepts full URLs or GitHub shorthand (`owner/repo`), which is expanded
    /// to `https://github.com/owner/repo` during deserialization.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_source_url",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) url: Option<String>,
    /// Local directory path for the overlay source (repo-relative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<PathBuf>,
}

impl Source {
    /// Returns `true` if this is a local directory source.
    pub(crate) const fn is_local(&self) -> bool {
        self.path.is_some()
    }

    /// Returns `true` if this is a git-based source.
    #[allow(dead_code)] // Utility method for callers
    pub(crate) const fn is_git(&self) -> bool {
        self.url.is_some()
    }

    /// Get the URL. Returns an error if this is a local source.
    pub(crate) fn url(&self) -> Result<&str> {
        self.url.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Source '{}' is a local path source, not a git source",
                self.name
            )
        })
    }

    /// Get the path. Returns an error if this is a git source.
    pub(crate) fn path(&self) -> Result<&Path> {
        self.path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Source '{}' is a git source, not a local path source",
                self.name
            )
        })
    }

    /// Validate that exactly one of url or path is set.
    #[allow(dead_code)] // Utility method; used in tests
    pub(crate) fn validate(&self) -> Result<()> {
        match (&self.url, &self.path) {
            (Some(_), None) | (None, Some(_)) => Ok(()),
            (Some(_), Some(_)) => anyhow::bail!(
                "Source '{}' has both url and path; only one is allowed",
                self.name
            ),
            (None, None) => anyhow::bail!(
                "Source '{}' has neither url nor path; one is required",
                self.name
            ),
        }
    }
}

/// Default overlay repository name for the one-part shorthand syntax.
/// When user types `username`, it expands to `username/repo-overlays`.
pub(crate) const DEFAULT_OVERLAY_REPO_NAME: &str = "repo-overlays";

/// Returns the overlay repository name for the one-part shorthand syntax.
///
/// Checks `REPOVERLAY_DEFAULT_REPO_NAME` env var first, falling back to
/// [`DEFAULT_OVERLAY_REPO_NAME`].
#[must_use]
pub(crate) fn default_overlay_repo_name() -> String {
    default_overlay_repo_name_with_env(std::env::var("REPOVERLAY_DEFAULT_REPO_NAME").ok())
}

/// Testable inner function: resolve default overlay repo name from an optional env value.
#[must_use]
fn default_overlay_repo_name_with_env(env_val: Option<String>) -> String {
    env_val.unwrap_or_else(|| DEFAULT_OVERLAY_REPO_NAME.to_string())
}

/// Parsed source URL input from the CLI.
///
/// Represents the three valid input formats for overlay source URLs:
/// - Full git URL (`https://...`, `git@...`)
/// - GitHub shorthand (`owner/repo`)
/// - Bare GitHub owner/username (`owner`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceUrlInput {
    /// A full git-cloneable URL.
    GitUrl(String),
    /// GitHub shorthand (`owner/repo`), expanded to `https://github.com/owner/repo`.
    GitHubShorthand { owner: String, repo: String },
    /// Bare owner name, expanded to `https://github.com/owner/{default_repo}`.
    BareOwner(String),
    /// A local directory path for overlay sources within the repo.
    LocalPath(PathBuf),
    /// A `file://` URL pointing to a local directory (possibly external to the repo).
    FileUrl {
        /// The local filesystem path (stripped of `file://` prefix).
        path: PathBuf,
        /// The original `file://` URL string for display and storage.
        original: String,
    },
}

impl SourceUrlInput {
    /// Returns the expanded git URL for this input.
    ///
    /// # Panics
    ///
    /// Panics if called on a `LocalPath` variant.
    #[must_use]
    pub(crate) fn to_url(&self) -> String {
        self.to_url_with_repo_name(&default_overlay_repo_name())
    }

    /// Testable version that accepts the default repo name as a parameter.
    #[must_use]
    fn to_url_with_repo_name(&self, default_repo: &str) -> String {
        match self {
            Self::GitUrl(url) => url.clone(),
            Self::GitHubShorthand { owner, repo } => {
                expand_github_shorthand(&format!("{owner}/{repo}"))
            }
            Self::BareOwner(owner) => expand_github_shorthand(&format!("{owner}/{default_repo}")),
            Self::FileUrl { original, .. } => original.clone(),
            Self::LocalPath(_) => panic!("to_url() called on LocalPath variant"),
        }
    }

    /// Returns `true` if this is a local path source (not a `file://` URL).
    #[must_use]
    pub(crate) const fn is_local(&self) -> bool {
        matches!(self, Self::LocalPath(_))
    }

    /// Returns `true` if this is a `file://` URL source.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn is_file_url(&self) -> bool {
        matches!(self, Self::FileUrl { .. })
    }

    /// Returns the local path. Panics if called on a git variant.
    #[must_use]
    pub(crate) fn local_path(&self) -> &Path {
        match self {
            Self::LocalPath(p) | Self::FileUrl { path: p, .. } => p,
            _ => panic!("local_path() called on non-LocalPath variant"),
        }
    }
}

impl FromStr for SourceUrlInput {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.is_empty() || s.chars().all(char::is_whitespace) {
            return Err(
                "Invalid source URL: input cannot be empty. Expected a git URL (https://...), \
                 GitHub shorthand (owner/repo), or a GitHub username (owner)."
                    .to_string(),
            );
        }

        // Check for file:// URLs — local path via URL syntax.
        if let Some(path) = s.strip_prefix("file://") {
            if path.is_empty() {
                return Err(
                    "Invalid file:// URL: path cannot be empty. Use file:///path/to/directory."
                        .to_string(),
                );
            }
            return Ok(Self::FileUrl {
                path: PathBuf::from(path),
                original: s.to_string(),
            });
        }

        // Check for local path indicators before git URL checks.
        // Require at least a directory name beyond the prefix.
        let trimmed = s.trim_end_matches('/');
        if (trimmed.starts_with("./") && trimmed.len() > 2)
            || (trimmed.starts_with("../") && trimmed.len() > 3)
            || (trimmed.starts_with('/') && trimmed.len() > 1)
            || trimmed.starts_with('~')
        {
            return Ok(Self::LocalPath(PathBuf::from(s)));
        }

        if is_git_url(s) {
            Ok(Self::GitUrl(s.to_string()))
        } else if is_github_shorthand(s) {
            let parts: Vec<&str> = s.split('/').collect();
            Ok(Self::GitHubShorthand {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
            })
        } else if is_bare_owner(s) {
            Ok(Self::BareOwner(s.to_string()))
        } else {
            Err(format!(
                "Invalid source URL: '{s}'. Expected a git URL (https://...), \
                 GitHub shorthand (owner/repo), or a GitHub username (owner)."
            ))
        }
    }
}

impl std::fmt::Display for SourceUrlInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitUrl(url) => write!(f, "{url}"),
            Self::GitHubShorthand { owner, repo } => write!(f, "{owner}/{repo}"),
            Self::BareOwner(owner) => write!(f, "{owner}"),
            Self::LocalPath(path) => write!(f, "{}", path.display()),
            Self::FileUrl { original, .. } => write!(f, "{original}"),
        }
    }
}

/// Check if a string looks like a git-cloneable URL.
fn is_git_url(s: &str) -> bool {
    s.contains("://") || s.starts_with("git@")
}

/// Check if a string is valid GitHub shorthand (`owner/repo`).
fn is_github_shorthand(s: &str) -> bool {
    let parts: Vec<&str> = s.split('/').collect();
    parts.len() == 2
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && !parts[0].contains(char::is_whitespace)
        && !parts[1].contains(char::is_whitespace)
}

/// Check if a string is a bare owner name (single word, no slashes).
///
/// Valid: `tylerbutler`, `my-org`, `user123`
/// Invalid: empty, whitespace, contains `/`
fn is_bare_owner(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains(char::is_whitespace)
}

/// Expand GitHub shorthand to a full URL.
#[must_use]
pub(crate) fn expand_github_shorthand(s: &str) -> String {
    format!("https://github.com/{s}")
}

/// Validate and normalize a source URL string.
///
/// Accepts:
/// - Full git URLs (`https://...`, `git@...`)
/// - GitHub shorthand (`owner/repo`) - expanded to `https://github.com/owner/repo`
/// - Bare owner name (`owner`) - expanded to `https://github.com/owner/repo-overlays`
///
/// Returns an error for invalid formats (empty, whitespace).
pub(crate) fn validate_source_url(url: &str) -> std::result::Result<String, String> {
    if is_git_url(url) {
        Ok(url.to_string())
    } else if is_github_shorthand(url) {
        Ok(expand_github_shorthand(url))
    } else if is_bare_owner(url) {
        let repo_name = default_overlay_repo_name();
        Ok(expand_github_shorthand(&format!("{url}/{repo_name}")))
    } else {
        Err(format!(
            "Invalid source URL: '{url}'. Expected a git URL (https://...), \
             GitHub shorthand (owner/repo), or a GitHub username (owner)."
        ))
    }
}

/// Custom deserializer for optional source URLs.
fn deserialize_optional_source_url<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    raw.map_or_else(
        || Ok(None),
        |s| {
            validate_source_url(&s)
                .map(Some)
                .map_err(serde::de::Error::custom)
        },
    )
}

/// Configuration for a shared overlay repository.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct OverlayRepoConfig {
    /// Git URL of the overlay repository.
    pub(crate) url: String,
    /// Optional override for the local clone path.
    /// Default: `~/.local/share/repoverlay/overlay-repo/`
    #[serde(default)]
    pub(crate) local_path: Option<PathBuf>,
}

/// Get the global config directory path.
///
/// Returns `~/.config/repoverlay/` on all Unix-like systems.
/// Respects `XDG_CONFIG_HOME` if set.
pub(crate) fn config_dir() -> Result<PathBuf> {
    config_dir_with_env(std::env::var("XDG_CONFIG_HOME").ok().as_deref())
}

/// Testable inner function: resolve config dir from an optional XDG value.
fn config_dir_with_env(xdg: Option<&str>) -> Result<PathBuf> {
    let base = if let Some(xdg) = xdg {
        PathBuf::from(xdg)
    } else {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
            .join(".config")
    };

    Ok(base.join("repoverlay"))
}

/// Get the path to the global config file.
pub(crate) fn global_config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.ccl"))
}

/// Returns the path to the per-repo config file: `<repo_root>/.repoverlay/config.ccl`
pub(crate) fn repo_config_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".repoverlay").join("config.ccl")
}

/// Load the global configuration.
pub(crate) fn load_global_config() -> Result<RepoverlayConfig> {
    let config_path = global_config_path()?;

    if !config_path.exists() {
        return Ok(RepoverlayConfig::default());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

    let config: RepoverlayConfig = sickle::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;

    Ok(config)
}

/// Load the per-repo configuration.
pub(crate) fn load_repo_config(repo_root: &Path) -> Result<Option<RepoverlayConfig>> {
    let config_path = repo_config_path(repo_root);

    if !config_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read repo config: {}", config_path.display()))?;

    let config: RepoverlayConfig = sickle::from_str(&content)
        .with_context(|| format!("Failed to parse repo config: {}", config_path.display()))?;

    Ok(Some(config))
}

/// Load the repoverlay configuration, merging global and per-repo configs.
///
/// When `repo_path` is provided, repo-local sources are loaded first (higher priority),
/// followed by global sources. When `None`, only global config is loaded.
pub(crate) fn load_config(repo_path: Option<&Path>) -> Result<RepoverlayConfig> {
    let config = load_global_config()?;

    if let Some(repo_root) = repo_path
        && let Some(repo_config) = load_repo_config(repo_root)?
    {
        return Ok(merge_repo_config(config, repo_config));
    }

    Ok(config)
}

pub(crate) fn merge_repo_config(
    mut global: RepoverlayConfig,
    repo_config: RepoverlayConfig,
) -> RepoverlayConfig {
    let mut merged_sources = repo_config.sources;
    merged_sources.extend(global.sources);
    global.sources = merged_sources;

    if repo_config.library_path.is_some() {
        global.library_path = repo_config.library_path;
    }

    for (name, repo_profile) in repo_config.profiles {
        let merged_profile = global.profiles.get(&name).map_or_else(
            || repo_profile.clone(),
            |base| crate::profile::merge_profile_config(base, &repo_profile),
        );
        global.profiles.insert(name, merged_profile);
    }

    global
}

/// Generate a config file for multi-source configuration.
//
// TODO(santa#205, santa#206): this re-serializes the typed struct via
// `sickle::to_string`, which DROPS user comments and blank lines on every save
// (verified: `source add` reduces a commented config to zero comments). Once
// sickle ships faithful comment round-trip (santa#205) and a
// comment/format-preserving read-modify-write API (santa#206), switch
// `save_config`/`save_repo_config` to load the existing document, merge the
// updated data in, and reprint — preserving the user's comments. Until then,
// edits to a commented config must be made by hand. Land before this long-lived
// `profiles` branch merges.
pub(crate) fn generate_sources_config_ccl(config: &RepoverlayConfig) -> String {
    sickle::to_string(config).expect("RepoverlayConfig serialization should not fail")
}

/// Save the global configuration.
pub(crate) fn save_config(config: &RepoverlayConfig) -> Result<()> {
    let config_path = global_config_path()?;

    // Ensure config directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }

    let content = generate_sources_config_ccl(config);

    fs::write(&config_path, content)
        .with_context(|| format!("Failed to write config file: {}", config_path.display()))?;

    Ok(())
}

/// Save per-repo configuration to `<repo_root>/.repoverlay/config.ccl`.
pub(crate) fn save_repo_config(repo_root: &Path, config: &RepoverlayConfig) -> Result<()> {
    let config_path = repo_config_path(repo_root);

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }

    let content = generate_sources_config_ccl(config);
    fs::write(&config_path, content)
        .with_context(|| format!("Failed to write repo config: {}", config_path.display()))?;

    // Ensure .repoverlay is in .git/info/exclude so it doesn't show as untracked.
    // Best-effort: skip if not in a git repo (e.g., during tests).
    let _ = crate::ensure_repoverlay_excluded(repo_root);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_dir() {
        let dir = config_dir();
        assert!(dir.is_ok());
        let dir = dir.unwrap();
        assert!(dir.ends_with("repoverlay") || dir.to_string_lossy().contains("repoverlay"));
    }

    #[test]
    fn test_repo_config_path() {
        let repo = PathBuf::from("/some/repo");
        let path = repo_config_path(&repo);
        assert_eq!(path, PathBuf::from("/some/repo/.repoverlay/config.ccl"));
    }

    #[test]
    fn test_load_global_config_missing() {
        // Should return default config when file doesn't exist
        let config = load_global_config();
        if let Ok(cfg) = config {
            // Only assert if no global config file exists (i.e., we got defaults)
            if !global_config_path().is_ok_and(|p| p.exists()) {
                assert!(cfg.sources.is_empty());
            }
        }
    }

    #[test]
    fn test_load_repo_config_missing() {
        let temp = TempDir::new().unwrap();
        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_none());
    }

    #[test]
    fn test_load_repo_config_valid() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        let config_content = r"
sources =
  =
    name = default
    url = https://github.com/org/overlays
";
        fs::write(config_dir.join("config.ccl"), config_content).unwrap();

        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.sources.len(), 1);
        assert_eq!(
            config.sources[0].url().unwrap(),
            "https://github.com/org/overlays"
        );
    }

    #[test]
    fn test_load_config_merges_same_name_profiles() {
        let temp = TempDir::new().unwrap();
        let repo_config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&repo_config_dir).unwrap();

        let repo_ccl = r"
profiles =
  rust-dev =
    overlays =
      = repo-rust
    mcps =
      servers =
        repo =
          command = repo-mcp
";
        fs::write(repo_config_dir.join("config.ccl"), repo_ccl).unwrap();

        let global = RepoverlayConfig {
            profiles: std::collections::BTreeMap::from([(
                "rust-dev".to_string(),
                crate::profile::ProfileConfig {
                    description: Some("Global Rust".to_string()),
                    overlays: vec!["global-rust".to_string()],
                    instructions: vec![crate::profile::InstructionConfig {
                        source: "global.md".to_string(),
                    }],
                    mcps: crate::profile::McpConfig {
                        servers: std::collections::BTreeMap::from([(
                            "global".to_string(),
                            crate::profile::McpServerConfig {
                                command: "global-mcp".to_string(),
                                args: Vec::new(),
                                env: std::collections::BTreeMap::new(),
                            },
                        )]),
                    },
                    skills: vec!["global-skill".to_string()],
                    plugins: vec!["global-plugin".to_string()],
                },
            )]),
            ..RepoverlayConfig::default()
        };
        let repo = load_repo_config(temp.path()).unwrap().unwrap();
        let merged = merge_repo_config(global, repo);
        let profile = merged.profiles.get("rust-dev").unwrap();

        assert_eq!(profile.description.as_deref(), Some("Global Rust"));
        assert_eq!(profile.overlays, vec!["repo-rust"]);
        assert!(profile.mcps.servers.contains_key("global"));
        assert!(profile.mcps.servers.contains_key("repo"));
        assert_eq!(profile.instructions[0].source, "global.md");
        assert_eq!(profile.skills, vec!["global-skill"]);
        assert_eq!(profile.plugins, vec!["global-plugin"]);
    }

    #[test]
    fn test_load_config_uses_global_when_no_repo() {
        // When repo_path is None, should return global config
        // This is a bit tricky to test fully without mocking the global config
        // but we can at least verify the function runs
        let result = load_config(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_global_config_path() {
        let path = global_config_path();
        assert!(path.is_ok());
        let path = path.unwrap();
        assert!(path.ends_with("config.ccl"));
        assert!(path.to_string_lossy().contains("repoverlay"));
    }

    #[test]
    fn test_default_repoverlay_config() {
        let config = RepoverlayConfig::default();
        assert!(config.sources.is_empty());
    }

    #[test]
    fn test_config_dir_with_xdg_value() {
        let dir = config_dir_with_env(Some("/custom/xdg")).unwrap();
        assert_eq!(dir, PathBuf::from("/custom/xdg/repoverlay"));
    }

    #[test]
    fn test_config_dir_without_xdg_uses_home() {
        let dir = config_dir_with_env(None).unwrap();
        // Should fall back to ~/.config/repoverlay
        assert!(dir.ends_with("repoverlay"));
        assert!(dir.to_string_lossy().contains(".config"));
    }

    // Additional edge case tests for config parsing
    #[test]
    fn test_load_repo_config_ignores_unknown_keys() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        // Config with extra/unknown keys
        let config_content = r"
sources =
  =
    name = default
    url = https://github.com/org/overlays

some_other_section =
  foo = bar
";
        fs::write(config_dir.join("config.ccl"), config_content).unwrap();

        // Should still parse successfully, ignoring unknown keys
        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.sources.len(), 1);
    }

    #[test]
    fn test_empty_config_file() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        // Empty config file
        fs::write(config_dir.join("config.ccl"), "").unwrap();

        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert!(config.sources.is_empty());
    }

    #[test]
    fn test_whitespace_only_config_file() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        // Whitespace-only config file
        fs::write(config_dir.join("config.ccl"), "   \n\n   \n").unwrap();

        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert!(config.sources.is_empty());
    }

    // ==================== Multi-source config tests ====================

    #[test]
    fn test_parse_sources_single() {
        // CCL list format: each list element is prefixed with `=`
        let ccl = r"
sources =
  =
    name = personal
    url = https://github.com/me/my-overlays
";
        let config: RepoverlayConfig = sickle::from_str(ccl).unwrap();
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].name, "personal");
        assert_eq!(
            config.sources[0].url().unwrap(),
            "https://github.com/me/my-overlays"
        );
    }

    #[test]
    fn test_parse_sources_multiple() {
        // CCL list format: each list element is prefixed with `=`
        let ccl = r"
sources =
  =
    name = personal
    url = https://github.com/me/my-overlays
  =
    name = my-team
    url = https://github.com/my-org/team-overlays
  =
    name = community
    url = https://github.com/repoverlay/overlays
";
        let config: RepoverlayConfig = sickle::from_str(ccl).unwrap();
        assert_eq!(config.sources.len(), 3);
        // Order should be preserved
        assert_eq!(config.sources[0].name, "personal");
        assert_eq!(config.sources[1].name, "my-team");
        assert_eq!(config.sources[2].name, "community");
    }

    #[test]
    fn test_parse_sources_empty() {
        let ccl = "";
        let config: RepoverlayConfig = sickle::from_str(ccl).unwrap();
        assert!(config.sources.is_empty());
    }

    #[test]
    fn test_parse_sources_missing_name() {
        // CCL list format: each list element is prefixed with `=`
        let ccl = r"
sources =
  =
    url = https://github.com/me/my-overlays
";
        // Sickle should error when required field is missing
        let result: Result<RepoverlayConfig, _> = sickle::from_str(ccl);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_sources_missing_url_and_path() {
        // A source with neither url nor path should parse but fail validation
        let ccl = r"
sources =
  =
    name = personal
";
        let config: RepoverlayConfig = sickle::from_str(ccl).unwrap();
        assert_eq!(config.sources.len(), 1);
        let source = &config.sources[0];
        assert!(source.url.is_none());
        assert!(source.path.is_none());
        assert!(source.validate().is_err());
    }

    #[test]
    fn test_sources_roundtrip() {
        let config = RepoverlayConfig {
            sources: vec![
                Source {
                    name: "personal".to_string(),
                    url: Some("https://github.com/me/my-overlays".to_string()),
                    path: None,
                },
                Source {
                    name: "team".to_string(),
                    url: Some("https://github.com/org/overlays".to_string()),
                    path: None,
                },
            ],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        let ccl = sickle::to_string(&config).unwrap();
        let parsed: RepoverlayConfig = sickle::from_str(&ccl).unwrap();

        assert_eq!(parsed.sources.len(), 2);
        assert_eq!(parsed.sources[0].name, "personal");
        assert_eq!(
            parsed.sources[0].url().unwrap(),
            "https://github.com/me/my-overlays"
        );
        assert_eq!(parsed.sources[1].name, "team");
        assert_eq!(
            parsed.sources[1].url().unwrap(),
            "https://github.com/org/overlays"
        );
    }

    #[test]
    fn test_source_equality() {
        let source1 = Source {
            name: "test".to_string(),
            url: Some("https://github.com/test/repo".to_string()),
            path: None,
        };
        let source2 = Source {
            name: "test".to_string(),
            url: Some("https://github.com/test/repo".to_string()),
            path: None,
        };
        let source3 = Source {
            name: "other".to_string(),
            url: Some("https://github.com/test/repo".to_string()),
            path: None,
        };

        assert_eq!(source1, source2);
        assert_ne!(source1, source3);
    }

    // ==================== URL validation tests ====================

    #[test]
    fn test_validate_source_url_full_https() {
        let result = validate_source_url("https://github.com/org/repo");
        assert_eq!(result.unwrap(), "https://github.com/org/repo");
    }

    #[test]
    fn test_validate_source_url_full_ssh() {
        let result = validate_source_url("git@github.com:org/repo.git");
        assert_eq!(result.unwrap(), "git@github.com:org/repo.git");
    }

    #[test]
    fn test_validate_source_url_git_protocol() {
        let result = validate_source_url("git://example.com/repo.git");
        assert_eq!(result.unwrap(), "git://example.com/repo.git");
    }

    #[test]
    fn test_validate_source_url_github_shorthand() {
        let result = validate_source_url("tylerbutler/repo-overlays");
        assert_eq!(
            result.unwrap(),
            "https://github.com/tylerbutler/repo-overlays"
        );
    }

    #[test]
    fn test_validate_source_url_bare_owner_expanded() {
        let result = validate_source_url("tylerbutler");
        assert_eq!(
            result.unwrap(),
            "https://github.com/tylerbutler/repo-overlays"
        );
    }

    #[test]
    fn test_validate_source_url_empty_rejected() {
        let result = validate_source_url("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_source_url_whitespace_rejected() {
        let result = validate_source_url("owner /repo");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_source_url_bare_owner_with_hyphens() {
        let result = validate_source_url("my-org");
        assert_eq!(result.unwrap(), "https://github.com/my-org/repo-overlays");
    }

    #[test]
    fn test_validate_source_url_whitespace_only_rejected() {
        let result = validate_source_url("  ");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_source_url_bare_owner_with_whitespace_rejected() {
        let result = validate_source_url("tyler butler");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_source_with_github_shorthand() {
        let ccl = r"
sources =
  =
    name = personal
    url = tylerbutler/repo-overlays
";
        let config: RepoverlayConfig = sickle::from_str(ccl).unwrap();
        assert_eq!(config.sources.len(), 1);
        assert_eq!(
            config.sources[0].url().unwrap(),
            "https://github.com/tylerbutler/repo-overlays"
        );
    }

    #[test]
    fn test_deserialize_source_with_bare_owner_expands() {
        let ccl = r"
sources =
  =
    name = personal
    url = tylerbutler
";
        let config: RepoverlayConfig = sickle::from_str(ccl).unwrap();
        assert_eq!(config.sources.len(), 1);
        assert_eq!(
            config.sources[0].url().unwrap(),
            "https://github.com/tylerbutler/repo-overlays"
        );
    }

    // ==================== get_default_overlay_repo_config tests ====================

    #[test]
    fn test_get_default_overlay_repo_config_from_sources() {
        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "default".to_string(),
                url: Some("https://github.com/test/overlays".to_string()),
                path: None,
            }],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        let result = config.get_default_overlay_repo_config();
        assert!(result.is_ok());
        let repo_config = result.unwrap();
        assert_eq!(repo_config.url, "https://github.com/test/overlays");
    }

    #[test]
    fn test_get_default_overlay_repo_config_no_sources() {
        let config = RepoverlayConfig {
            sources: vec![],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        let result = config.get_default_overlay_repo_config();
        assert!(result.is_err());
    }

    // ==================== get_overlay_repo_config_by_name tests ====================

    #[test]
    fn test_get_overlay_repo_config_by_name_none_falls_back_to_default() {
        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "default".to_string(),
                url: Some("https://github.com/org/repo".to_string()),
                path: None,
            }],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        let result = config.get_overlay_repo_config_by_name(None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().url, "https://github.com/org/repo");
    }

    #[test]
    fn test_get_overlay_repo_config_by_name_found() {
        let config = RepoverlayConfig {
            sources: vec![
                Source {
                    name: "primary".to_string(),
                    url: Some("https://github.com/org/primary".to_string()),
                    path: None,
                },
                Source {
                    name: "secondary".to_string(),
                    url: Some("https://github.com/org/secondary".to_string()),
                    path: None,
                },
            ],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        let result = config.get_overlay_repo_config_by_name(Some("secondary"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().url, "https://github.com/org/secondary");
    }

    #[test]
    fn test_get_overlay_repo_config_by_name_not_found() {
        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "primary".to_string(),
                url: Some("https://github.com/org/primary".to_string()),
                path: None,
            }],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        let result = config.get_overlay_repo_config_by_name(Some("nonexistent"));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("nonexistent"));
        assert!(err_msg.contains("primary"));
    }

    // ==================== SourceUrlInput tests ====================

    #[test]
    fn test_source_url_input_git_url() {
        let input: SourceUrlInput = "https://github.com/org/repo".parse().unwrap();
        assert_eq!(
            input,
            SourceUrlInput::GitUrl("https://github.com/org/repo".to_string())
        );
        assert_eq!(input.to_url(), "https://github.com/org/repo");
    }

    #[test]
    fn test_source_url_input_ssh_url() {
        let input: SourceUrlInput = "git@github.com:org/repo.git".parse().unwrap();
        assert_eq!(
            input,
            SourceUrlInput::GitUrl("git@github.com:org/repo.git".to_string())
        );
        assert_eq!(input.to_url(), "git@github.com:org/repo.git");
    }

    #[test]
    fn test_source_url_input_github_shorthand() {
        let input: SourceUrlInput = "owner/repo".parse().unwrap();
        assert_eq!(
            input,
            SourceUrlInput::GitHubShorthand {
                owner: "owner".to_string(),
                repo: "repo".to_string()
            }
        );
        assert_eq!(input.to_url(), "https://github.com/owner/repo");
    }

    #[test]
    fn test_source_url_input_bare_owner() {
        let input: SourceUrlInput = "tylerbutler".parse().unwrap();
        assert_eq!(input, SourceUrlInput::BareOwner("tylerbutler".to_string()));
        assert_eq!(
            input.to_url(),
            "https://github.com/tylerbutler/repo-overlays"
        );
    }

    #[test]
    fn test_source_url_input_empty_rejected() {
        let result: std::result::Result<SourceUrlInput, _> = "".parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_source_url_input_whitespace_rejected() {
        let result: std::result::Result<SourceUrlInput, _> = "  ".parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_source_url_input_display() {
        let git: SourceUrlInput = "https://github.com/a/b".parse().unwrap();
        assert_eq!(format!("{git}"), "https://github.com/a/b");

        let shorthand: SourceUrlInput = "owner/repo".parse().unwrap();
        assert_eq!(format!("{shorthand}"), "owner/repo");

        let bare: SourceUrlInput = "user".parse().unwrap();
        assert_eq!(format!("{bare}"), "user");

        let local: SourceUrlInput = "./my-overlays".parse().unwrap();
        assert_eq!(format!("{local}"), "./my-overlays");
    }

    #[test]
    fn test_source_url_input_local_path_relative() {
        let input: SourceUrlInput = "./my-overlays".parse().unwrap();
        assert_eq!(
            input,
            SourceUrlInput::LocalPath(PathBuf::from("./my-overlays"))
        );
        assert!(input.is_local());
        assert_eq!(input.local_path(), Path::new("./my-overlays"));
    }

    #[test]
    fn test_source_url_input_local_path_absolute() {
        let input: SourceUrlInput = "/absolute/path".parse().unwrap();
        assert_eq!(
            input,
            SourceUrlInput::LocalPath(PathBuf::from("/absolute/path"))
        );
        assert!(input.is_local());
    }

    #[test]
    fn test_source_url_input_local_path_tilde() {
        let input: SourceUrlInput = "~/overlays".parse().unwrap();
        assert_eq!(
            input,
            SourceUrlInput::LocalPath(PathBuf::from("~/overlays"))
        );
        assert!(input.is_local());
    }

    #[test]
    fn test_source_url_input_local_path_parent() {
        let input: SourceUrlInput = "../parent/overlays".parse().unwrap();
        assert_eq!(
            input,
            SourceUrlInput::LocalPath(PathBuf::from("../parent/overlays"))
        );
        assert!(input.is_local());
    }

    #[test]
    fn test_source_url_input_owner_repo_not_local() {
        let input: SourceUrlInput = "owner/repo".parse().unwrap();
        assert!(!input.is_local());
        assert!(matches!(input, SourceUrlInput::GitHubShorthand { .. }));
    }

    #[test]
    fn test_source_url_input_bare_owner_not_local() {
        let input: SourceUrlInput = "username".parse().unwrap();
        assert!(!input.is_local());
        assert!(matches!(input, SourceUrlInput::BareOwner(_)));
    }

    // ==================== default_overlay_repo_name tests ====================

    #[test]
    fn test_default_overlay_repo_name_with_env_uses_value() {
        let result = default_overlay_repo_name_with_env(Some("my-overlays".to_string()));
        assert_eq!(result, "my-overlays");
    }

    #[test]
    fn test_default_overlay_repo_name_with_env_falls_back_to_constant() {
        let result = default_overlay_repo_name_with_env(None);
        assert_eq!(result, "repo-overlays");
    }

    #[test]
    fn test_bare_owner_to_url_uses_custom_repo_name() {
        let input = SourceUrlInput::BareOwner("myuser".to_string());
        let url = input.to_url_with_repo_name("custom-overlays");
        assert_eq!(url, "https://github.com/myuser/custom-overlays");
    }

    // ==================== Per-repo config merge tests ====================

    #[test]
    fn test_load_repo_config_no_file_returns_none() {
        let temp = TempDir::new().unwrap();
        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_none());
    }

    #[test]
    fn test_load_repo_config_with_url_source() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        let config_content = r"
sources =
  =
    name = local-overlays
    url = https://github.com/org/local-overlays
";
        fs::write(config_dir.join("config.ccl"), config_content).unwrap();

        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].name, "local-overlays");
        assert_eq!(
            config.sources[0].url().unwrap(),
            "https://github.com/org/local-overlays"
        );
    }

    #[test]
    fn test_load_config_merges_repo_before_global() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        let config_content = r"
sources =
  =
    name = repo-local
    url = https://github.com/org/repo-overlays
";
        fs::write(config_dir.join("config.ccl"), config_content).unwrap();

        let config = load_config(Some(temp.path())).unwrap();
        // Repo sources should appear first
        assert!(!config.sources.is_empty());
        assert_eq!(config.sources[0].name, "repo-local");
    }

    #[test]
    fn test_load_config_none_returns_global_only() {
        let config = load_config(None).unwrap();
        // Should work the same as load_global_config
        let global = load_global_config().unwrap();
        assert_eq!(config.sources.len(), global.sources.len());
    }

    #[test]
    fn test_save_repo_config_roundtrip() {
        let temp = TempDir::new().unwrap();

        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "local-source".to_string(),
                url: Some("https://github.com/org/overlays".to_string()),
                path: None,
            }],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        save_repo_config(temp.path(), &config).unwrap();

        let config_path = repo_config_path(temp.path());
        assert!(config_path.exists());

        let loaded = load_repo_config(temp.path()).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.sources.len(), 1);
        assert_eq!(loaded.sources[0].name, "local-source");
        assert_eq!(
            loaded.sources[0].url().unwrap(),
            "https://github.com/org/overlays"
        );
    }

    #[test]
    fn test_save_repo_config_creates_directory() {
        let temp = TempDir::new().unwrap();
        let config = RepoverlayConfig::default();

        assert!(!temp.path().join(".repoverlay").exists());

        save_repo_config(temp.path(), &config).unwrap();

        assert!(temp.path().join(".repoverlay").exists());
        assert!(repo_config_path(temp.path()).exists());
    }

    // ==================== Source validation tests ====================

    #[test]
    fn test_source_validate_url_only_ok() {
        let source = Source {
            name: "test".to_string(),
            url: Some("https://github.com/org/repo".to_string()),
            path: None,
        };
        assert!(source.validate().is_ok());
        assert!(source.is_git());
        assert!(!source.is_local());
    }

    #[test]
    fn test_source_validate_path_only_ok() {
        let source = Source {
            name: "test".to_string(),
            url: None,
            path: Some(PathBuf::from("./my-overlays")),
        };
        assert!(source.validate().is_ok());
        assert!(source.is_local());
        assert!(!source.is_git());
    }

    #[test]
    fn test_source_validate_both_set_err() {
        let source = Source {
            name: "test".to_string(),
            url: Some("https://github.com/org/repo".to_string()),
            path: Some(PathBuf::from("./my-overlays")),
        };
        let err = source.validate().unwrap_err();
        assert!(err.to_string().contains("both url and path"));
    }

    #[test]
    fn test_source_validate_neither_set_err() {
        let source = Source {
            name: "test".to_string(),
            url: None,
            path: None,
        };
        let err = source.validate().unwrap_err();
        assert!(err.to_string().contains("neither url nor path"));
    }

    // ==================== CCL serialization roundtrip for path sources ====================

    #[test]
    fn test_source_with_path_ccl_roundtrip() {
        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "local-overlays".to_string(),
                url: None,
                path: Some(PathBuf::from("my-overlays")),
            }],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        let ccl = generate_sources_config_ccl(&config);
        assert!(ccl.contains("name = local-overlays"));
        assert!(ccl.contains("path = my-overlays"));
        assert!(!ccl.contains("url ="));

        let parsed: RepoverlayConfig = sickle::from_str(&ccl).unwrap();
        assert_eq!(parsed.sources.len(), 1);
        assert_eq!(parsed.sources[0].name, "local-overlays");
        assert!(parsed.sources[0].url.is_none());
        assert_eq!(
            parsed.sources[0].path.as_deref(),
            Some(Path::new("my-overlays"))
        );
    }

    #[test]
    fn test_mixed_sources_ccl_roundtrip() {
        let config = RepoverlayConfig {
            sources: vec![
                Source {
                    name: "local".to_string(),
                    url: None,
                    path: Some(PathBuf::from("overlays")),
                },
                Source {
                    name: "remote".to_string(),
                    url: Some("https://github.com/org/overlays".to_string()),
                    path: None,
                },
            ],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        let ccl = generate_sources_config_ccl(&config);
        let parsed: RepoverlayConfig = sickle::from_str(&ccl).unwrap();

        assert_eq!(parsed.sources.len(), 2);
        assert_eq!(parsed.sources[0].name, "local");
        assert!(parsed.sources[0].is_local());
        assert_eq!(
            parsed.sources[0].path.as_deref(),
            Some(Path::new("overlays"))
        );
        assert_eq!(parsed.sources[1].name, "remote");
        assert!(parsed.sources[1].is_git());
        assert_eq!(
            parsed.sources[1].url().unwrap(),
            "https://github.com/org/overlays"
        );
    }

    // ==================== Config merge ordering tests ====================

    #[test]
    fn test_config_merge_repo_sources_come_first() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        let repo_ccl = r"
sources =
  =
    name = repo-source
    path = my-overlays
";
        fs::write(config_dir.join("config.ccl"), repo_ccl).unwrap();

        let config = load_config(Some(temp.path())).unwrap();
        // Repo sources should be first in the merged list
        assert!(!config.sources.is_empty());
        assert_eq!(config.sources[0].name, "repo-source");

        // If there are global sources, they should come after
        if config.sources.len() > 1 {
            assert_ne!(config.sources[1].name, "repo-source");
        }
    }

    // ==================== SourceUrlInput local path parsing tests ====================

    #[test]
    fn test_source_url_input_dot_slash_is_local() {
        let input: SourceUrlInput = "./foo".parse().unwrap();
        assert!(input.is_local());
        assert_eq!(input.local_path(), Path::new("./foo"));
    }

    #[test]
    fn test_source_url_input_absolute_path_is_local() {
        let input: SourceUrlInput = "/abs/path".parse().unwrap();
        assert!(input.is_local());
        assert_eq!(input.local_path(), Path::new("/abs/path"));
    }

    #[test]
    fn test_source_url_input_tilde_is_local() {
        let input: SourceUrlInput = "~/foo".parse().unwrap();
        assert!(input.is_local());
        assert_eq!(input.local_path(), Path::new("~/foo"));
    }

    #[test]
    fn test_source_url_input_dot_dot_is_local() {
        let input: SourceUrlInput = "../bar".parse().unwrap();
        assert!(input.is_local());
        assert_eq!(input.local_path(), Path::new("../bar"));
    }

    #[test]
    fn test_source_url_input_owner_repo_still_github_shorthand() {
        let input: SourceUrlInput = "owner/repo".parse().unwrap();
        assert!(!input.is_local());
        assert!(matches!(input, SourceUrlInput::GitHubShorthand { .. }));
    }

    // ==================== SourceUrlInput file:// URL parsing tests ====================

    #[test]
    fn test_source_url_input_file_url_is_file_url() {
        let input: SourceUrlInput = "file:///tmp/my-overlays".parse().unwrap();
        assert!(input.is_file_url());
        assert!(!input.is_local());
        assert_eq!(input.local_path(), Path::new("/tmp/my-overlays"));
    }

    #[test]
    fn test_source_url_input_file_url_two_slashes() {
        let input: SourceUrlInput = "file:///home/user/overlays".parse().unwrap();
        assert!(input.is_file_url());
        assert_eq!(input.local_path(), Path::new("/home/user/overlays"));
    }

    #[test]
    fn test_source_url_input_file_url_not_git_url() {
        let input: SourceUrlInput = "file:///some/path".parse().unwrap();
        assert!(!matches!(input, SourceUrlInput::GitUrl(_)));
    }

    #[test]
    fn test_source_url_input_file_url_empty_path_rejected() {
        let result: std::result::Result<SourceUrlInput, _> = "file://".parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_source_url_input_file_url_display() {
        let input: SourceUrlInput = "file:///tmp/overlays".parse().unwrap();
        assert_eq!(format!("{input}"), "file:///tmp/overlays");
    }

    #[test]
    fn test_source_url_input_file_url_preserves_original() {
        // file:// URLs should preserve the original string for roundtripping
        let input: SourceUrlInput = "file:///tmp/overlays".parse().unwrap();
        assert!(input.is_file_url());
    }

    // ==================== Save/load roundtrip for path-based repo config ====================

    #[test]
    fn test_save_repo_config_with_path_source_roundtrip() {
        let temp = TempDir::new().unwrap();

        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "local-source".to_string(),
                url: None,
                path: Some(PathBuf::from("my-overlays")),
            }],
            library_path: None,
            profiles: std::collections::BTreeMap::new(),
        };

        save_repo_config(temp.path(), &config).unwrap();

        let loaded = load_repo_config(temp.path()).unwrap().unwrap();
        assert_eq!(loaded.sources.len(), 1);
        assert_eq!(loaded.sources[0].name, "local-source");
        assert!(loaded.sources[0].is_local());
        assert_eq!(
            loaded.sources[0].path.as_deref(),
            Some(Path::new("my-overlays"))
        );
        assert!(loaded.sources[0].url.is_none());
    }

    #[test]
    fn test_parse_repo_config_with_path_source() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        let config_content = r"
sources =
  =
    name = local-overlays
    path = my-overlays
";
        fs::write(config_dir.join("config.ccl"), config_content).unwrap();

        let config = load_repo_config(temp.path()).unwrap().unwrap();
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].name, "local-overlays");
        assert!(config.sources[0].is_local());
        assert_eq!(
            config.sources[0].path.as_deref(),
            Some(Path::new("my-overlays"))
        );
    }

    #[test]
    fn snapshot_source_url_input_display() {
        let inputs: Vec<SourceUrlInput> = [
            "https://github.com/owner/repo",
            "git@github.com:owner/repo.git",
            "owner/repo",
            "myuser",
        ]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();

        let output: Vec<String> = inputs.iter().map(|i| format!("{i}")).collect();
        insta::assert_snapshot!(output.join("\n"));
    }
}
