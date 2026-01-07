//! State management for repoverlay.
//!
//! Handles overlay state persistence, both in-repo (`.repoverlay/`) and external
//! (`~/.local/share/repoverlay/`) for recovery after `git clean`.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Constants for state directory structure
pub const STATE_DIR: &str = ".repoverlay";
pub const OVERLAYS_DIR: &str = "overlays";
pub const META_FILE: &str = "meta.toml";
pub const CONFIG_FILE: &str = "repoverlay.toml";
pub const GIT_EXCLUDE: &str = ".git/info/exclude";
pub const MANAGED_SECTION_NAME: &str = "managed";

/// Source of an overlay - can be local or from GitHub.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OverlaySource {
    /// Local filesystem overlay
    Local {
        /// Absolute path to the overlay directory
        path: PathBuf,
    },
    /// GitHub repository overlay
    GitHub {
        /// Original URL as provided by user (for display)
        url: String,
        /// Repository owner
        owner: String,
        /// Repository name
        repo: String,
        /// Git ref (branch/tag name or commit SHA)
        git_ref: String,
        /// Resolved commit SHA at time of apply
        commit: String,
        /// Subdirectory within the repo (if any)
        subpath: Option<String>,
        /// When the cache was last updated
        cached_at: DateTime<Utc>,
    },
}

impl OverlaySource {
    /// Create a new local source.
    pub fn local(path: PathBuf) -> Self {
        OverlaySource::Local { path }
    }

    /// Create a new GitHub source.
    pub fn github(
        url: String,
        owner: String,
        repo: String,
        git_ref: String,
        commit: String,
        subpath: Option<String>,
    ) -> Self {
        OverlaySource::GitHub {
            url,
            owner,
            repo,
            git_ref,
            commit,
            subpath,
            cached_at: Utc::now(),
        }
    }

    /// Get a display string for the source.
    #[allow(dead_code)]
    pub fn display(&self) -> String {
        match self {
            OverlaySource::Local { path } => path.display().to_string(),
            OverlaySource::GitHub {
                url,
                git_ref,
                commit,
                ..
            } => {
                format!("{} ({}@{})", url, git_ref, &commit[..12.min(commit.len())])
            }
        }
    }

    /// Check if this is a GitHub source.
    #[allow(dead_code)]
    pub fn is_github(&self) -> bool {
        matches!(self, OverlaySource::GitHub { .. })
    }

    /// Get the local path for this source (for local sources only).
    #[allow(dead_code)]
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            OverlaySource::Local { path } => Some(path),
            OverlaySource::GitHub { .. } => None,
        }
    }
}

/// Global metadata for the .repoverlay directory.
#[derive(Debug, Deserialize, Serialize)]
pub struct GlobalMeta {
    pub version: u32,
}

impl Default for GlobalMeta {
    fn default() -> Self {
        Self { version: 1 }
    }
}

/// State file tracking an applied overlay (.repoverlay/overlays/<name>.toml).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OverlayState {
    pub meta: StateMeta,
    pub files: Vec<FileEntry>,
}

/// Metadata about an applied overlay.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StateMeta {
    pub applied_at: DateTime<Utc>,
    #[serde(flatten)]
    pub source: OverlaySource,
    pub name: String,
}

/// Legacy metadata format (for backward compatibility).
#[derive(Debug, Deserialize)]
pub struct LegacyStateMeta {
    pub applied_at: DateTime<Utc>,
    pub source: PathBuf,
    pub name: String,
}

/// A file entry in the overlay state.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FileEntry {
    pub source: PathBuf,
    pub target: PathBuf,
    #[serde(rename = "type")]
    pub link_type: LinkType,
}

/// Type of file link.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LinkType {
    Symlink,
    Copy,
}

/// Configuration file for an overlay source (repoverlay.toml).
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct OverlayConfig {
    #[serde(default)]
    pub overlay: OverlayConfigMeta,
    #[serde(default)]
    pub mappings: std::collections::HashMap<String, String>,
}

