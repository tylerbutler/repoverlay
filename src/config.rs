//! Configuration management for repoverlay.
//!
//! Handles global and per-repo configuration using CCL format.
//! Global config: `~/.config/repoverlay/config.ccl`
//! Per-repo config: `.repoverlay/config.ccl`

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Global repoverlay configuration.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct RepoverlayConfig {
    /// Configured overlay sources (checked in order for resolution).
    #[serde(default)]
    pub sources: Vec<Source>,
    /// Legacy overlay repository configuration (for backwards compatibility).
    /// New configs should use `sources` instead.
    /// Deprecated: will be removed in 1.0 (#79).
    #[serde(default)]
    pub overlay_repo: Option<OverlayRepoConfig>,
}

impl RepoverlayConfig {
    /// Get an `OverlayRepoConfig` from the first configured source, falling
    /// back to the legacy `overlay_repo` field.
    ///
    /// Commands that need a single overlay repo (create, inspect, sync, etc.)
    /// should use this instead of accessing `overlay_repo` directly.
    pub fn get_default_overlay_repo_config(&self) -> Result<OverlayRepoConfig> {
        // Prefer new multi-source config
        if let Some(source) = self.sources.first() {
            let cache_dir = directories::ProjectDirs::from("", "", "repoverlay")
                .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
            let local_path = cache_dir.cache_dir().join("sources").join(&source.name);
            return Ok(OverlayRepoConfig {
                url: source.url.clone(),
                local_path: Some(local_path),
            });
        }

        // Fall back to legacy overlay_repo
        self.overlay_repo.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "Overlay repository not configured.\n\n\
                 Run 'repoverlay source add <url>' to set up an overlay source."
            )
        })
    }

    /// Get an `OverlayRepoConfig` for a specific named source.
    ///
    /// Looks up the source by name in the configured sources list.
    /// Falls back to `get_default_overlay_repo_config` if `source_name` is `None`.
    pub fn get_overlay_repo_config_by_name(
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
            url: source.url.clone(),
            local_path: Some(local_path),
        })
    }
}

/// An overlay source repository.
///
/// Sources are checked in order when resolving overlay references.
/// Earlier sources have higher priority.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct Source {
    /// Name for this source (used in CLI output and `--source` flag).
    pub name: String,
    /// Git URL of the overlay repository.
    /// Accepts full URLs or GitHub shorthand (`owner/repo`), which is expanded
    /// to `https://github.com/owner/repo` during deserialization.
    #[serde(deserialize_with = "deserialize_source_url")]
    pub url: String,
}

/// Default overlay repository name for the one-part shorthand syntax.
/// When user types `username`, it expands to `username/repo-overlays`.
pub const DEFAULT_OVERLAY_REPO_NAME: &str = "repo-overlays";

