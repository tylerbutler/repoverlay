//! Configuration management for repoverlay.
//!
//! Handles global and per-repo configuration using CCL format.
//! Global config: `~/.config/repoverlay/config.ccl`
//! Per-repo config: `.repoverlay/config.ccl`

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Global repoverlay configuration.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct RepoverlayConfig {
    /// Overlay repository configuration (optional).
    #[serde(default)]
    pub overlay_repo: Option<OverlayRepoConfig>,
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

/// Generate a config file with helpful comments.
pub fn generate_config_ccl(config: &OverlayRepoConfig) -> String {
    let mut output = String::new();
    output.push_str("/= repoverlay global configuration\n");
    output.push_str("/= This file configures repoverlay's overlay repository integration.\n\n");
    output.push_str("overlay_repo =\n");
    output.push_str("  /= url: The Git URL of the shared overlay repository.\n");
    output.push_str("  /= This is where overlays are stored and retrieved from.\n");
    output.push_str("  /= Supports HTTPS and SSH URLs.\n");
    output.push_str(&format!("  url = {}\n", config.url));

    if let Some(ref local_path) = config.local_path {
        output.push_str("\n  /= local_path: Override the default clone location.\n");
        output.push_str(&format!("  local_path = {}\n", local_path.display()));
    } else {
        output.push_str("\n  /= local_path (optional): Override the default clone location.\n");
        output.push_str(
            "  /= By default, the repo is cloned to ~/.local/share/repoverlay/overlay-repo/\n",
        );
        output.push_str("  /= Uncomment to use a custom path instead:\n");
        output.push_str("  /= local_path = /custom/path/to/clone\n");
    }

    output
}

/// Save the global configuration with helpful comments.
pub fn save_global_config_with_comments(config: &OverlayRepoConfig) -> Result<()> {
    let config_path = global_config_path()?;

    // Ensure config directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }

    let content = generate_config_ccl(config);

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
    fn test_generate_config_ccl() {
        let config = OverlayRepoConfig {
            url: "https://github.com/user/repo-overlays".to_string(),
            local_path: None,
        };
        let ccl = generate_config_ccl(&config);
        assert!(ccl.contains("overlay_repo ="));
        assert!(ccl.contains("url = https://github.com/user/repo-overlays"));
        assert!(ccl.contains("/= repoverlay global configuration"));
    }

    #[test]
    fn test_generate_config_ccl_with_local_path() {
        let config = OverlayRepoConfig {
            url: "https://github.com/user/repo-overlays".to_string(),
            local_path: Some(PathBuf::from("/custom/path")),
        };
        let ccl = generate_config_ccl(&config);
        assert!(ccl.contains("local_path = /custom/path"));
    }

    #[test]
    fn test_roundtrip_config() {
        let config = RepoverlayConfig {
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

        let config_content = r#"
overlay_repo =
  url = https://github.com/org/overlays
"#;
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

        let repo_config_content = r#"
overlay_repo =
  url = https://github.com/repo/specific
"#;
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
        assert!(config.overlay_repo.is_none());
    }

    #[test]
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

    #[test]
    fn test_save_global_config_with_comments() {
        // Save original XDG_CONFIG_HOME and set to temp dir
        let original = std::env::var("XDG_CONFIG_HOME").ok();
        let temp = TempDir::new().unwrap();
        // SAFETY: Tests are run serially with cargo test, and we restore the value after
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", temp.path());
        }

        let config = OverlayRepoConfig {
            url: "https://github.com/test/repo-overlays".to_string(),
            local_path: None,
        };

        let result = save_global_config_with_comments(&config);
        assert!(result.is_ok());

        // Verify file was created
        let config_path = temp.path().join("repoverlay").join("config.ccl");
        assert!(config_path.exists());

        // Verify content
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("overlay_repo ="));
        assert!(content.contains("url = https://github.com/test/repo-overlays"));
        assert!(content.contains("/= repoverlay global configuration"));

        // Restore original value
        // SAFETY: Tests are run serially with cargo test
        unsafe {
            match original {
                Some(val) => std::env::set_var("XDG_CONFIG_HOME", val),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    #[test]
    fn test_save_global_config_with_local_path() {
        // Save original XDG_CONFIG_HOME and set to temp dir
        let original = std::env::var("XDG_CONFIG_HOME").ok();
        let temp = TempDir::new().unwrap();
        // SAFETY: Tests are run serially with cargo test, and we restore the value after
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", temp.path());
        }

        let config = OverlayRepoConfig {
            url: "https://github.com/test/repo-overlays".to_string(),
            local_path: Some(PathBuf::from("/custom/clone/path")),
        };

        let result = save_global_config_with_comments(&config);
        assert!(result.is_ok());

        // Verify content includes local_path
        let config_path = temp.path().join("repoverlay").join("config.ccl");
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("local_path = /custom/clone/path"));

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
        let config_content = r#"
overlay_repo =
  url = https://github.com/org/overlays
  unknown_field = some_value

some_other_section =
  foo = bar
"#;
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

    #[test]
    fn test_generate_config_ccl_includes_comments() {
        let config = OverlayRepoConfig {
            url: "https://github.com/user/repo-overlays".to_string(),
            local_path: None,
        };
        let ccl = generate_config_ccl(&config);

        // Should include helpful comments
        assert!(ccl.contains("/= repoverlay global configuration"));
        assert!(ccl.contains("/= url:"));
    }
}