/// Metadata section of overlay config.
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct OverlayConfigMeta {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Get the external state directory for storing backup state.
///
/// Location: `~/.local/share/repoverlay/applied/` (Linux/macOS)
/// or `%LOCALAPPDATA%\repoverlay\applied\` (Windows)
pub fn external_state_dir() -> Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("", "", "repoverlay")
        .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;

    Ok(proj_dirs.data_dir().join("applied"))
}

/// Get the external state directory for a specific target repository.
///
/// Uses a hash of the canonical target path to create a unique directory.
pub fn external_state_dir_for_target(target: &Path) -> Result<PathBuf> {
    let base = external_state_dir()?;
    let target_hash = hash_path(target);
    Ok(base.join(target_hash))
}

/// Save overlay state to the external backup location.
pub fn save_external_state(target: &Path, overlay_name: &str, state: &OverlayState) -> Result<()> {
    let dir = external_state_dir_for_target(target)?;
    fs::create_dir_all(&dir)?;

    // Also save a marker file with the original target path for debugging
    let marker_path = dir.join(".target_path");
    if !marker_path.exists() {
        fs::write(&marker_path, target.display().to_string())?;
    }

    let state_file = dir.join(format!("{}.toml", overlay_name));
    fs::write(&state_file, toml::to_string_pretty(state)?)?;

    Ok(())
}

/// Remove overlay state from the external backup location.
pub fn remove_external_state(target: &Path, overlay_name: &str) -> Result<()> {
    let dir = external_state_dir_for_target(target)?;
    let state_file = dir.join(format!("{}.toml", overlay_name));

    if state_file.exists() {
        fs::remove_file(&state_file)?;
    }

    // Clean up the directory if empty (except for the marker file)
    if dir.exists() {
        let remaining: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != ".target_path")
            .collect();

        if remaining.is_empty() {
            fs::remove_dir_all(&dir)?;
        }
    }

    Ok(())
}

/// Load all overlay states from the external backup location for a target.
pub fn load_external_states(target: &Path) -> Result<Vec<OverlayState>> {
    let dir = external_state_dir_for_target(target)?;

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut states = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().map(|e| e == "toml").unwrap_or(false)
            && path.file_name() != Some(std::ffi::OsStr::new(".target_path"))
        {
            let content = fs::read_to_string(&path)?;
            if let Ok(state) = toml::from_str::<OverlayState>(&content) {
                states.push(state);
            }
        }
    }

    Ok(states)
}

/// Hash a path to create a unique identifier.
fn hash_path(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Generate the start marker for a git exclude section.
pub fn exclude_marker_start(name: &str) -> String {
    format!("# repoverlay:{} start", name)
}

/// Generate the end marker for a git exclude section.
pub fn exclude_marker_end(name: &str) -> String {
    format!("# repoverlay:{} end", name)
}

/// Validate and normalize overlay name for use as filename.
pub fn normalize_overlay_name(name: &str) -> Result<String> {
    let normalized: String = name
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();

    if normalized.is_empty() {
        anyhow::bail!("Invalid overlay name: '{}'", name);
    }
    Ok(normalized)
}

/// Load all target paths from all applied overlays, returning a map of path -> overlay_name.
pub fn load_all_overlay_targets(
    target: &Path,
) -> Result<std::collections::HashMap<String, String>> {
    let mut targets = std::collections::HashMap::new();
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        return Ok(targets);
    }

    for entry in fs::read_dir(&overlays_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "toml").unwrap_or(false) {
            let content = fs::read_to_string(&path)?;
            let state: OverlayState = toml::from_str(&content)?;
            let overlay_name = state.meta.name.clone();

            for file in state.files {
                let target_str = file.target.to_string_lossy().to_string();
                targets.insert(target_str, overlay_name.clone());
            }
        }
    }

    Ok(targets)
}