/// Returns the overlay repository name for the one-part shorthand syntax.
///
/// Checks `REPOVERLAY_DEFAULT_REPO_NAME` env var first, falling back to
/// [`DEFAULT_OVERLAY_REPO_NAME`].
#[must_use]
pub fn default_overlay_repo_name() -> String {
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
pub enum SourceUrlInput {
    /// A full git-cloneable URL.
    GitUrl(String),
    /// GitHub shorthand (`owner/repo`), expanded to `https://github.com/owner/repo`.
    GitHubShorthand { owner: String, repo: String },
    /// Bare owner name, expanded to `https://github.com/owner/{default_repo}`.
    BareOwner(String),
}

impl SourceUrlInput {
    /// Returns the expanded git URL for this input.
    #[must_use]
    pub fn to_url(&self) -> String {
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
pub fn expand_github_shorthand(s: &str) -> String {
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
pub fn validate_source_url(url: &str) -> std::result::Result<String, String> {
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

/// Custom deserializer that validates and expands source URLs.
fn deserialize_source_url<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    validate_source_url(&raw).map_err(serde::de::Error::custom)
}

/// Configuration for a shared overlay repository.
/// Deprecated: will be removed in 1.0 (#79). Use `Source` instead.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OverlayRepoConfig {
    /// Git URL of the overlay repository.
    pub url: String,
    /// Optional override for the local clone path.
    /// Default: `~/.local/share/repoverlay/overlay-repo/`
    #[serde(default)]
    pub local_path: Option<PathBuf>,
}

/// Check if a config uses the old `overlay_repo` format and needs migration.
///
/// Returns `true` if the config has `overlay_repo` set but no `sources`.
/// This indicates the config should be migrated to the new multi-source format.
/// Will be removed in 1.0 along with `overlay_repo` support (#79).
#[must_use]
pub const fn needs_migration(config: &RepoverlayConfig) -> bool {
    config.overlay_repo.is_some() && config.sources.is_empty()
}

/// Migrate old config format to new multi-source format.
///
/// If the config uses the legacy `overlay_repo` key, converts it to a source
/// named "default". Returns a message describing the migration if one occurred.
/// Will be removed in 1.0 along with `overlay_repo` support (#79).
#[must_use]
pub fn migrate_config(config: &mut RepoverlayConfig) -> Option<String> {
    if needs_migration(config) {
        let old = config.overlay_repo.take().unwrap();
        config.sources.push(Source {
            name: "default".to_string(),
            url: old.url,
        });
        Some("Migrated overlay_repo to sources format".to_string())
    } else {
        None
    }
}

/// Get the global config directory path.
///
/// Returns `~/.config/repoverlay/` on all Unix-like systems.
/// Respects `XDG_CONFIG_HOME` if set.
pub fn config_dir() -> Result<PathBuf> {
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
pub fn global_config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.ccl"))
}

/// Get the path to the per-repo config file.
pub fn repo_config_path(repo_path: &Path) -> PathBuf {
    repo_path.join(".repoverlay").join("config.ccl")
}

/// Load the global configuration.
///
/// Automatically migrates legacy `overlay_repo` configs to the `sources` format.
pub fn load_global_config() -> Result<RepoverlayConfig> {
    let config_path = global_config_path()?;

    if !config_path.exists() {
        return Ok(RepoverlayConfig::default());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

    let mut config: RepoverlayConfig = sickle::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;

    // Auto-migrate legacy overlay_repo to sources format
    if let Some(message) = migrate_config(&mut config) {
        save_config(&config)?;
        eprintln!("{message}");
        eprintln!("Config updated: {}", config_path.display());
    }

    Ok(config)
}

/// Load the per-repo configuration.
pub fn load_repo_config(repo_path: &Path) -> Result<Option<RepoverlayConfig>> {
    let config_path = repo_config_path(repo_path);

    if !config_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

    let config: RepoverlayConfig = sickle::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;

    Ok(Some(config))
}

/// Load configuration, merging global with per-repo overrides.
///
/// Per-repo settings override global settings.
pub fn load_config(repo_path: Option<&Path>) -> Result<RepoverlayConfig> {
    let mut config = load_global_config()?;

    if let Some(repo) = repo_path
        && let Some(repo_config) = load_repo_config(repo)?
        && repo_config.overlay_repo.is_some()
    {
        config.overlay_repo = repo_config.overlay_repo;
    }

    Ok(config)
}

/// Generate a config file for multi-source configuration.
pub fn generate_sources_config_ccl(config: &RepoverlayConfig) -> String {
    let mut output = String::new();
    output.push_str("/= repoverlay global configuration\n");
    output.push_str("/= This file configures repoverlay's overlay sources.\n\n");

    if !config.sources.is_empty() {
        output.push_str(
            "/= Sources are checked in priority order (first listed = highest priority).\n",
        );
        output.push_str(
            "/= To change priority, edit this file directly or remove and re-add sources.\n",
        );
        output.push_str("sources =\n");

        for source in &config.sources {
            output.push_str("  =\n");
            let _ = writeln!(output, "    name = {}", source.name);
            let _ = writeln!(output, "    url = {}", source.url);
        }
    }

    // Include legacy overlay_repo if present. Will be removed in 1.0 (#79).
    if let Some(ref overlay_repo) = config.overlay_repo {
        if !config.sources.is_empty() {
            output.push_str(
                "\n/= Legacy overlay_repo configuration (deprecated, use sources instead)\n",
            );
        }
        output.push_str("overlay_repo =\n");
        let _ = writeln!(output, "  url = {}", overlay_repo.url);
        if let Some(ref local_path) = overlay_repo.local_path {
            let _ = writeln!(output, "  local_path = {}", local_path.display());
        }
    }

    output
}

/// Save the global configuration.
pub fn save_config(config: &RepoverlayConfig) -> Result<()> {
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
        // This test relies on the config file not existing in the test environment
        // Skip assertion if a user's config already exists, as it may have overlay_repo set
        let config = load_global_config();
        if let Ok(cfg) = config {
            // Only assert if no global config file exists (i.e., we got defaults)
            if !global_config_path().is_ok_and(|p| p.exists()) {
                assert!(cfg.overlay_repo.is_none());
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
    fn test_roundtrip_config() {
        let config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/test/overlays".to_string(),
                local_path: None,
            }),
        };

        // Serialize to CCL
        let ccl = sickle::to_string(&config).unwrap();

        // Deserialize back
        let parsed: RepoverlayConfig = sickle::from_str(&ccl).unwrap();

        assert!(parsed.overlay_repo.is_some());
        let overlay_repo = parsed.overlay_repo.unwrap();
        assert_eq!(overlay_repo.url, "https://github.com/test/overlays");
        assert!(overlay_repo.local_path.is_none());
    }

    #[test]
    fn test_load_repo_config_valid() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        let config_content = r"
overlay_repo =
  url = https://github.com/org/overlays
";
        fs::write(config_dir.join("config.ccl"), config_content).unwrap();

        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert!(config.overlay_repo.is_some());
        assert_eq!(
            config.overlay_repo.unwrap().url,
            "https://github.com/org/overlays"
        );
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
    fn test_load_config_repo_overrides_global() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).unwrap();

        let repo_config_content = r"
overlay_repo =
  url = https://github.com/repo/specific
";
        fs::write(config_dir.join("config.ccl"), repo_config_content).unwrap();

        // The repo config should be used when present
        let config = load_config(Some(temp.path())).unwrap();
        // If repo config has overlay_repo, it should override global
        if let Some(overlay_repo) = config.overlay_repo {
            assert_eq!(overlay_repo.url, "https://github.com/repo/specific");
        }
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
    fn test_overlay_repo_config_with_local_path_roundtrip() {
        let config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/test/overlays".to_string(),
                local_path: Some(PathBuf::from("/custom/path")),
            }),
        };

        let ccl = sickle::to_string(&config).unwrap();
        let parsed: RepoverlayConfig = sickle::from_str(&ccl).unwrap();

        assert!(parsed.overlay_repo.is_some());
        let overlay_repo = parsed.overlay_repo.unwrap();
        assert_eq!(overlay_repo.local_path, Some(PathBuf::from("/custom/path")));
    }

    #[test]
    fn test_default_repoverlay_config() {
        let config = RepoverlayConfig::default();
        assert!(config.sources.is_empty());
        assert!(config.overlay_repo.is_none());
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
overlay_repo =
  url = https://github.com/org/overlays
  unknown_field = some_value

some_other_section =
  foo = bar
";
        fs::write(config_dir.join("config.ccl"), config_content).unwrap();

        // Should still parse successfully, ignoring unknown keys
        let config = load_repo_config(temp.path()).unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert!(config.overlay_repo.is_some());
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
        // overlay_repo should be None since not specified
        assert!(config.overlay_repo.is_none());
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
        assert!(config.overlay_repo.is_none());
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
        assert_eq!(config.sources[0].url, "https://github.com/me/my-overlays");
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
    fn test_parse_sources_missing_url() {
        // CCL list format: each list element is prefixed with `=`
        let ccl = r"
sources =
  =
    name = personal
";
        // Sickle should error when required field is missing
        let result: Result<RepoverlayConfig, _> = sickle::from_str(ccl);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_old_format() {
        // Config with old overlay_repo format
        let old_config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/org/overlays".to_string(),
                local_path: None,
            }),
        };
        assert!(needs_migration(&old_config));

        // Config with new sources format - no migration needed
        let new_config = RepoverlayConfig {
            sources: vec![Source {
                name: "default".to_string(),
                url: "https://github.com/org/overlays".to_string(),
            }],
            overlay_repo: None,
        };
        assert!(!needs_migration(&new_config));

        // Empty config - no migration needed
        let empty_config = RepoverlayConfig::default();
        assert!(!needs_migration(&empty_config));
    }

    #[test]
    fn test_sources_roundtrip() {
        let config = RepoverlayConfig {
            sources: vec![
                Source {
                    name: "personal".to_string(),
                    url: "https://github.com/me/my-overlays".to_string(),
                },
                Source {
                    name: "team".to_string(),
                    url: "https://github.com/org/overlays".to_string(),
                },
            ],
            overlay_repo: None,
        };

        let ccl = sickle::to_string(&config).unwrap();
        let parsed: RepoverlayConfig = sickle::from_str(&ccl).unwrap();

        assert_eq!(parsed.sources.len(), 2);
        assert_eq!(parsed.sources[0].name, "personal");
        assert_eq!(parsed.sources[0].url, "https://github.com/me/my-overlays");
        assert_eq!(parsed.sources[1].name, "team");
        assert_eq!(parsed.sources[1].url, "https://github.com/org/overlays");
    }

    // ==================== Migration tests ====================

    #[test]
    fn test_migrate_old_format() {
        let mut config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/org/overlays".to_string(),
                local_path: None,
            }),
        };

        let message = migrate_config(&mut config);

        assert!(message.is_some());
        assert!(message.unwrap().contains("Migrated"));
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].name, "default");
        assert_eq!(config.sources[0].url, "https://github.com/org/overlays");
        assert!(config.overlay_repo.is_none());
    }

    #[test]
    fn test_migrate_preserves_url() {
        let original_url = "https://github.com/specific/repo".to_string();
        let mut config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: original_url.clone(),
                local_path: None,
            }),
        };

        let _ = migrate_config(&mut config);

        assert_eq!(config.sources[0].url, original_url);
    }

    #[test]
    fn test_migrate_idempotent() {
        let mut config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/org/overlays".to_string(),
                local_path: None,
            }),
        };

        // First migration
        let _ = migrate_config(&mut config);
        assert_eq!(config.sources.len(), 1);

        // Second migration should do nothing
        let message = migrate_config(&mut config);
        assert!(message.is_none());
        assert_eq!(config.sources.len(), 1);
    }

    #[test]
    fn test_new_format_no_migration() {
        let mut config = RepoverlayConfig {
            sources: vec![Source {
                name: "existing".to_string(),
                url: "https://github.com/existing/repo".to_string(),
            }],
            overlay_repo: None,
        };

        let message = migrate_config(&mut config);

        assert!(message.is_none());
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].name, "existing");
    }

    // ==================== Auto-migration integration tests ====================

    #[test]
    fn test_auto_migration_rewrites_config_on_disk() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.ccl");

        // Write a legacy config
        fs::write(
            &config_path,
            "overlay_repo =\n  url = https://github.com/org/overlays\n",
        )
        .unwrap();

        // Simulate what load_global_config does: parse, migrate, save
        let content = fs::read_to_string(&config_path).unwrap();
        let mut config: RepoverlayConfig = sickle::from_str(&content).unwrap();
        assert!(config.overlay_repo.is_some());
        assert!(config.sources.is_empty());

        let message = migrate_config(&mut config);
        assert!(message.is_some());

        // Save migrated config
        let output = generate_sources_config_ccl(&config);
        fs::write(&config_path, &output).unwrap();

        // Verify the result
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].name, "default");
        assert_eq!(config.sources[0].url, "https://github.com/org/overlays");
        assert!(config.overlay_repo.is_none());

        // Verify the file on disk was rewritten with sources format
        let rewritten = fs::read_to_string(&config_path).unwrap();
        assert!(rewritten.contains("sources"));
        assert!(rewritten.contains("default"));
        assert!(!rewritten.contains("overlay_repo"));
    }

    #[test]
    fn test_auto_migration_idempotent_on_reload() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.ccl");

        // Write a legacy config
        fs::write(
            &config_path,
            "overlay_repo =\n  url = https://github.com/org/overlays\n",
        )
        .unwrap();

        // First load + migrate
        let content = fs::read_to_string(&config_path).unwrap();
        let mut config: RepoverlayConfig = sickle::from_str(&content).unwrap();
        let message = migrate_config(&mut config);
        assert!(message.is_some());
        let output = generate_sources_config_ccl(&config);
        fs::write(&config_path, &output).unwrap();

        let content_after_first = fs::read_to_string(&config_path).unwrap();

        // Second load should not trigger migration
        let mut config2: RepoverlayConfig = sickle::from_str(&content_after_first).unwrap();
        let message2 = migrate_config(&mut config2);
        assert!(message2.is_none());
        assert_eq!(config2.sources.len(), 1);
        assert!(config2.overlay_repo.is_none());
    }

    #[test]
    fn test_no_migration_when_already_sources_format() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.ccl");

        let content =
            "sources =\n  =\n    name = personal\n    url = https://github.com/me/overlays\n";
        fs::write(&config_path, content).unwrap();

        let mut config: RepoverlayConfig = sickle::from_str(content).unwrap();
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].name, "personal");
        assert!(config.overlay_repo.is_none());

        let message = migrate_config(&mut config);
        assert!(message.is_none());
    }

    #[test]
    fn test_needs_migration_with_both_sources_and_overlay_repo() {
        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "existing".to_string(),
                url: "https://github.com/existing/repo".to_string(),
            }],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/legacy/repo".to_string(),
                local_path: None,
            }),
        };
        // Should NOT trigger migration when sources already exist
        assert!(!needs_migration(&config));
    }

    #[test]
    fn test_migrate_config_skips_when_both_present() {
        let mut config = RepoverlayConfig {
            sources: vec![Source {
                name: "existing".to_string(),
                url: "https://github.com/existing/repo".to_string(),
            }],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/legacy/repo".to_string(),
                local_path: None,
            }),
        };

        let message = migrate_config(&mut config);
        assert!(message.is_none());
        // Sources unchanged, overlay_repo left as-is
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].name, "existing");
        assert!(config.overlay_repo.is_some());
    }

    #[test]
    fn test_generate_config_after_migration() {
        let mut config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/org/overlays".to_string(),
                local_path: None,
            }),
        };

        let _ = migrate_config(&mut config);
        let output = generate_sources_config_ccl(&config);

        // Should have sources section
        assert!(output.contains("sources"));
        assert!(output.contains("default"));
        assert!(output.contains("https://github.com/org/overlays"));
        // Should NOT have legacy overlay_repo section
        assert!(!output.contains("overlay_repo"));
    }

    #[test]
    fn test_generate_config_with_legacy_and_sources() {
        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "personal".to_string(),
                url: "https://github.com/me/overlays".to_string(),
            }],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/legacy/repo".to_string(),
                local_path: None,
            }),
        };

        let output = generate_sources_config_ccl(&config);

        // Should include both sections
        assert!(output.contains("sources"));
        assert!(output.contains("personal"));
        assert!(output.contains("overlay_repo"));
        assert!(output.contains("deprecated"));
    }

    #[test]
    fn test_migrated_config_roundtrips_through_save_load() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.ccl");

        // Migrate in memory
        let mut config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/org/overlays".to_string(),
                local_path: None,
            }),
        };
        let _ = migrate_config(&mut config);

        // Write using generate function
        let content = generate_sources_config_ccl(&config);
        fs::write(&config_path, &content).unwrap();

        // Re-read and parse
        let reloaded_content = fs::read_to_string(&config_path).unwrap();
        let reloaded: RepoverlayConfig = sickle::from_str(&reloaded_content).unwrap();

        assert_eq!(reloaded.sources.len(), 1);
        assert_eq!(reloaded.sources[0].name, "default");
        assert_eq!(reloaded.sources[0].url, "https://github.com/org/overlays");
        assert!(reloaded.overlay_repo.is_none());
        assert!(!needs_migration(&reloaded));
    }

    #[test]
    fn test_generate_config_legacy_overlay_repo_with_local_path() {
        let config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/org/overlays".to_string(),
                local_path: Some(PathBuf::from("/custom/local/path")),
            }),
        };

        let output = generate_sources_config_ccl(&config);
        assert!(output.contains("overlay_repo"));
        assert!(output.contains("url = https://github.com/org/overlays"));
        assert!(output.contains("local_path = /custom/local/path"));
    }

    #[test]
    fn test_source_equality() {
        let source1 = Source {
            name: "test".to_string(),
            url: "https://github.com/test/repo".to_string(),
        };
        let source2 = Source {
            name: "test".to_string(),
            url: "https://github.com/test/repo".to_string(),
        };
        let source3 = Source {
            name: "other".to_string(),
            url: "https://github.com/test/repo".to_string(),
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
            config.sources[0].url,
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
            config.sources[0].url,
            "https://github.com/tylerbutler/repo-overlays"
        );
    }

    // ==================== get_default_overlay_repo_config tests ====================

    #[test]
    fn test_get_default_overlay_repo_config_from_sources() {
        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "default".to_string(),
                url: "https://github.com/test/overlays".to_string(),
            }],
            overlay_repo: None,
        };

        let result = config.get_default_overlay_repo_config();
        assert!(result.is_ok());
        let repo_config = result.unwrap();
        assert_eq!(repo_config.url, "https://github.com/test/overlays");
    }

    #[test]
    fn test_get_default_overlay_repo_config_from_legacy() {
        let config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/legacy/repo".to_string(),
                local_path: None,
            }),
        };

        let result = config.get_default_overlay_repo_config();
        assert!(result.is_ok());
        let repo_config = result.unwrap();
        assert_eq!(repo_config.url, "https://github.com/legacy/repo");
    }

    #[test]
    fn test_get_default_overlay_repo_config_prefers_sources() {
        let config = RepoverlayConfig {
            sources: vec![Source {
                name: "primary".to_string(),
                url: "https://github.com/new/repo".to_string(),
            }],
            overlay_repo: Some(OverlayRepoConfig {
                url: "https://github.com/old/repo".to_string(),
                local_path: None,
            }),
        };

        let result = config.get_default_overlay_repo_config();
        assert!(result.is_ok());
        let repo_config = result.unwrap();
        assert_eq!(repo_config.url, "https://github.com/new/repo");
    }

    #[test]
    fn test_get_default_overlay_repo_config_no_sources_no_legacy() {
        let config = RepoverlayConfig {
            sources: vec![],
            overlay_repo: None,
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
                url: "https://github.com/org/repo".to_string(),
            }],
            overlay_repo: None,
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
                    url: "https://github.com/org/primary".to_string(),
                },
                Source {
                    name: "secondary".to_string(),
                    url: "https://github.com/org/secondary".to_string(),
                },
            ],
            overlay_repo: None,
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
                url: "https://github.com/org/primary".to_string(),
            }],
            overlay_repo: None,
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
}
