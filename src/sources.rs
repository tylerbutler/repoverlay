//! Multi-source overlay resolution.
//!
//! Manages multiple overlay sources with priority-based resolution.
//! Sources are checked in order; first match wins. Key types are
//! `SourceManager` (coordinates resolution across sources) and
//! `ResolvedOverlay` (a successfully located overlay with its source metadata).

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::OverlayName;
use crate::config::{OverlayRepoConfig, Source};
use crate::overlay_repo::{AvailableOverlay, OverlayRepoManager};
use crate::state::ResolvedVia;
use crate::upstream::UpstreamInfo;

/// Backend for a managed overlay source.
enum ManagedSourceBackend {
    /// A git-backed source that is cloned and pulled.
    Git(OverlayRepoManager),
    /// A local directory source (no cloning needed).
    Local {
        /// Absolute path to the local directory (resolved from repo-relative at construction).
        path: PathBuf,
    },
}

impl ManagedSourceBackend {
    /// Whether this backend needs to be cloned before use.
    #[allow(dead_code)] // Utility method for future use
    fn needs_clone(&self) -> bool {
        match self {
            Self::Git(manager) => manager.needs_clone(),
            Self::Local { .. } => false,
        }
    }

    /// The base path where overlays are stored.
    fn repo_path(&self) -> &Path {
        match self {
            Self::Git(manager) => manager.path(),
            Self::Local { path } => path,
        }
    }
}

/// A managed source wrapping a backend.
struct ManagedSource {
    source: Source,
    backend: ManagedSourceBackend,
}

/// Result of resolving an overlay from sources.
#[derive(Debug)]
pub(crate) struct ResolvedOverlay {
    /// Path to the resolved overlay directory.
    pub(crate) path: PathBuf,
    /// Source from which the overlay was resolved.
    pub(crate) source: Source,
    /// How the overlay was resolved (direct match or upstream fallback).
    pub(crate) resolved_via: ResolvedVia,
    /// Current commit SHA of the source repository.
    pub(crate) commit: String,
}

/// Manager for multiple overlay sources.
///
/// Sources are checked in order during resolution. The first source
/// containing the requested overlay wins.
pub(crate) struct SourceManager {
    sources: Vec<ManagedSource>,
}

/// Cache directory for sources.
fn sources_cache_dir() -> Result<PathBuf> {
    let base = directories::ProjectDirs::from("", "", "repoverlay")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
    Ok(base.cache_dir().join("sources"))
}

