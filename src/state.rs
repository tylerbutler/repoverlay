//! State management for repoverlay.
//!
//! Handles overlay state persistence, both in-repo (`.repoverlay/`) and external
//! (`~/.local/share/repoverlay/`) for recovery after `git clean`.
//!
//! Due to sickle's serde limitations (no support for nested maps or complex structures),
//! state files use a completely flat format with string-encoded data.

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
pub const META_FILE: &str = "meta.ccl";
pub const CONFIG_FILE: &str = "repoverlay.ccl";
pub const GIT_EXCLUDE: &str = ".git/info/exclude";
pub const MANAGED_SECTION_NAME: &str = "managed";

/// Source of an overlay - can be local, from GitHub, or from a shared overlay repository.
/// This is used for in-memory representation; see OverlayStateFlat for serialization.
#[derive(Debug, Clone)]
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
    /// Overlay from a shared overlay repository (org/repo/name format)
    OverlayRepo {
        /// Target organization (e.g., "microsoft")
        org: String,
        /// Target repository (e.g., "FluidFramework")
        repo: String,
        /// Overlay name (e.g., "claude-config")
        name: String,
        /// Commit SHA at time of apply
        commit: String,
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

    /// Create a new overlay repository source.
    pub fn overlay_repo(org: String, repo: String, name: String, commit: String) -> Self {
        OverlaySource::OverlayRepo {
            org,
            repo,
            name,
            commit,
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
            OverlaySource::OverlayRepo {
                org,
                repo,
                name,
                commit,
            } => {
                format!(
                    "{}/{}/{} (@{})",
                    org,
                    repo,
                    name,
                    &commit[..12.min(commit.len())]
                )
            }
        }
    }

    /// Check if this is a GitHub source.
    #[allow(dead_code)]
    pub fn is_github(&self) -> bool {
        matches!(self, OverlaySource::GitHub { .. })
    }

    /// Check if this is an overlay repository source.
    #[allow(dead_code)]
    pub fn is_overlay_repo(&self) -> bool {
        matches!(self, OverlaySource::OverlayRepo { .. })
    }

    /// Get the local path for this source (for local sources only).
    #[allow(dead_code)]
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            OverlaySource::Local { path } => Some(path),
            OverlaySource::GitHub { .. } | OverlaySource::OverlayRepo { .. } => None,
        }
    }

    /// Encode source as a string for flat storage.
    /// Format: "type|field1|field2|..."
    fn to_encoded_string(&self) -> String {
        match self {
            OverlaySource::Local { path } => {
                format!("local|{}", path.to_string_lossy())
            }
            OverlaySource::GitHub {
                url,
                owner,
                repo,
                git_ref,
                commit,
                subpath,
                cached_at,
            } => {
                format!(
                    "github|{}|{}|{}|{}|{}|{}|{}",
                    url,
                    owner,
                    repo,
                    git_ref,
                    commit,
                    subpath.as_deref().unwrap_or(""),
                    cached_at.to_rfc3339()
                )
            }
            OverlaySource::OverlayRepo {
                org,
                repo,
                name,
                commit,
            } => {
                format!("overlay_repo|{}|{}|{}|{}", org, repo, name, commit)
            }
        }
    }

    /// Decode source from encoded string.
    fn from_encoded_string(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.splitn(8, '|').collect();
        if parts.is_empty() {
            return None;
        }

        match parts[0] {
            "local" if parts.len() >= 2 => Some(OverlaySource::Local {
                path: PathBuf::from(parts[1]),
            }),
            "github" if parts.len() >= 8 => {
                let cached_at = DateTime::parse_from_rfc3339(parts[7])
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Some(OverlaySource::GitHub {
                    url: parts[1].to_string(),
                    owner: parts[2].to_string(),
                    repo: parts[3].to_string(),
                    git_ref: parts[4].to_string(),
                    commit: parts[5].to_string(),
                    subpath: if parts[6].is_empty() {
                        None
                    } else {
                        Some(parts[6].to_string())
                    },
                    cached_at,
                })
            }
            "overlay_repo" if parts.len() >= 5 => Some(OverlaySource::OverlayRepo {
                org: parts[1].to_string(),
                repo: parts[2].to_string(),
                name: parts[3].to_string(),
                commit: parts[4].to_string(),
            }),
            _ => None,
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

/// Flat state file format for CCL serialization.
/// All fields are primitive strings to work around sickle's limitations.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OverlayStateFlat {
    /// Overlay display name
    pub name: String,
    /// ISO 8601 timestamp when applied
    pub applied_at: String,
    /// Encoded source info (type|field1|field2|...)
    pub source: String,
    /// Comma-separated list of "target_path:source_path:link_type" entries
    #[serde(default)]
    pub files: String,
}

/// State file tracking an applied overlay (.repoverlay/overlays/<name>.ccl).
/// In-memory representation with proper types.
#[derive(Debug, Clone)]
pub struct OverlayState {
    pub name: String,
    pub applied_at: DateTime<Utc>,
    pub source: OverlaySource,
    pub files: Vec<FileEntry>,
}

impl OverlayState {
    /// Create a new overlay state.
    pub fn new(name: String, source: OverlaySource) -> Self {
        Self {
            name,
            applied_at: Utc::now(),
            source,
            files: Vec::new(),
        }
    }

    /// Add a file entry to the state.
    pub fn add_file(&mut self, entry: FileEntry) {
        self.files.push(entry);
    }

    /// Convert to flat format for serialization.
    pub fn to_flat(&self) -> OverlayStateFlat {
        let files_str = self
            .files
            .iter()
            .map(|f| {
                let link_str = match f.link_type {
                    LinkType::Symlink => "symlink",
                    LinkType::Copy => "copy",
                };
                format!(
                    "{}:{}:{}",
                    f.target.to_string_lossy(),
                    f.source.to_string_lossy(),
                    link_str
                )
            })
            .collect::<Vec<_>>()
            .join(",");

        OverlayStateFlat {
            name: self.name.clone(),
            applied_at: self.applied_at.to_rfc3339(),
            source: self.source.to_encoded_string(),
            files: files_str,
        }
    }

    /// Create from flat format after deserialization.
    pub fn from_flat(flat: OverlayStateFlat) -> Option<Self> {
        let applied_at = DateTime::parse_from_rfc3339(&flat.applied_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let source = OverlaySource::from_encoded_string(&flat.source)?;

        let files = if flat.files.is_empty() {
            Vec::new()
        } else {
            flat.files
                .split(',')
                .filter_map(|entry| {
                    let parts: Vec<&str> = entry.splitn(3, ':').collect();
                    if parts.len() == 3 {
                        let link_type = match parts[2] {
                            "copy" => LinkType::Copy,
                            _ => LinkType::Symlink,
                        };
                        Some(FileEntry {
                            target: PathBuf::from(parts[0]),
                            source: PathBuf::from(parts[1]),
                            link_type,
                        })
                    } else {
                        None
                    }
                })
                .collect()
        };

        Some(Self {
            name: flat.name,
            applied_at,
            source,
            files,
        })
    }

    /// Get the number of files in the overlay.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Iterate over file entries.
    pub fn file_entries(&self) -> &[FileEntry] {
        &self.files
    }
}

/// A file entry in the overlay state.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub source: PathBuf,
    pub target: PathBuf,
    pub link_type: LinkType,
}

/// Type of file link.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinkType {
    Symlink,
    Copy,
}

