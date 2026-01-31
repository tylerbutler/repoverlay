//! Multi-source overlay resolution.
//!
//! Manages multiple overlay sources with priority-based resolution.
//! Sources are checked in order; first match wins.

use anyhow::Result;
use std::path::PathBuf;

use crate::config::{OverlayRepoConfig, Source};
use crate::overlay_repo::{AvailableOverlay, OverlayRepoManager};
use crate::state::ResolvedVia;
use crate::upstream::UpstreamInfo;

/// A managed source wrapping an `OverlayRepoManager`.
struct ManagedSource {
    source: Source,
    manager: OverlayRepoManager,
}

/// Result of resolving an overlay from sources.
#[derive(Debug)]
pub struct ResolvedOverlay {
    /// Path to the resolved overlay directory.
    pub path: PathBuf,
    /// Source from which the overlay was resolved.
    pub source: Source,
    /// How the overlay was resolved (direct match or upstream fallback).
    pub resolved_via: ResolvedVia,
    /// Current commit SHA of the source repository.
    pub commit: String,
}

/// Manager for multiple overlay sources.
///
/// Sources are checked in order during resolution. The first source
/// containing the requested overlay wins.
pub struct SourceManager {
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
    /// Each source is configured to clone to a subdirectory within the cache.
    pub fn new(sources: Vec<Source>) -> Result<Self> {
        let cache_dir = sources_cache_dir()?;
        let managed_sources = sources
            .into_iter()
            .map(|source| {
                let local_path = cache_dir.join(&source.name);
                let config = OverlayRepoConfig {
                    url: source.url.clone(),
                    local_path: Some(local_path),
                };
                let manager = OverlayRepoManager::new(config)?;
                Ok(ManagedSource { source, manager })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            sources: managed_sources,
        })
    }

    /// Get the list of source names in priority order.
    #[must_use]
    pub fn source_names(&self) -> Vec<&str> {
        self.sources
            .iter()
            .map(|s| s.source.name.as_str())
            .collect()
    }

    /// Get a source by name.
    #[allow(dead_code)] // Utility method for future use
    pub fn get_source(&self, name: &str) -> Option<&Source> {
        self.sources
            .iter()
            .find(|s| s.source.name == name)
            .map(|s| &s.source)
    }

    /// Ensure all sources are cloned.
    pub fn ensure_all_cloned(&self) -> Result<()> {
        for ms in &self.sources {
            ms.manager.ensure_cloned()?;
        }
        Ok(())
    }

    /// Pull updates for all sources.
    pub fn pull_all(&self) -> Result<()> {
        for ms in &self.sources {
            if !ms.manager.needs_clone() {
                ms.manager.pull()?;
            }
        }
        Ok(())
    }

    /// Resolve an overlay reference against all sources in priority order.
    ///
    /// Returns `None` if no source has the overlay.
    /// If `source_filter` is provided, only that source is checked.
    pub fn resolve(
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
            // Skip sources that aren't cloned yet
            if ms.manager.needs_clone() {
                continue;
            }

            // Try to resolve from this source
            if let Ok((path, resolved_via)) = ms
                .manager
                .get_overlay_path_with_fallback(org, repo, name, upstream)
            {
                let commit = ms.manager.get_current_commit()?;
                return Ok(Some(ResolvedOverlay {
                    path,
                    source: ms.source.clone(),
                    resolved_via,
                    commit,
                }));
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
    pub fn find_all_matches(
        &self,
        org: &str,
        repo: &str,
        name: &str,
        upstream: Option<&UpstreamInfo>,
    ) -> Vec<(Source, ResolvedVia)> {
        let mut matches = Vec::new();

        for ms in &self.sources {
            // Skip sources that aren't cloned
            if ms.manager.needs_clone() {
                continue;
            }

            if let Ok((_, resolved_via)) = ms
                .manager
                .get_overlay_path_with_fallback(org, repo, name, upstream)
            {
                matches.push((ms.source.clone(), resolved_via));
            }
        }

        matches
    }

    /// List all overlays across all sources.
    #[allow(dead_code)] // Utility method for future multi-source `list` command
    pub fn list_all_overlays(&self) -> Result<Vec<(Source, AvailableOverlay)>> {
        let mut all = Vec::new();

        for ms in &self.sources {
            // Skip sources that aren't cloned
            if ms.manager.needs_clone() {
                continue;
            }

            let overlays = ms.manager.list_overlays()?;
            for overlay in overlays {
                all.push((ms.source.clone(), overlay));
            }
        }

        Ok(all)
    }
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
                url: "file://dummy".to_string(), // Not used since already cloned
            },
            Source {
                name: "team".to_string(),
                url: "file://dummy".to_string(),
            },
        ];

        // Create manager with pre-existing clones
        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
                url: "file://dummy".to_string(),
            },
            Source {
                name: "team".to_string(),
                url: "file://dummy".to_string(),
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
            url: "file://dummy".to_string(),
        }];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
                url: "file://dummy".to_string(),
            },
            Source {
                name: "team".to_string(),
                url: "file://dummy".to_string(),
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
            url: "file://dummy".to_string(),
        }];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
                url: "file://dummy".to_string(),
            },
            Source {
                name: "team".to_string(),
                url: "file://dummy".to_string(),
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
            url: "file://dummy".to_string(),
        }];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
                url: "file://dummy".to_string(),
            },
            Source {
                name: "team".to_string(),
                url: "file://dummy".to_string(),
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
                url: "file://dummy".to_string(),
            },
            Source {
                name: "team".to_string(),
                url: "file://dummy".to_string(),
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
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
                url: "file://dummy".to_string(),
            },
            Source {
                name: "team".to_string(),
                url: "file://dummy".to_string(),
            },
        ];

        let manager = SourceManager {
            sources: sources
                .into_iter()
                .map(|source| {
                    let local_path = cache_dir.join(&source.name);
                    let config = OverlayRepoConfig {
                        url: source.url.clone(),
                        local_path: Some(local_path),
                    };
                    ManagedSource {
                        source,
                        manager: OverlayRepoManager::new(config).unwrap(),
                    }
                })
                .collect(),
        };

        let names = manager.source_names();
        assert_eq!(names, vec!["personal", "team"]);
    }
}