impl SourceManager {
    /// Create a new source manager from a list of sources.
    ///
    /// Each git source is configured to clone to a subdirectory within the cache.
    /// Local sources resolve their path relative to `repo_root`.
    pub(crate) fn new(sources: Vec<Source>, repo_root: Option<&Path>) -> Result<Self> {
        let cache_dir = sources_cache_dir()?;
        let managed_sources = sources
            .into_iter()
            .map(|source| {
                let backend = if source.is_local() {
                    let base = repo_root.ok_or_else(|| {
                        anyhow::anyhow!(
                            "Cannot use local source '{}' without a repository context",
                            source.name
                        )
                    })?;
                    let base_canonical = base.canonicalize().with_context(|| {
                        format!("Repository path does not exist: {}", base.display())
                    })?;
                    let source_path = base.join(source.path()?);
                    let canonical_path = source_path.canonicalize().with_context(|| {
                        format!(
                            "Local source '{}' path does not exist: {}",
                            source.name,
                            source_path.display()
                        )
                    })?;
                    if !canonical_path.starts_with(&base_canonical) {
                        anyhow::bail!(
                            "Local source '{}' path escapes repository boundary: {}",
                            source.name,
                            canonical_path.display()
                        );
                    }
                    ManagedSourceBackend::Local {
                        path: canonical_path,
                    }
                } else {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url()?.to_string(),
                        local_path: Some(local_path),
                    };
                    let manager = OverlayRepoManager::new(config)?;
                    ManagedSourceBackend::Git(manager)
                };
                Ok(ManagedSource { source, backend })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            sources: managed_sources,
        })
    }

    /// Get the list of source names in priority order.
    #[must_use]
    pub(crate) fn source_names(&self) -> Vec<&str> {
        self.sources
            .iter()
            .map(|s| s.source.name.as_str())
            .collect()
    }

    /// Get a source by name.
    #[allow(dead_code)] // Utility method for future use
    pub(crate) fn get_source(&self, name: &str) -> Option<&Source> {
        self.sources
            .iter()
            .find(|s| s.source.name == name)
            .map(|s| &s.source)
    }

    /// Ensure all git sources are cloned.
    pub(crate) fn ensure_all_cloned(&self) -> Result<()> {
        for ms in &self.sources {
            if let ManagedSourceBackend::Git(manager) = &ms.backend {
                manager.ensure_cloned()?;
            }
        }
        Ok(())
    }

    /// Pull updates for all git sources.
    pub(crate) fn pull_all(&self) -> Result<()> {
        for ms in &self.sources {
            if let ManagedSourceBackend::Git(manager) = &ms.backend
                && !manager.needs_clone()
            {
                manager.pull()?;
            }
        }
        Ok(())
    }

    /// Resolve an overlay reference against all sources in priority order.
    ///
    /// Returns `None` if no source has the overlay.
    /// If `source_filter` is provided, only that source is checked.
    pub(crate) fn resolve(
        &self,
        org: &str,
        repo: &str,
        name: &str,
        upstream: Option<&UpstreamInfo>,
        source_filter: Option<&str>,
    ) -> Result<Option<ResolvedOverlay>> {
        let sources_to_check: Vec<&ManagedSource> = if let Some(filter_name) = source_filter {
            // Only check the specified source
            let source = self
                .sources
                .iter()
                .find(|s| s.source.name == filter_name)
                .ok_or_else(|| {
                    let available: Vec<_> = self.source_names();
                    anyhow::anyhow!(
                        "Unknown source: {filter_name}\nAvailable sources: {}",
                        available.join(", ")
                    )
                })?;
            vec![source]
        } else {
            self.sources.iter().collect()
        };

        for ms in sources_to_check {
            match &ms.backend {
                ManagedSourceBackend::Git(manager) => {
                    if manager.needs_clone() {
                        continue;
                    }
                    if let Ok((path, resolved_via)) =
                        manager.get_overlay_path_with_fallback(org, repo, name, upstream)
                    {
                        let commit = manager.get_current_commit()?;
                        return Ok(Some(ResolvedOverlay {
                            path,
                            source: ms.source.clone(),
                            resolved_via,
                            commit,
                        }));
                    }
                }
                ManagedSourceBackend::Local { path: base_path } => {
                    if let Some(overlay_path) = get_overlay_path_in_dir(base_path, org, repo, name)
                    {
                        return Ok(Some(ResolvedOverlay {
                            path: overlay_path,
                            source: ms.source.clone(),
                            resolved_via: ResolvedVia::Direct,
                            commit: String::from("local"),
                        }));
                    }
                    if let Some(upstream_info) = upstream
                        && let Some(overlay_path) = get_overlay_path_in_dir(
                            base_path,
                            &upstream_info.org,
                            &upstream_info.repo,
                            name,
                        )
                    {
                        return Ok(Some(ResolvedOverlay {
                            path: overlay_path,
                            source: ms.source.clone(),
                            resolved_via: ResolvedVia::Upstream,
                            commit: String::from("local"),
                        }));
                    }
                }
            }
            // Not found in this source, continue to next
        }

        Ok(None)
    }

    /// Find all sources that have a specific overlay.
    ///
    /// Returns a list of (source, `resolved_via`) pairs for each source
    /// that has the overlay.
    #[must_use]
    #[allow(dead_code)] // Utility method for future `resolve` command
    pub(crate) fn find_all_matches(
        &self,
        org: &str,
        repo: &str,
        name: &str,
        upstream: Option<&UpstreamInfo>,
    ) -> Vec<(Source, ResolvedVia)> {
        let mut matches = Vec::new();

        for ms in &self.sources {
            match &ms.backend {
                ManagedSourceBackend::Git(manager) => {
                    if manager.needs_clone() {
                        continue;
                    }
                    if let Ok((_, resolved_via)) =
                        manager.get_overlay_path_with_fallback(org, repo, name, upstream)
                    {
                        matches.push((ms.source.clone(), resolved_via));
                    }
                }
                ManagedSourceBackend::Local { path: base_path } => {
                    if get_overlay_path_in_dir(base_path, org, repo, name).is_some() {
                        matches.push((ms.source.clone(), ResolvedVia::Direct));
                    } else if let Some(upstream_info) = upstream
                        && get_overlay_path_in_dir(
                            base_path,
                            &upstream_info.org,
                            &upstream_info.repo,
                            name,
                        )
                        .is_some()
                    {
                        matches.push((ms.source.clone(), ResolvedVia::Upstream));
                    }
                }
            }
        }

        matches
    }

    /// List overlay names for a specific org/repo across all sources.
    ///
    /// Returns unique overlay names (deduplicated across sources).
    pub(crate) fn list_overlays_for_repo(
        &self,
        org: &str,
        repo: &str,
    ) -> anyhow::Result<Vec<OverlayName>> {
        let mut names = std::collections::HashSet::new();

        for ms in &self.sources {
            match &ms.backend {
                ManagedSourceBackend::Git(manager) => {
                    if manager.needs_clone() {
                        continue;
                    }
                    if let Ok(overlays) = manager.list_overlays_for_repo(org, repo) {
                        for overlay in overlays {
                            names.insert(OverlayName::try_new(overlay.name)?);
                        }
                    }
                }
                ManagedSourceBackend::Local { path: base_path } => {
                    if let Ok(overlays) = list_overlays_in_dir(base_path) {
                        for overlay in overlays {
                            if overlay.org.eq_ignore_ascii_case(org)
                                && overlay.repo.eq_ignore_ascii_case(repo)
                            {
                                names.insert(OverlayName::try_new(overlay.name)?);
                            }
                        }
                    }
                }
            }
        }

        let mut result: Vec<_> = names.into_iter().collect();
        result.sort();
        Ok(result)
    }

    /// Get the base path for a named source.
    pub(crate) fn get_source_base_path(&self, source_name: &str) -> Option<&Path> {
        self.sources
            .iter()
            .find(|ms| ms.source.name == source_name)
            .map(|ms| ms.backend.repo_path())
    }

    /// Get the current commit for a named source.
    ///
    /// Returns `"local"` for local directory sources.
    pub(crate) fn get_source_commit(&self, source_name: &str) -> Result<String> {
        let ms = self
            .sources
            .iter()
            .find(|ms| ms.source.name == source_name)
            .ok_or_else(|| anyhow::anyhow!("Source not found: {source_name}"))?;
        match &ms.backend {
            ManagedSourceBackend::Git(manager) => manager.get_current_commit(),
            ManagedSourceBackend::Local { .. } => Ok("local".to_string()),
        }
    }

    /// List all overlays across all sources.
    pub(crate) fn list_all_overlays(&self) -> Result<Vec<(Source, AvailableOverlay)>> {
        let mut all = Vec::new();

        for ms in &self.sources {
            match &ms.backend {
                ManagedSourceBackend::Git(manager) => {
                    if manager.needs_clone() {
                        continue;
                    }
                    let overlays = manager.list_overlays()?;
                    for overlay in overlays {
                        all.push((ms.source.clone(), overlay));
                    }
                }
                ManagedSourceBackend::Local { path: base_path } => {
                    let overlays = list_overlays_in_dir(base_path)?;
                    for overlay in overlays {
                        all.push((ms.source.clone(), overlay));
                    }
                }
            }
        }

        Ok(all)
    }
}