/// Configuration file for an overlay source (repoverlay.ccl).
/// Note: This uses nested structures which won't roundtrip through sickle,
/// but it's only read (not written) by repoverlay.
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

    let state_file = dir.join(format!("{}.ccl", overlay_name));
    let flat = state.to_flat();
    let content = sickle::to_string(&flat).context("Failed to serialize state to CCL")?;
    fs::write(&state_file, content)?;

    Ok(())
}

/// Remove overlay state from the external backup location.
pub fn remove_external_state(target: &Path, overlay_name: &str) -> Result<()> {
    let dir = external_state_dir_for_target(target)?;
    let state_file = dir.join(format!("{}.ccl", overlay_name));

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

        if path.extension().map(|e| e == "ccl").unwrap_or(false)
            && path.file_name() != Some(std::ffi::OsStr::new(".target_path"))
        {
            let content = fs::read_to_string(&path)?;
            if let Ok(flat) = sickle::from_str::<OverlayStateFlat>(&content)
                && let Some(state) = OverlayState::from_flat(flat)
            {
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
        if path.extension().map(|e| e == "ccl").unwrap_or(false) {
            let content = fs::read_to_string(&path)?;
            if let Ok(flat) = sickle::from_str::<OverlayStateFlat>(&content)
                && let Some(state) = OverlayState::from_flat(flat)
            {
                for file in &state.files {
                    targets.insert(
                        file.target.to_string_lossy().to_string(),
                        state.name.clone(),
                    );
                }
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
                .map(|ext| ext == "ccl")
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
        .join(format!("{}.ccl", name));

    let content = fs::read_to_string(&state_file)
        .with_context(|| format!("Failed to read overlay state: {}", name))?;

    let flat: OverlayStateFlat = sickle::from_str(&content)
        .with_context(|| format!("Failed to parse overlay state: {}", name))?;

    OverlayState::from_flat(flat)
        .ok_or_else(|| anyhow::anyhow!("Failed to decode overlay state: {}", name))
}

/// Save an overlay state to the in-repo state file.
pub fn save_overlay_state(target: &Path, state: &OverlayState) -> Result<()> {
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    fs::create_dir_all(&overlays_dir)?;

    let normalized_name = normalize_overlay_name(&state.name)?;
    let state_file = overlays_dir.join(format!("{}.ccl", normalized_name));

    let flat = state.to_flat();
    let content = sickle::to_string(&flat).context("Failed to serialize overlay state")?;
    fs::write(&state_file, content)?;

    Ok(())
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
    fn test_overlay_source_encoding_local() {
        let source = OverlaySource::local(PathBuf::from("/path/to/overlay"));
        let encoded = source.to_encoded_string();
        let decoded = OverlaySource::from_encoded_string(&encoded).unwrap();

        match decoded {
            OverlaySource::Local { path } => {
                assert_eq!(path, PathBuf::from("/path/to/overlay"));
            }
            _ => panic!("Expected Local source"),
        }
    }

    #[test]
    fn test_overlay_source_encoding_github() {
        let source = OverlaySource::github(
            "https://github.com/owner/repo".to_string(),
            "owner".to_string(),
            "repo".to_string(),
            "main".to_string(),
            "abc123def456".to_string(),
            Some("subdir".to_string()),
        );
        let encoded = source.to_encoded_string();
        let decoded = OverlaySource::from_encoded_string(&encoded).unwrap();

        match decoded {
            OverlaySource::GitHub {
                url, owner, repo, ..
            } => {
                assert_eq!(url, "https://github.com/owner/repo");
                assert_eq!(owner, "owner");
                assert_eq!(repo, "repo");
            }
            _ => panic!("Expected GitHub source"),
        }
    }

    #[test]
    fn test_overlay_state_flat_roundtrip() {
        let mut state = OverlayState::new(
            "test-overlay".to_string(),
            OverlaySource::local(PathBuf::from("/overlay/source")),
        );
        state.add_file(FileEntry {
            source: PathBuf::from(".envrc"),
            target: PathBuf::from(".envrc"),
            link_type: LinkType::Symlink,
        });
        state.add_file(FileEntry {
            source: PathBuf::from("config.json"),
            target: PathBuf::from(".config/app/config.json"),
            link_type: LinkType::Copy,
        });

        let flat = state.to_flat();
        let restored = OverlayState::from_flat(flat).unwrap();

        assert_eq!(restored.name, "test-overlay");
        assert_eq!(restored.files.len(), 2);
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

        let mut state = OverlayState::new(
            "test-overlay".to_string(),
            OverlaySource::local(PathBuf::from("/overlay/source")),
        );
        state.add_file(FileEntry {
            source: PathBuf::from(".envrc"),
            target: PathBuf::from(".envrc"),
            link_type: LinkType::Symlink,
        });

        // Save
        save_external_state(target_path, "test-overlay", &state).unwrap();

        // Load
        let loaded = load_external_states(target_path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "test-overlay");

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
