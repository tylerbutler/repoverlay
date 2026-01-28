//! State management for repoverlay.
//!
//! Handles overlay state persistence, both in-repo (`.repoverlay/`) and external
//! (`~/.local/share/repoverlay/`) for recovery after `git clean`.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use log::debug;
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

/// How an overlay was resolved from a reference.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedVia {
    /// Resolved directly (exact org/repo match)
    Direct,
    /// Resolved via upstream fallback
    Upstream,
}

/// Source of an overlay - can be local, from GitHub, or from a shared overlay repository.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
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
        #[serde(default)]
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
        /// How this overlay was resolved (direct match or upstream fallback)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_via: Option<ResolvedVia>,
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
    #[allow(dead_code)]
    pub fn overlay_repo(org: String, repo: String, name: String, commit: String) -> Self {
        OverlaySource::OverlayRepo {
            org,
            repo,
            name,
            commit,
            resolved_via: None,
        }
    }

    /// Create a new overlay repository source with resolution info.
    pub fn overlay_repo_with_resolution(
        org: String,
        repo: String,
        name: String,
        commit: String,
        resolved_via: ResolvedVia,
    ) -> Self {
        OverlaySource::OverlayRepo {
            org,
            repo,
            name,
            commit,
            resolved_via: Some(resolved_via),
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
                resolved_via,
            } => {
                let via = match resolved_via {
                    Some(ResolvedVia::Upstream) => " via upstream",
                    _ => "",
                };
                format!(
                    "{}/{}/{}{} (@{})",
                    org,
                    repo,
                    name,
                    via,
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

/// State file tracking an applied overlay (`.repoverlay/overlays/<name>.ccl`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OverlayState {
    pub name: String,
    pub applied_at: DateTime<Utc>,
    pub source: OverlaySource,
    #[serde(default)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileEntry {
    pub source: PathBuf,
    pub target: PathBuf,
    pub link_type: LinkType,
    /// Type of entry - File (default) or Directory.
    /// Backwards compatible: missing field defaults to File.
    #[serde(default)]
    pub entry_type: EntryType,
}

/// Type of file link.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkType {
    Symlink,
    Copy,
}

/// Type of entry (file or directory).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    #[default]
    File,
    Directory,
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
    /// Directories to symlink as a unit (not walk their contents).
    /// These directories will be symlinked directly instead of having
    /// their individual files symlinked.
    #[serde(default)]
    pub directories: Vec<String>,
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
    debug!("save_external_state: {}", overlay_name);
    let dir = external_state_dir_for_target(target)?;
    fs::create_dir_all(&dir)?;

    // Also save a marker file with the original target path for debugging
    let marker_path = dir.join(".target_path");
    if !marker_path.exists() {
        fs::write(&marker_path, target.display().to_string())?;
    }

    let state_file = dir.join(format!("{}.ccl", overlay_name));
    let content = sickle::to_string(state).context("Failed to serialize state to CCL")?;
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
    debug!("load_external_states: {}", target.display());
    let dir = external_state_dir_for_target(target)?;

    if !dir.exists() {
        debug!("no external state directory found");
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
            if let Ok(state) = sickle::from_str::<OverlayState>(&content) {
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
            if let Ok(state) = sickle::from_str::<OverlayState>(&content) {
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
    debug!("load_overlay_state: {}", name);
    let state_file = target
        .join(STATE_DIR)
        .join(OVERLAYS_DIR)
        .join(format!("{}.ccl", name));

    let content = fs::read_to_string(&state_file)
        .with_context(|| format!("Failed to read overlay state: {}", name))?;

    sickle::from_str(&content).with_context(|| format!("Failed to parse overlay state: {}", name))
}

/// Save an overlay state to the in-repo state file.
pub fn save_overlay_state(target: &Path, state: &OverlayState) -> Result<()> {
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    fs::create_dir_all(&overlays_dir)?;

    let normalized_name = normalize_overlay_name(&state.name)?;
    let state_file = overlays_dir.join(format!("{}.ccl", normalized_name));

    let content = sickle::to_string(state).context("Failed to serialize overlay state")?;
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
    fn test_overlay_source_serde_roundtrip_local() {
        let source = OverlaySource::local(PathBuf::from("/path/to/overlay"));
        let serialized = sickle::to_string(&source).unwrap();
        let deserialized: OverlaySource = sickle::from_str(&serialized).unwrap();

        match deserialized {
            OverlaySource::Local { path } => {
                assert_eq!(path, PathBuf::from("/path/to/overlay"));
            }
            _ => panic!("Expected Local source"),
        }
    }

    #[test]
    fn test_overlay_source_serde_roundtrip_github() {
        let source = OverlaySource::github(
            "https://github.com/owner/repo".to_string(),
            "owner".to_string(),
            "repo".to_string(),
            "main".to_string(),
            "abc123def456".to_string(),
            Some("subdir".to_string()),
        );
        let serialized = sickle::to_string(&source).unwrap();
        let deserialized: OverlaySource = sickle::from_str(&serialized).unwrap();

        match deserialized {
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
    fn test_overlay_state_serde_roundtrip() {
        let mut state = OverlayState::new(
            "test-overlay".to_string(),
            OverlaySource::local(PathBuf::from("/overlay/source")),
        );
        state.add_file(FileEntry {
            source: PathBuf::from(".envrc"),
            target: PathBuf::from(".envrc"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::File,
        });
        state.add_file(FileEntry {
            source: PathBuf::from("config.json"),
            target: PathBuf::from(".config/app/config.json"),
            link_type: LinkType::Copy,
            entry_type: EntryType::File,
        });

        let serialized = sickle::to_string(&state).unwrap();
        let restored: OverlayState = sickle::from_str(&serialized).unwrap();

        assert_eq!(restored.name, "test-overlay");
        assert_eq!(restored.files.len(), 2);
        assert_eq!(restored.files[0].link_type, LinkType::Symlink);
        assert_eq!(restored.files[1].link_type, LinkType::Copy);
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
            entry_type: EntryType::File,
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

    #[test]
    fn test_overlay_source_overlay_repo_with_resolved_via() {
        let source = OverlaySource::OverlayRepo {
            org: "microsoft".to_string(),
            repo: "FluidFramework".to_string(),
            name: "claude-config".to_string(),
            commit: "abc123".to_string(),
            resolved_via: Some(ResolvedVia::Upstream),
        };

        let serialized = sickle::to_string(&source).unwrap();
        let deserialized: OverlaySource = sickle::from_str(&serialized).unwrap();

        match deserialized {
            OverlaySource::OverlayRepo { resolved_via, .. } => {
                assert_eq!(resolved_via, Some(ResolvedVia::Upstream));
            }
            _ => panic!("Expected OverlayRepo"),
        }
    }

    #[test]
    fn test_resolved_via_direct_is_default() {
        let source = OverlaySource::OverlayRepo {
            org: "tylerbutler".to_string(),
            repo: "FluidFramework".to_string(),
            name: "claude-config".to_string(),
            commit: "abc123".to_string(),
            resolved_via: None,
        };

        let serialized = sickle::to_string(&source).unwrap();
        // Should work without resolved_via field
        assert!(!serialized.contains("resolved_via") || serialized.contains("resolved_via = "));
    }

    #[test]
    fn test_overlay_source_overlay_repo() {
        let source = OverlaySource::overlay_repo(
            "microsoft".to_string(),
            "FluidFramework".to_string(),
            "claude-config".to_string(),
            "abc123def456".to_string(),
        );
        assert!(source.is_overlay_repo());
        assert!(!source.is_github());
        assert_eq!(source.local_path(), None);
        assert!(
            source
                .display()
                .contains("microsoft/FluidFramework/claude-config")
        );
    }

    #[test]
    fn test_overlay_source_display_github_short_commit() {
        let source = OverlaySource::github(
            "https://github.com/owner/repo".to_string(),
            "owner".to_string(),
            "repo".to_string(),
            "main".to_string(),
            "abc".to_string(), // Short commit
            None,
        );
        let display = source.display();
        assert!(display.contains("abc")); // Should handle short commits gracefully
    }

    #[test]
    fn test_overlay_source_display_overlay_repo_via_upstream() {
        let source = OverlaySource::OverlayRepo {
            org: "microsoft".to_string(),
            repo: "FluidFramework".to_string(),
            name: "claude-config".to_string(),
            commit: "abc123def456".to_string(),
            resolved_via: Some(ResolvedVia::Upstream),
        };
        let display = source.display();
        assert!(display.contains("via upstream"));
    }

    #[test]
    fn test_overlay_state_methods() {
        let mut state = OverlayState::new(
            "test".to_string(),
            OverlaySource::local(PathBuf::from("/path")),
        );

        assert_eq!(state.file_count(), 0);
        assert!(state.file_entries().is_empty());

        state.add_file(FileEntry {
            source: PathBuf::from("a.txt"),
            target: PathBuf::from("a.txt"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::File,
        });

        assert_eq!(state.file_count(), 1);
        assert_eq!(state.file_entries().len(), 1);
    }

    #[test]
    fn test_global_meta_default() {
        let meta = GlobalMeta::default();
        assert_eq!(meta.version, 1);
    }

    #[test]
    fn test_list_applied_overlays_empty() {
        let temp = TempDir::new().unwrap();
        let overlays = list_applied_overlays(temp.path()).unwrap();
        assert!(overlays.is_empty());
    }

    #[test]
    fn test_list_applied_overlays_with_overlays() {
        let temp = TempDir::new().unwrap();
        let overlays_dir = temp.path().join(STATE_DIR).join(OVERLAYS_DIR);
        fs::create_dir_all(&overlays_dir).unwrap();

        // Create some overlay state files
        fs::write(overlays_dir.join("alpha.ccl"), "name = alpha").unwrap();
        fs::write(overlays_dir.join("beta.ccl"), "name = beta").unwrap();
        fs::write(overlays_dir.join("gamma.ccl"), "name = gamma").unwrap();

        let overlays = list_applied_overlays(temp.path()).unwrap();
        assert_eq!(overlays.len(), 3);
        // Should be sorted
        assert_eq!(overlays[0], "alpha");
        assert_eq!(overlays[1], "beta");
        assert_eq!(overlays[2], "gamma");
    }

    #[test]
    fn test_list_applied_overlays_ignores_non_ccl_files() {
        let temp = TempDir::new().unwrap();
        let overlays_dir = temp.path().join(STATE_DIR).join(OVERLAYS_DIR);
        fs::create_dir_all(&overlays_dir).unwrap();

        fs::write(overlays_dir.join("overlay.ccl"), "name = overlay").unwrap();
        fs::write(overlays_dir.join("readme.txt"), "not an overlay").unwrap();
        fs::write(overlays_dir.join("meta.json"), "{}").unwrap();

        let overlays = list_applied_overlays(temp.path()).unwrap();
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0], "overlay");
    }

    #[test]
    fn test_load_all_overlay_targets_empty() {
        let temp = TempDir::new().unwrap();
        let targets = load_all_overlay_targets(temp.path()).unwrap();
        assert!(targets.is_empty());
    }

    #[test]
    fn test_load_all_overlay_targets_with_files() {
        let temp = TempDir::new().unwrap();
        let overlays_dir = temp.path().join(STATE_DIR).join(OVERLAYS_DIR);
        fs::create_dir_all(&overlays_dir).unwrap();

        // Create a proper overlay state
        let state = OverlayState {
            name: "test-overlay".to_string(),
            applied_at: Utc::now(),
            source: OverlaySource::local(PathBuf::from("/path")),
            files: vec![
                FileEntry {
                    source: PathBuf::from(".envrc"),
                    target: PathBuf::from(".envrc"),
                    link_type: LinkType::Symlink,
                    entry_type: EntryType::File,
                },
                FileEntry {
                    source: PathBuf::from("config.json"),
                    target: PathBuf::from(".config/app.json"),
                    link_type: LinkType::Copy,
                    entry_type: EntryType::File,
                },
            ],
        };
        let content = sickle::to_string(&state).unwrap();
        fs::write(overlays_dir.join("test-overlay.ccl"), content).unwrap();

        let targets = load_all_overlay_targets(temp.path()).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets.get(".envrc"), Some(&"test-overlay".to_string()));
        assert_eq!(
            targets.get(".config/app.json"),
            Some(&"test-overlay".to_string())
        );
    }

    #[test]
    fn test_save_and_load_overlay_state() {
        let temp = TempDir::new().unwrap();

        let mut state = OverlayState::new(
            "my-overlay".to_string(),
            OverlaySource::local(PathBuf::from("/source/path")),
        );
        state.add_file(FileEntry {
            source: PathBuf::from(".envrc"),
            target: PathBuf::from(".envrc"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::File,
        });

        // Save
        save_overlay_state(temp.path(), &state).unwrap();

        // Verify file exists
        let state_file = temp
            .path()
            .join(STATE_DIR)
            .join(OVERLAYS_DIR)
            .join("my-overlay.ccl");
        assert!(state_file.exists());

        // Load
        let loaded = load_overlay_state(temp.path(), "my-overlay").unwrap();
        assert_eq!(loaded.name, "my-overlay");
        assert_eq!(loaded.files.len(), 1);
    }

    #[test]
    fn test_load_overlay_state_not_found() {
        let temp = TempDir::new().unwrap();
        let result = load_overlay_state(temp.path(), "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_overlay_name_special_chars() {
        assert_eq!(normalize_overlay_name("my overlay!").unwrap(), "my-overlay");
        assert_eq!(normalize_overlay_name("Test@123").unwrap(), "test123");
        assert_eq!(normalize_overlay_name("  spaces  ").unwrap(), "--spaces--");
    }

    #[test]
    fn test_normalize_overlay_name_preserves_underscores() {
        assert_eq!(
            normalize_overlay_name("my_overlay_name").unwrap(),
            "my_overlay_name"
        );
    }

    #[test]
    fn test_external_state_multiple_overlays() {
        let temp_target = TempDir::new().unwrap();
        let target_path = temp_target.path();

        // Save multiple overlays
        let state1 = OverlayState::new(
            "overlay-a".to_string(),
            OverlaySource::local(PathBuf::from("/source/a")),
        );
        let state2 = OverlayState::new(
            "overlay-b".to_string(),
            OverlaySource::local(PathBuf::from("/source/b")),
        );

        save_external_state(target_path, "overlay-a", &state1).unwrap();
        save_external_state(target_path, "overlay-b", &state2).unwrap();

        // Load all
        let loaded = load_external_states(target_path).unwrap();
        assert_eq!(loaded.len(), 2);

        // Remove one
        remove_external_state(target_path, "overlay-a").unwrap();
        let after = load_external_states(target_path).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "overlay-b");
    }

    #[test]
    fn test_remove_external_state_nonexistent() {
        let temp_target = TempDir::new().unwrap();
        // Should not error when removing nonexistent state
        let result = remove_external_state(temp_target.path(), "nonexistent");
        assert!(result.is_ok());
    }

    #[test]
    fn test_link_type_serde() {
        // Test Symlink
        let entry = FileEntry {
            source: PathBuf::from("src"),
            target: PathBuf::from("dst"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::File,
        };
        let s = sickle::to_string(&entry).unwrap();
        assert!(s.contains("symlink"));

        // Test Copy
        let entry2 = FileEntry {
            source: PathBuf::from("src"),
            target: PathBuf::from("dst"),
            link_type: LinkType::Copy,
            entry_type: EntryType::File,
        };
        let s2 = sickle::to_string(&entry2).unwrap();
        assert!(s2.contains("copy"));
    }

    #[test]
    fn test_resolved_via_serde() {
        let direct = ResolvedVia::Direct;
        let upstream = ResolvedVia::Upstream;

        // Create sources with each resolution type
        let source_direct = OverlaySource::overlay_repo_with_resolution(
            "org".to_string(),
            "repo".to_string(),
            "name".to_string(),
            "abc123".to_string(),
            direct,
        );
        let source_upstream = OverlaySource::overlay_repo_with_resolution(
            "org".to_string(),
            "repo".to_string(),
            "name".to_string(),
            "abc123".to_string(),
            upstream,
        );

        let s1 = sickle::to_string(&source_direct).unwrap();
        let s2 = sickle::to_string(&source_upstream).unwrap();

        assert!(s1.contains("direct"));
        assert!(s2.contains("upstream"));
    }

    #[test]
    fn test_entry_type_serde() {
        // Test File entry type
        let entry_file = FileEntry {
            source: PathBuf::from("src"),
            target: PathBuf::from("dst"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::File,
        };
        let s = sickle::to_string(&entry_file).unwrap();
        assert!(s.contains("file"));

        // Test Directory entry type
        let entry_dir = FileEntry {
            source: PathBuf::from("scratch"),
            target: PathBuf::from("scratch"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::Directory,
        };
        let s2 = sickle::to_string(&entry_dir).unwrap();
        assert!(s2.contains("directory"));
    }

    #[test]
    fn test_entry_type_default() {
        // EntryType should default to File
        assert_eq!(EntryType::default(), EntryType::File);
    }

    #[test]
    fn test_entry_type_equality() {
        assert_eq!(EntryType::File, EntryType::File);
        assert_eq!(EntryType::Directory, EntryType::Directory);
        assert_ne!(EntryType::File, EntryType::Directory);
    }

    #[test]
    fn test_overlay_config_with_directories() {
        let config_str = r#"
overlay =
  name = test-overlay

directories =
  = scratch
  = .claude
"#;
        let config: OverlayConfig = sickle::from_str(config_str).unwrap();
        assert_eq!(config.overlay.name, Some("test-overlay".to_string()));
        assert_eq!(config.directories.len(), 2);
        assert!(config.directories.contains(&"scratch".to_string()));
        assert!(config.directories.contains(&".claude".to_string()));
    }

    #[test]
    fn test_overlay_config_empty_directories() {
        let config_str = r#"
overlay =
  name = test-overlay
"#;
        let config: OverlayConfig = sickle::from_str(config_str).unwrap();
        assert!(config.directories.is_empty());
    }

    // TODO: Enable once tylerbutler/santa#71 is fixed
    // Forward slashes in map keys currently cause parsing errors in sickle
    #[test]
    #[ignore]
    fn test_overlay_config_mappings_with_forward_slashes() {
        let config_str = r#"
overlay =
  name = test-overlay

mappings =
  config/settings.json = .vscode/settings.json
  src/template.env = .env
"#;
        let config: OverlayConfig = sickle::from_str(config_str).unwrap();
        assert_eq!(config.mappings.len(), 2);
        assert_eq!(
            config.mappings.get("config/settings.json"),
            Some(&".vscode/settings.json".to_string())
        );
        assert_eq!(
            config.mappings.get("src/template.env"),
            Some(&".env".to_string())
        );
    }

    #[test]
    fn test_load_all_overlay_targets_with_directories() {
        let temp = TempDir::new().unwrap();

        // Create .repoverlay/overlays directory
        let overlays_dir = temp.path().join(STATE_DIR).join(OVERLAYS_DIR);
        fs::create_dir_all(&overlays_dir).unwrap();

        // Create a state file with directory entry
        let state = OverlayState {
            name: "test-overlay".to_string(),
            source: OverlaySource::local(PathBuf::from("/source")),
            applied_at: chrono::Utc::now(),
            files: vec![
                FileEntry {
                    source: PathBuf::from(".envrc"),
                    target: PathBuf::from(".envrc"),
                    link_type: LinkType::Symlink,
                    entry_type: EntryType::File,
                },
                FileEntry {
                    source: PathBuf::from("scratch"),
                    target: PathBuf::from("scratch"),
                    link_type: LinkType::Symlink,
                    entry_type: EntryType::Directory,
                },
            ],
        };
        let content = sickle::to_string(&state).unwrap();
        fs::write(overlays_dir.join("test-overlay.ccl"), content).unwrap();

        let targets = load_all_overlay_targets(temp.path()).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets.get(".envrc"), Some(&"test-overlay".to_string()));
        assert_eq!(targets.get("scratch"), Some(&"test-overlay".to_string()));
    }

    #[test]
    fn test_file_entry_with_directory_roundtrip() {
        let entry = FileEntry {
            source: PathBuf::from("scratch"),
            target: PathBuf::from("scratch"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::Directory,
        };

        let serialized = sickle::to_string(&entry).unwrap();
        let deserialized: FileEntry = sickle::from_str(&serialized).unwrap();

        assert_eq!(deserialized.source, entry.source);
        assert_eq!(deserialized.target, entry.target);
        assert_eq!(deserialized.link_type, entry.link_type);
        assert_eq!(deserialized.entry_type, entry.entry_type);
    }

    #[test]
    fn test_backwards_compatible_entry_type() {
        // Old state files without entry_type should default to File
        let old_format = r#"
source = /some/path
target = /some/target
link_type = symlink
"#;
        let entry: FileEntry = sickle::from_str(old_format).unwrap();
        assert_eq!(entry.entry_type, EntryType::File);
    }
}