/// Check if an overlay exists at `base/org/repo/name`.
fn get_overlay_path_in_dir(base: &Path, org: &str, repo: &str, name: &str) -> Option<PathBuf> {
    if !is_valid_path_component(org)
        || !is_valid_path_component(repo)
        || !is_valid_path_component(name)
    {
        return None;
    }

    let overlay_path = base.join(org).join(repo).join(name);
    let canonical_overlay = overlay_path.canonicalize().ok()?;
    let canonical_base = base.canonicalize().ok()?;
    if canonical_overlay.starts_with(canonical_base) && canonical_overlay.is_dir() {
        Some(overlay_path)
    } else {
        None
    }
}

/// Validate path component safety for org/repo/overlay segments.
fn is_valid_path_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.contains('/')
        && !component.contains('\\')
}

/// The detected layout of a local overlay source directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceLayout {
    /// Standard org/repo/name nesting (3 levels deep).
    Structured,
    /// Flat layout — subdirectories are individual overlays, or the directory itself
    /// is a single overlay.
    Flat,
}

/// Detect whether a directory uses structured (org/repo/name) or flat layout.
///
/// A directory is considered structured if any visible top-level subdirectory
/// contains a visible subdirectory that itself contains a visible subdirectory
/// (i.e., 3 levels of nesting exist). Otherwise it is flat.
pub(crate) fn detect_source_layout(base: &Path) -> Result<SourceLayout> {
    if !base.exists() {
        return Ok(SourceLayout::Flat);
    }

    for top_entry in fs::read_dir(base)? {
        let top_entry = top_entry?;
        if !top_entry.path().is_dir() || top_entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }

        // Check if this top-level dir contains a subdir that contains a subdir
        if let Ok(mid_entries) = fs::read_dir(top_entry.path()) {
            for mid_entry in mid_entries {
                let Ok(mid_entry) = mid_entry else { continue };
                if !mid_entry.path().is_dir()
                    || mid_entry.file_name().to_string_lossy().starts_with('.')
                {
                    continue;
                }

                if let Ok(leaf_entries) = fs::read_dir(mid_entry.path()) {
                    for leaf_entry in leaf_entries {
                        let Ok(leaf_entry) = leaf_entry else { continue };
                        if leaf_entry.path().is_dir()
                            && !leaf_entry.file_name().to_string_lossy().starts_with('.')
                        {
                            return Ok(SourceLayout::Structured);
                        }
                    }
                }
            }
        }
    }

    Ok(SourceLayout::Flat)
}

/// List overlays from a flat (non-nested) directory.
///
/// Each visible subdirectory is treated as a separate overlay. If the directory
/// itself contains no subdirectories but has files, it is treated as a single
/// overlay named after the directory.
fn list_overlays_in_flat_dir(base: &Path) -> Result<Vec<AvailableOverlay>> {
    let mut overlays = Vec::new();
    let mut has_subdirs = false;
    let mut has_files = false;

    for entry in fs::read_dir(base)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            has_subdirs = true;
            let overlay_name = name;
            let has_config = path.join("repoverlay.ccl").exists();

            overlays.push(AvailableOverlay {
                org: String::new(),
                repo: String::new(),
                name: overlay_name,
                has_config,
                flat: true,
            });
        } else {
            has_files = true;
        }
    }

    if !has_subdirs && has_files {
        // The directory itself is a single overlay (has files but no subdirs)
        let dir_name = base.file_name().map_or_else(
            || "overlay".to_string(),
            |n| n.to_string_lossy().to_string(),
        );
        let has_config = base.join("repoverlay.ccl").exists();

        overlays.push(AvailableOverlay {
            org: String::new(),
            repo: String::new(),
            name: dir_name,
            has_config,
            flat: true,
        });
    }

    overlays.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(overlays)
}

/// List all overlays in a directory, auto-detecting layout.
///
/// Detects whether the directory uses structured (org/repo/name) or flat layout
/// and scans accordingly. Structured directories use 3-level nesting; flat
/// directories treat each subdirectory as an overlay (or the directory itself
/// if it contains no subdirectories).
pub(crate) fn list_overlays_in_dir(base: &Path) -> Result<Vec<AvailableOverlay>> {
    if !base.exists() {
        return Ok(Vec::new());
    }

    match detect_source_layout(base)? {
        SourceLayout::Structured => list_overlays_in_structured_dir(base),
        SourceLayout::Flat => list_overlays_in_flat_dir(base),
    }
}

