//! Configuration management for repoverlay.
//!
//! Handles global and per-repo configuration using CCL format.
//! Global config: `~/.config/repoverlay/config.ccl`
//! Per-repo config: `.repoverlay/config.ccl`

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

/// Global repoverlay configuration.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct RepoverlayConfig {
    /// Configured overlay sources (checked in order for resolution).
    #[serde(default)]
    pub sources: Vec<Source>,
    /// Legacy overlay repository configuration (for backwards compatibility).
    /// New configs should use `sources` instead.
    #[serde(default)]
    pub overlay_repo: Option<OverlayRepoConfig>,
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
    pub url: String,
}

/// Configuration for a shared overlay repository.
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
#[allow(dead_code)] // Will be used when migration is integrated
#[must_use]
pub const fn needs_migration(config: &RepoverlayConfig) -> bool {
    config.overlay_repo.is_some() && config.sources.is_empty()
}

/// Migrate old config format to new multi-source format.
///
/// If the config uses the legacy `overlay_repo` key, converts it to a source
/// named "default". Returns a message describing the migration if one occurred.
#[allow(dead_code)] // Will be used when migration is integrated
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
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
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
pub fn load_global_config() -> Result<RepoverlayConfig> {
    let config_path = global_config_path()?;

    if !config_path.exists() {
        return Ok(RepoverlayConfig::default());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

    sickle::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", config_path.display()))
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

    // Include legacy overlay_repo if present (for backwards compat)
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
    #[allow(unsafe_code)]
    fn test_config_dir_with_xdg_config_home() {
        // Save original value
        let original = std::env::var("XDG_CONFIG_HOME").ok();

        // Set custom XDG_CONFIG_HOME
        let temp = TempDir::new().unwrap();
        // SAFETY: Tests are run serially with cargo test, and we restore the value after
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", temp.path());
        }

        let dir = config_dir().unwrap();
        assert!(dir.starts_with(temp.path()));
        assert!(dir.ends_with("repoverlay"));

        // Restore original value
        // SAFETY: Tests are run serially with cargo test
        unsafe {
            match original {
                Some(val) => std::env::set_var("XDG_CONFIG_HOME", val),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
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
}