/// List all applied overlays, returning their normalized names.
pub fn list_applied_overlays(target: &Path) -> Result<Vec<String>> {
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        return Ok(Vec::new());
    }

    let mut names: Vec<String> = fs::read_dir(&overlays_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "toml")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .collect();

    names.sort();
    Ok(names)
}

/// Load an overlay state from the in-repo state file.
pub fn load_overlay_state(target: &Path, name: &str) -> Result<OverlayState> {
    let state_file = target
        .join(STATE_DIR)
        .join(OVERLAYS_DIR)
        .join(format!("{}.toml", name));

    let content = fs::read_to_string(&state_file)
        .with_context(|| format!("Failed to read overlay state: {}", name))?;

    // Try to parse with new format first, fall back to legacy
    if let Ok(state) = toml::from_str::<OverlayState>(&content) {
        return Ok(state);
    }

    // Try legacy format and convert
    #[derive(Deserialize)]
    struct LegacyOverlayState {
        meta: LegacyStateMeta,
        files: Vec<FileEntry>,
    }

    let legacy: LegacyOverlayState = toml::from_str(&content)
        .with_context(|| format!("Failed to parse overlay state: {}", name))?;

    Ok(OverlayState {
        meta: StateMeta {
            applied_at: legacy.meta.applied_at,
            source: OverlaySource::Local {
                path: legacy.meta.source,
            },
            name: legacy.meta.name,
        },
        files: legacy.files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_normalize_overlay_name() {
        assert_eq!(normalize_overlay_name("my-overlay").unwrap(), "my-overlay");
        assert_eq!(normalize_overlay_name("My Overlay").unwrap(), "my-overlay");
        assert_eq!(
            normalize_overlay_name("test_overlay_123").unwrap(),
            "test_overlay_123"
        );
        assert!(normalize_overlay_name("").is_err());
        assert!(normalize_overlay_name("!!!").is_err());
    }

    #[test]
    fn test_overlay_source_local() {
        let source = OverlaySource::local(PathBuf::from("/path/to/overlay"));
        assert!(!source.is_github());
        assert_eq!(source.local_path(), Some(Path::new("/path/to/overlay")));
        assert!(source.display().contains("/path/to/overlay"));
    }

    #[test]
    fn test_overlay_source_github() {
        let source = OverlaySource::github(
            "https://github.com/owner/repo".to_string(),
            "owner".to_string(),
            "repo".to_string(),
            "main".to_string(),
            "abc123def456".to_string(),
            None,
        );
        assert!(source.is_github());
        assert_eq!(source.local_path(), None);
        assert!(source.display().contains("github.com"));
    }

    #[test]
    fn test_hash_path_consistency() {
        let path = Path::new("/test/path");
        let hash1 = hash_path(path);
        let hash2 = hash_path(path);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_path_uniqueness() {
        let hash1 = hash_path(Path::new("/path/one"));
        let hash2 = hash_path(Path::new("/path/two"));
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_external_state_roundtrip() {
        let temp_target = TempDir::new().unwrap();
        let target_path = temp_target.path();

        let state = OverlayState {
            meta: StateMeta {
                applied_at: Utc::now(),
                source: OverlaySource::local(PathBuf::from("/overlay/source")),
                name: "test-overlay".to_string(),
            },
            files: vec![FileEntry {
                source: PathBuf::from(".envrc"),
                target: PathBuf::from(".envrc"),
                link_type: LinkType::Symlink,
            }],
        };

        // Save
        save_external_state(target_path, "test-overlay", &state).unwrap();

        // Load
        let loaded = load_external_states(target_path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].meta.name, "test-overlay");

        // Remove
        remove_external_state(target_path, "test-overlay").unwrap();
        let after_remove = load_external_states(target_path).unwrap();
        assert!(after_remove.is_empty());
    }

    #[test]
    fn test_exclude_markers() {
        assert_eq!(exclude_marker_start("test"), "# repoverlay:test start");
        assert_eq!(exclude_marker_end("test"), "# repoverlay:test end");
    }
}