/// List all overlays in a directory using the org/repo/name structure.
///
/// Same scanning logic as `OverlayRepoManager::list_overlays()`.
fn list_overlays_in_structured_dir(base: &Path) -> Result<Vec<AvailableOverlay>> {
    let mut overlays = Vec::new();

    if !base.exists() {
        return Ok(overlays);
    }

    for org_entry in fs::read_dir(base)? {
        let org_entry = org_entry?;
        let org_path = org_entry.path();

        if !org_path.is_dir() || org_entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }

        let org_name = org_entry.file_name().to_string_lossy().to_string();

        for repo_entry in fs::read_dir(&org_path)? {
            let repo_entry = repo_entry?;
            let repo_path = repo_entry.path();

            if !repo_path.is_dir() || repo_entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }

            let repo_name = repo_entry.file_name().to_string_lossy().to_string();

            for overlay_entry in fs::read_dir(&repo_path)? {
                let overlay_entry = overlay_entry?;
                let overlay_path = overlay_entry.path();

                if !overlay_path.is_dir()
                    || overlay_entry.file_name().to_string_lossy().starts_with('.')
                {
                    continue;
                }

                let overlay_name = overlay_entry.file_name().to_string_lossy().to_string();
                let has_config = overlay_path.join("repoverlay.ccl").exists();

                overlays.push(AvailableOverlay {
                    org: org_name.clone(),
                    repo: repo_name.clone(),
                    name: overlay_name,
                    has_config,
                    flat: false,
                });
            }
        }
    }

    overlays.sort_by(|a, b| (&a.org, &a.repo, &a.name).cmp(&(&b.org, &b.repo, &b.name)));

    Ok(overlays)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Helper to create a mock overlay source directory structure.
    fn create_mock_source(dir: &Path, overlays: &[(&str, &str, &str)]) {
        // Initialize as git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();

        // Configure git user for commits
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(dir)
            .output()
            .unwrap();

        // Create overlay directories
        for (org, repo, name) in overlays {
            let overlay_path = dir.join(org).join(repo).join(name);
            fs::create_dir_all(&overlay_path).unwrap();
            // Add a marker file
            fs::write(overlay_path.join("CLAUDE.md"), "# Test overlay").unwrap();
        }

        // Commit the files
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "Initial"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[test]
    fn test_resolve_first_match_wins() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Create two sources, both with the same overlay
        let source1_path = cache_dir.join("personal");
        let source2_path = cache_dir.join("team");
        fs::create_dir_all(&source1_path).unwrap();
        fs::create_dir_all(&source2_path).unwrap();

        create_mock_source(
            &source1_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );
        create_mock_source(
            &source2_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        // Create sources
        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("file://dummy".to_string()), // Not used since already cloned
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        // Create manager with pre-existing clones
        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Resolve should return the first source (personal)
        let result = manager
            .resolve("microsoft", "FluidFramework", "claude-config", None, None)
            .unwrap();

        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.source.name, "personal");
    }

    #[test]
    fn test_resolve_priority_order() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Source 1 does NOT have the overlay
        let source1_path = cache_dir.join("personal");
        fs::create_dir_all(&source1_path).unwrap();
        create_mock_source(&source1_path, &[("other", "repo", "some-overlay")]);

        // Source 2 HAS the overlay
        let source2_path = cache_dir.join("team");
        fs::create_dir_all(&source2_path).unwrap();
        create_mock_source(
            &source2_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Should find in team (second source)
        let result = manager
            .resolve("microsoft", "FluidFramework", "claude-config", None, None)
            .unwrap();

        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.source.name, "team");
    }

    #[test]
    fn test_resolve_not_found_in_any() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Create a source with different overlays
        let source_path = cache_dir.join("personal");
        fs::create_dir_all(&source_path).unwrap();
        create_mock_source(&source_path, &[("other", "repo", "different-overlay")]);

        let sources = vec![Source {
            name: "personal".to_string(),
            url: Some("file://dummy".to_string()),
            path: None,
        }];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Should return None when not found
        let result = manager
            .resolve("microsoft", "FluidFramework", "claude-config", None, None)
            .unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_skips_missing_sources() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Source 1 is NOT cloned (directory doesn't exist)
        // Source 2 IS cloned and has the overlay
        let source2_path = cache_dir.join("team");
        fs::create_dir_all(&source2_path).unwrap();
        create_mock_source(
            &source2_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![
            Source {
                name: "personal".to_string(), // Not cloned
                url: Some("file://dummy".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Should skip personal (not cloned) and find in team
        let result = manager
            .resolve("microsoft", "FluidFramework", "claude-config", None, None)
            .unwrap();

        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.source.name, "team");
    }

    #[test]
    fn test_resolve_with_upstream_fallback() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Source has overlay under the upstream org/repo, not the fork
        let source_path = cache_dir.join("personal");
        fs::create_dir_all(&source_path).unwrap();
        create_mock_source(
            &source_path,
            &[("upstream-org", "upstream-repo", "claude-config")],
        );

        let sources = vec![Source {
            name: "personal".to_string(),
            url: Some("file://dummy".to_string()),
            path: None,
        }];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Request for fork org/repo with upstream fallback
        let upstream = UpstreamInfo {
            org: "upstream-org".to_string(),
            repo: "upstream-repo".to_string(),
            remote_name: "upstream".to_string(),
        };

        let result = manager
            .resolve(
                "fork-org",
                "fork-repo",
                "claude-config",
                Some(&upstream),
                None,
            )
            .unwrap();

        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.resolved_via, ResolvedVia::Upstream);
    }

    #[test]
    fn test_source_filter_uses_specific_source() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Both sources have the overlay
        let source1_path = cache_dir.join("personal");
        let source2_path = cache_dir.join("team");
        fs::create_dir_all(&source1_path).unwrap();
        fs::create_dir_all(&source2_path).unwrap();

        create_mock_source(
            &source1_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );
        create_mock_source(
            &source2_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Explicitly request from team source
        let result = manager
            .resolve(
                "microsoft",
                "FluidFramework",
                "claude-config",
                None,
                Some("team"),
            )
            .unwrap();

        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.source.name, "team");
    }

    #[test]
    fn test_source_filter_unknown_source_error() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        let source_path = cache_dir.join("personal");
        fs::create_dir_all(&source_path).unwrap();
        create_mock_source(
            &source_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![Source {
            name: "personal".to_string(),
            url: Some("file://dummy".to_string()),
            path: None,
        }];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Request with unknown source should error
        let result = manager.resolve(
            "microsoft",
            "FluidFramework",
            "claude-config",
            None,
            Some("unknown-source"),
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Unknown source"));
        assert!(err.to_string().contains("personal"));
    }

    #[test]
    fn test_find_all_matches() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Both sources have the overlay
        let source1_path = cache_dir.join("personal");
        let source2_path = cache_dir.join("team");
        fs::create_dir_all(&source1_path).unwrap();
        fs::create_dir_all(&source2_path).unwrap();

        create_mock_source(
            &source1_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );
        create_mock_source(
            &source2_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Should find in both sources
        let matches =
            manager.find_all_matches("microsoft", "FluidFramework", "claude-config", None);

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].0.name, "personal");
        assert_eq!(matches[1].0.name, "team");
    }

    #[test]
    fn test_list_all_overlays() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        let source1_path = cache_dir.join("personal");
        let source2_path = cache_dir.join("team");
        fs::create_dir_all(&source1_path).unwrap();
        fs::create_dir_all(&source2_path).unwrap();

        create_mock_source(
            &source1_path,
            &[
                ("microsoft", "FluidFramework", "claude-config"),
                ("microsoft", "FluidFramework", "vscode-settings"),
            ],
        );
        create_mock_source(&source2_path, &[("google", "chromium", "dev-setup")]);

        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        let all_overlays = manager.list_all_overlays().unwrap();

        // Should have 3 total overlays
        assert_eq!(all_overlays.len(), 3);

        // Check that overlays from different sources are included
        let personal_count = all_overlays
            .iter()
            .filter(|(s, _)| s.name == "personal")
            .count();
        let team_count = all_overlays
            .iter()
            .filter(|(s, _)| s.name == "team")
            .count();

        assert_eq!(personal_count, 2);
        assert_eq!(team_count, 1);
    }

    #[test]
    fn test_source_names() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        let names = manager.source_names();
        assert_eq!(names, vec!["personal", "team"]);
    }

    #[test]
    fn test_get_source_found() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("https://github.com/user/overlays".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("https://github.com/team/overlays".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        let source = manager.get_source("personal");
        assert!(source.is_some());
        let source = source.unwrap();
        assert_eq!(source.name, "personal");
        assert_eq!(source.url().unwrap(), "https://github.com/user/overlays");
    }

    #[test]
    fn test_get_source_not_found() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        let sources = vec![Source {
            name: "personal".to_string(),
            url: Some("file://dummy".to_string()),
            path: None,
        }];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        let source = manager.get_source("nonexistent");
        assert!(source.is_none());
    }

    #[test]
    fn test_list_overlays_for_repo_with_mixed_sources() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Source 1 is cloned and has overlays
        let source1_path = cache_dir.join("personal");
        fs::create_dir_all(&source1_path).unwrap();
        create_mock_source(
            &source1_path,
            &[
                ("microsoft", "FluidFramework", "claude-config"),
                ("microsoft", "FluidFramework", "vscode-settings"),
            ],
        );

        // Source 2 is NOT cloned (directory doesn't exist)

        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Should find overlays from cloned source, skip uncloned source
        let overlays = manager
            .list_overlays_for_repo("microsoft", "FluidFramework")
            .unwrap();
        assert_eq!(overlays.len(), 2);
        assert!(overlays.contains(&OverlayName::new("claude-config")));
        assert!(overlays.contains(&OverlayName::new("vscode-settings")));
    }

    #[test]
    fn test_list_overlays_for_repo_deduplication() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Both sources have the same overlay
        let source1_path = cache_dir.join("personal");
        let source2_path = cache_dir.join("team");
        fs::create_dir_all(&source1_path).unwrap();
        fs::create_dir_all(&source2_path).unwrap();

        create_mock_source(
            &source1_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );
        create_mock_source(
            &source2_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![
            Source {
                name: "personal".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
            Source {
                name: "team".to_string(),
                url: Some("file://dummy".to_string()),
                path: None,
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Should deduplicate across sources
        let overlays = manager
            .list_overlays_for_repo("microsoft", "FluidFramework")
            .unwrap();
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0], "claude-config");
    }

    #[test]
    fn test_list_overlays_for_repo_no_matches() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        let source_path = cache_dir.join("personal");
        fs::create_dir_all(&source_path).unwrap();
        create_mock_source(
            &source_path,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![Source {
            name: "personal".to_string(),
            url: Some("file://dummy".to_string()),
            path: None,
        }];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url().unwrap().to_string(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        backend: ManagedSourceBackend::Git(
                            OverlayRepoManager::new(config).unwrap(),
                        ),
                    }
                })
                .collect(),
        };

        // Different repo should return empty
        let overlays = manager
            .list_overlays_for_repo("google", "chromium")
            .unwrap();
        assert!(overlays.is_empty());
    }

    /// Test that `sources_cache_dir` returns error when `ProjectDirs` is unavailable.
    /// This catches mutants that would replace the error with `Ok(Default::default())`.
    #[test]
    fn sources_cache_dir_fails_without_project_dirs() {
        // The function should return an error, not Ok(PathBuf::new())
        let result = sources_cache_dir();
        assert!(
            result.is_ok(),
            "sources_cache_dir should work in test environment with valid home dir"
        );
        // Verify it returns a valid path, not an empty default
        let path = result.unwrap();
        assert!(
            !path.as_os_str().is_empty(),
            "sources_cache_dir should not return empty path"
        );
    }

    // ==================== Local directory source tests ====================

    /// Create a local overlay directory (no git init needed).
    fn create_local_source_dir(dir: &Path, overlays: &[(&str, &str, &str)]) {
        for (org, repo, name) in overlays {
            let overlay_path = dir.join(org).join(repo).join(name);
            fs::create_dir_all(&overlay_path).unwrap();
            fs::write(overlay_path.join(".envrc"), "export FOO=bar").unwrap();
        }
    }

    #[test]
    fn test_local_source_resolve_finds_overlay() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        // Create local overlay directory inside the "repo"
        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(
            &overlays_dir,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![Source {
            name: "local-overlays".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        let result = manager
            .resolve("microsoft", "FluidFramework", "claude-config", None, None)
            .unwrap();

        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.source.name, "local-overlays");
        assert_eq!(resolved.commit, "local");
        assert_eq!(resolved.resolved_via, ResolvedVia::Direct);
        assert!(
            resolved
                .path
                .ends_with("microsoft/FluidFramework/claude-config")
        );
    }

    #[test]
    fn test_local_source_resolve_not_found() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(
            &overlays_dir,
            &[("microsoft", "FluidFramework", "claude-config")],
        );

        let sources = vec![Source {
            name: "local-overlays".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        let result = manager
            .resolve("google", "chromium", "dev-setup", None, None)
            .unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_local_source_with_upstream_fallback() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(
            &overlays_dir,
            &[("upstream-org", "upstream-repo", "claude-config")],
        );

        let sources = vec![Source {
            name: "local-overlays".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        let upstream = UpstreamInfo {
            org: "upstream-org".to_string(),
            repo: "upstream-repo".to_string(),
            remote_name: "upstream".to_string(),
        };

        let result = manager
            .resolve(
                "fork-org",
                "fork-repo",
                "claude-config",
                Some(&upstream),
                None,
            )
            .unwrap();

        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.resolved_via, ResolvedVia::Upstream);
        assert_eq!(resolved.commit, "local");
    }

    #[test]
    fn test_mixed_local_and_git_sources() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path();

        // Git source
        let git_source_path = cache_dir.join("git-source");
        fs::create_dir_all(&git_source_path).unwrap();
        create_mock_source(&git_source_path, &[("org", "repo", "git-overlay")]);

        // Local source
        let local_dir = cache_dir.join("local-overlays");
        fs::create_dir_all(&local_dir).unwrap();
        create_local_source_dir(&local_dir, &[("org", "repo", "local-overlay")]);

        let manager = SourceManager {
            sources: vec![
                ManagedSource {
                    source: Source {
                        name: "git-source".to_string(),
                        url: Some("file://dummy".to_string()),
                        path: None,
                    },
                    backend: ManagedSourceBackend::Git(
                        OverlayRepoManager::new(OverlayRepoConfig {
                            url: "file://dummy".to_string(),
                            local_path: Some(git_source_path),
                        })
                        .unwrap(),
                    ),
                },
                ManagedSource {
                    source: Source {
                        name: "local-source".to_string(),
                        url: None,
                        path: Some(PathBuf::from("local-overlays")),
                    },
                    backend: ManagedSourceBackend::Local { path: local_dir },
                },
            ],
        };

        // Git overlay resolves from git source
        let git_result = manager
            .resolve("org", "repo", "git-overlay", None, None)
            .unwrap();
        assert!(git_result.is_some());
        assert_eq!(git_result.unwrap().source.name, "git-source");

        // Local overlay resolves from local source
        let local_result = manager
            .resolve("org", "repo", "local-overlay", None, None)
            .unwrap();
        assert!(local_result.is_some());
        let resolved = local_result.unwrap();
        assert_eq!(resolved.source.name, "local-source");
        assert_eq!(resolved.commit, "local");
    }

    #[test]
    fn test_list_overlays_in_dir_local() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        create_local_source_dir(
            base,
            &[
                ("org-a", "repo-1", "overlay-x"),
                ("org-a", "repo-1", "overlay-y"),
                ("org-b", "repo-2", "overlay-z"),
            ],
        );

        let overlays = list_overlays_in_dir(base).unwrap();

        assert_eq!(overlays.len(), 3);

        let names: Vec<String> = overlays
            .iter()
            .map(|o| format!("{}/{}/{}", o.org, o.repo, o.name))
            .collect();
        assert!(names.contains(&"org-a/repo-1/overlay-x".to_string()));
        assert!(names.contains(&"org-a/repo-1/overlay-y".to_string()));
        assert!(names.contains(&"org-b/repo-2/overlay-z".to_string()));
    }

    #[test]
    fn test_list_overlays_in_dir_empty() {
        let temp = TempDir::new().unwrap();
        let overlays = list_overlays_in_dir(temp.path()).unwrap();
        assert!(overlays.is_empty());
    }

    #[test]
    fn test_list_overlays_in_dir_nonexistent() {
        let result = list_overlays_in_dir(Path::new("/nonexistent/path"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_list_overlays_in_dir_skips_hidden() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create visible overlay
        create_local_source_dir(base, &[("org", "repo", "visible")]);

        // Create hidden directories at each level
        let hidden_org = base.join(".hidden-org").join("repo").join("overlay");
        fs::create_dir_all(&hidden_org).unwrap();

        let hidden_repo = base.join("org").join(".hidden-repo").join("overlay");
        fs::create_dir_all(&hidden_repo).unwrap();

        let hidden_overlay = base.join("org").join("repo").join(".hidden-overlay");
        fs::create_dir_all(&hidden_overlay).unwrap();

        let overlays = list_overlays_in_dir(base).unwrap();
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].name, "visible");
    }

    #[test]
    fn test_detect_structured_layout() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create org/repo/name structure
        create_local_source_dir(base, &[("org", "repo", "overlay")]);

        let layout = detect_source_layout(base).unwrap();
        assert_eq!(layout, SourceLayout::Structured);
    }

    #[test]
    fn test_detect_flat_layout_with_subdirs() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create flat subdirectories (each is an overlay)
        let overlay_a = base.join("config-a");
        fs::create_dir_all(&overlay_a).unwrap();
        fs::write(overlay_a.join(".envrc"), "export FOO=bar").unwrap();

        let overlay_b = base.join("config-b");
        fs::create_dir_all(&overlay_b).unwrap();
        fs::write(overlay_b.join(".editorconfig"), "[*]\nindent = 4").unwrap();

        let layout = detect_source_layout(base).unwrap();
        assert_eq!(layout, SourceLayout::Flat);
    }

    #[test]
    fn test_detect_flat_layout_with_files_only() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Directory with only files (no subdirs) → flat
        fs::write(base.join(".envrc"), "export FOO=bar").unwrap();
        fs::write(base.join("repoverlay.ccl"), "").unwrap();

        let layout = detect_source_layout(base).unwrap();
        assert_eq!(layout, SourceLayout::Flat);
    }

    #[test]
    fn test_detect_flat_layout_empty() {
        let temp = TempDir::new().unwrap();
        let layout = detect_source_layout(temp.path()).unwrap();
        assert_eq!(layout, SourceLayout::Flat);
    }

    #[test]
    fn test_detect_flat_layout_nonexistent() {
        let layout = detect_source_layout(Path::new("/nonexistent/path")).unwrap();
        assert_eq!(layout, SourceLayout::Flat);
    }

    #[test]
    fn test_list_flat_dir_with_subdirs() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create flat overlay subdirectories
        let overlay_a = base.join("config-a");
        fs::create_dir_all(&overlay_a).unwrap();
        fs::write(overlay_a.join(".envrc"), "export FOO=bar").unwrap();

        let overlay_b = base.join("config-b");
        fs::create_dir_all(&overlay_b).unwrap();
        fs::write(overlay_b.join("repoverlay.ccl"), "").unwrap();

        let overlays = list_overlays_in_dir(base).unwrap();
        assert_eq!(overlays.len(), 2);

        assert!(overlays.iter().all(|o| o.flat));
        assert!(overlays.iter().all(|o| o.org.is_empty()));
        assert!(overlays.iter().all(|o| o.repo.is_empty()));

        let names: Vec<&str> = overlays.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&"config-a"));
        assert!(names.contains(&"config-b"));

        // config-b has repoverlay.ccl
        let config_b = overlays.iter().find(|o| o.name == "config-b").unwrap();
        assert!(config_b.has_config);
        let config_a = overlays.iter().find(|o| o.name == "config-a").unwrap();
        assert!(!config_a.has_config);
    }

    #[test]
    fn test_list_flat_dir_single_overlay() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Directory with files but no subdirs → single overlay
        fs::write(base.join(".envrc"), "export FOO=bar").unwrap();
        fs::write(base.join("repoverlay.ccl"), "").unwrap();

        let overlays = list_overlays_in_dir(base).unwrap();
        assert_eq!(overlays.len(), 1);

        let overlay = &overlays[0];
        assert!(overlay.flat);
        assert!(overlay.has_config);
        assert!(overlay.org.is_empty());
        assert!(overlay.repo.is_empty());
        // Name is derived from the temp directory name
        assert!(!overlay.name.is_empty());
    }

    #[test]
    fn test_list_flat_dir_skips_hidden() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Visible overlay
        let visible = base.join("visible");
        fs::create_dir_all(&visible).unwrap();
        fs::write(visible.join(".envrc"), "").unwrap();

        // Hidden overlay (should be skipped)
        let hidden = base.join(".hidden");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join(".envrc"), "").unwrap();

        let overlays = list_overlays_in_dir(base).unwrap();
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].name, "visible");
    }

    #[test]
    fn test_flat_overlay_relative_path() {
        use crate::overlay_repo::AvailableOverlay;

        let flat = AvailableOverlay {
            org: String::new(),
            repo: String::new(),
            name: "my-config".to_string(),
            has_config: true,
            flat: true,
        };
        assert_eq!(flat.relative_path(), Path::new("my-config"));

        let structured = AvailableOverlay {
            org: "org".to_string(),
            repo: "repo".to_string(),
            name: "overlay".to_string(),
            has_config: false,
            flat: false,
        };
        assert_eq!(structured.relative_path(), Path::new("org/repo/overlay"));
    }

    #[test]
    fn test_flat_overlay_display() {
        use crate::overlay_repo::AvailableOverlay;

        let flat = AvailableOverlay {
            org: String::new(),
            repo: String::new(),
            name: "my-config".to_string(),
            has_config: true,
            flat: true,
        };
        assert_eq!(flat.to_string(), "my-config");

        let structured = AvailableOverlay {
            org: "org".to_string(),
            repo: "repo".to_string(),
            name: "overlay".to_string(),
            has_config: false,
            flat: false,
        };
        assert_eq!(structured.to_string(), "org/repo/overlay");
    }

    #[test]
    fn test_structured_dir_still_works() {
        // Ensure structured directories continue to work exactly as before
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        create_local_source_dir(
            base,
            &[
                ("org-a", "repo-1", "overlay-x"),
                ("org-b", "repo-2", "overlay-z"),
            ],
        );

        let overlays = list_overlays_in_dir(base).unwrap();
        assert_eq!(overlays.len(), 2);
        assert!(overlays.iter().all(|o| !o.flat));

        let names: Vec<String> = overlays
            .iter()
            .map(|o| format!("{}/{}/{}", o.org, o.repo, o.name))
            .collect();
        assert!(names.contains(&"org-a/repo-1/overlay-x".to_string()));
        assert!(names.contains(&"org-b/repo-2/overlay-z".to_string()));
    }

    #[test]
    fn test_local_source_ensure_cloned_skipped() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(&overlays_dir, &[("org", "repo", "overlay")]);

        let sources = vec![Source {
            name: "local".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        // ensure_all_cloned should not error on local sources
        assert!(manager.ensure_all_cloned().is_ok());
    }

    #[test]
    fn test_local_source_pull_all_skipped() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(&overlays_dir, &[("org", "repo", "overlay")]);

        let sources = vec![Source {
            name: "local".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        // pull_all should not error on local sources
        assert!(manager.pull_all().is_ok());
    }

    #[test]
    fn test_local_source_get_source_commit_returns_local() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(&overlays_dir, &[("org", "repo", "overlay")]);

        let sources = vec![Source {
            name: "local".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        let commit = manager.get_source_commit("local").unwrap();
        assert_eq!(commit, "local");
    }

    #[test]
    fn test_local_source_needs_clone_false() {
        let temp = TempDir::new().unwrap();
        let local_backend = ManagedSourceBackend::Local {
            path: temp.path().to_path_buf(),
        };
        assert!(!local_backend.needs_clone());
    }

    #[test]
    fn test_local_source_repo_path() {
        let temp = TempDir::new().unwrap();
        let local_backend = ManagedSourceBackend::Local {
            path: temp.path().to_path_buf(),
        };
        assert_eq!(local_backend.repo_path(), temp.path());
    }

    #[test]
    fn test_local_source_list_all_overlays() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(
            &overlays_dir,
            &[("org", "repo", "overlay-a"), ("org", "repo", "overlay-b")],
        );

        let sources = vec![Source {
            name: "local".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        let all = manager.list_all_overlays().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|(s, _)| s.name == "local"));

        let names: Vec<&str> = all.iter().map(|(_, o)| o.name.as_str()).collect();
        assert!(names.contains(&"overlay-a"));
        assert!(names.contains(&"overlay-b"));
    }

    #[test]
    fn test_local_source_list_overlays_for_repo() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(
            &overlays_dir,
            &[
                ("microsoft", "FluidFramework", "claude-config"),
                ("microsoft", "FluidFramework", "vscode-settings"),
                ("google", "chromium", "dev-setup"),
            ],
        );

        let sources = vec![Source {
            name: "local".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        let overlays = manager
            .list_overlays_for_repo("microsoft", "FluidFramework")
            .unwrap();
        assert_eq!(overlays.len(), 2);
        assert!(overlays.contains(&OverlayName::new("claude-config")));
        assert!(overlays.contains(&OverlayName::new("vscode-settings")));
    }

    #[test]
    fn test_local_source_nonexistent_path_errors() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let sources = vec![Source {
            name: "missing".to_string(),
            url: None,
            path: Some(PathBuf::from("nonexistent-dir")),
        }];

        let result = SourceManager::new(sources, Some(repo_root));
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn test_local_source_path_escape_rejected() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path();
        let repo_root = workspace.join("repo");
        let outside = workspace.join("outside");
        fs::create_dir_all(&repo_root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let sources = vec![Source {
            name: "escape".to_string(),
            url: None,
            path: Some(PathBuf::from("../outside")),
        }];

        let result = SourceManager::new(sources, Some(&repo_root));
        assert!(result.is_err());
    }

    #[test]
    fn test_local_source_absolute_path_rejected() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path();
        let repo_root = workspace.join("repo");
        let outside = workspace.join("outside");
        fs::create_dir_all(&repo_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let outside_abs = outside.canonicalize().unwrap();

        let sources = vec![Source {
            name: "absolute".to_string(),
            url: None,
            path: Some(outside_abs),
        }];

        let result = SourceManager::new(sources, Some(&repo_root));
        assert!(result.is_err());
    }

    #[test]
    fn test_get_overlay_path_in_dir_rejects_traversal_components() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path();
        let base = workspace.join("base");
        fs::create_dir_all(&base).unwrap();

        fs::create_dir_all(workspace.join("outside/evil")).unwrap();
        fs::create_dir_all(base.join("repo/overlay")).unwrap();

        assert!(get_overlay_path_in_dir(&base, "..", "outside", "evil").is_none());
        assert!(get_overlay_path_in_dir(&base, ".", "repo", "overlay").is_none());
        assert!(get_overlay_path_in_dir(&base, "org/repo", "x", "y").is_none());
    }

    #[test]
    fn test_local_source_no_repo_root_errors() {
        let sources = vec![Source {
            name: "local".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let result = SourceManager::new(sources, None);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("repository context"));
    }

    #[test]
    fn test_local_source_find_all_matches() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        let overlays_dir = repo_root.join("my-overlays");
        fs::create_dir_all(&overlays_dir).unwrap();
        create_local_source_dir(&overlays_dir, &[("org", "repo", "overlay")]);

        let sources = vec![Source {
            name: "local".to_string(),
            url: None,
            path: Some(PathBuf::from("my-overlays")),
        }];

        let manager = SourceManager::new(sources, Some(repo_root)).unwrap();

        let matches = manager.find_all_matches("org", "repo", "overlay", None);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0.name, "local");
        assert_eq!(matches[0].1, ResolvedVia::Direct);
    }
}
