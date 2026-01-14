//! Overlay repository management for repoverlay.
//!
//! Handles cloning, updating, and managing a shared overlay repository.
//! The overlay repository stores overlays organized by target repository:
//! `<org>/<repo>/<overlay-name>/`

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::OverlayRepoConfig;

/// Default subdirectory name for the overlay repo clone.
const OVERLAY_REPO_DIR: &str = "overlay-repo";

/// Metadata file name for the overlay repo.
const OVERLAY_REPO_META: &str = ".repoverlay-overlay-repo-meta.ccl";

/// Metadata about the overlay repository clone.
#[derive(Debug, Deserialize, Serialize)]
pub struct OverlayRepoMeta {
    /// The clone URL
    pub clone_url: String,
    /// When the repo was last fetched
    pub last_fetched: DateTime<Utc>,
    /// The current commit SHA
    pub commit: String,
}

/// Information about an available overlay in the repository.
#[derive(Debug, Clone)]
pub struct AvailableOverlay {
    /// Target organization (e.g., "microsoft")
    pub org: String,
    /// Target repository (e.g., "FluidFramework")
    pub repo: String,
    /// Overlay name (e.g., "claude-config")
    pub name: String,
    /// Whether the overlay has a repoverlay.ccl config file
    pub has_config: bool,
}

/// Manager for the overlay repository.
pub struct OverlayRepoManager {
    /// Path to the cloned overlay repository
    repo_path: PathBuf,
    /// Configuration for the overlay repo
    config: OverlayRepoConfig,
}

impl OverlayRepoManager {
    /// Create a new overlay repository manager.
    pub fn new(config: OverlayRepoConfig) -> Result<Self> {
        let repo_path = match &config.local_path {
            Some(path) => path.clone(),
            None => default_overlay_repo_path()?,
        };

        Ok(Self { repo_path, config })
    }

    /// Check if the overlay repository needs to be cloned.
    pub fn needs_clone(&self) -> bool {
        !self.repo_path.exists() || !self.repo_path.join(".git").exists()
    }

    /// Ensure the overlay repo is cloned.
    pub fn ensure_cloned(&self) -> Result<()> {
        if self.needs_clone() {
            self.clone_repo()?;
        }
        Ok(())
    }

    /// Clone the overlay repository.
    fn clone_repo(&self) -> Result<()> {
        // Create parent directories
        if let Some(parent) = self.repo_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let output = Command::new("git")
            .args(["clone", "--depth", "1", &self.config.url])
            .arg(&self.repo_path)
            .output()
            .context("Failed to execute git clone")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not found") || stderr.contains("Repository not found") {
                bail!("Overlay repository not found: {}", self.config.url);
            }
            bail!("Failed to clone overlay repository: {}", stderr.trim());
        }

        self.save_meta()?;
        Ok(())
    }

    /// Pull latest changes from the remote.
    pub fn pull(&self) -> Result<()> {
        if !self.repo_path.exists() {
            bail!("Overlay repository not cloned. Run 'repoverlay init-repo' first.");
        }

        let output = Command::new("git")
            .args(["pull", "--ff-only"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute git pull")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to pull overlay repository: {}", stderr.trim());
        }

        self.save_meta()?;
        Ok(())
    }

    /// Get the current commit SHA.
    pub fn get_current_commit(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute git rev-parse")?;

        if !output.status.success() {
            bail!("Failed to get current commit");
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    /// Save metadata about the overlay repo.
    fn save_meta(&self) -> Result<()> {
        let commit = self.get_current_commit()?;
        let meta = OverlayRepoMeta {
            clone_url: self.config.url.clone(),
            last_fetched: Utc::now(),
            commit,
        };

        let meta_path = self.repo_path.join(OVERLAY_REPO_META);
        fs::write(&meta_path, sickle::to_string(&meta)?)?;

        Ok(())
    }

    /// List all available overlays in the repository.
    pub fn list_overlays(&self) -> Result<Vec<AvailableOverlay>> {
        if !self.repo_path.exists() {
            bail!("Overlay repository not cloned. Run 'repoverlay init-repo' first.");
        }

        let mut overlays = Vec::new();

        // Walk the directory structure: org/repo/overlay-name/
        for org_entry in fs::read_dir(&self.repo_path)? {
            let org_entry = org_entry?;
            let org_path = org_entry.path();

            // Skip non-directories and hidden files
            if !org_path.is_dir() || org_entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }

            let org_name = org_entry.file_name().to_string_lossy().to_string();

            for repo_entry in fs::read_dir(&org_path)? {
                let repo_entry = repo_entry?;
                let repo_path = repo_entry.path();

                if !repo_path.is_dir() || repo_entry.file_name().to_string_lossy().starts_with('.')
                {
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

                    // Check if it has a config file
                    let has_config = overlay_path.join("repoverlay.ccl").exists();

                    overlays.push(AvailableOverlay {
                        org: org_name.clone(),
                        repo: repo_name.clone(),
                        name: overlay_name,
                        has_config,
                    });
                }
            }
        }

        // Sort by org/repo/name
        overlays.sort_by(|a, b| (&a.org, &a.repo, &a.name).cmp(&(&b.org, &b.repo, &b.name)));

        Ok(overlays)
    }

    /// List overlays for a specific target repository.
    pub fn list_overlays_for_repo(&self, org: &str, repo: &str) -> Result<Vec<AvailableOverlay>> {
        let all = self.list_overlays()?;
        Ok(all
            .into_iter()
            .filter(|o| o.org.eq_ignore_ascii_case(org) && o.repo.eq_ignore_ascii_case(repo))
            .collect())
    }

    /// Get the path to a specific overlay.
    pub fn get_overlay_path(&self, org: &str, repo: &str, name: &str) -> Result<PathBuf> {
        let path = self.repo_path.join(org).join(repo).join(name);

        if !path.exists() {
            bail!("Overlay not found: {}/{}/{}", org, repo, name);
        }

        Ok(path)
    }

    /// Stage an overlay for publishing.
    ///
    /// Copies files from source_dir to the overlay repo at org/repo/name/
    /// Returns the destination path.
    pub fn stage_overlay(
        &self,
        org: &str,
        repo: &str,
        name: &str,
        source_dir: &Path,
    ) -> Result<PathBuf> {
        let dest_path = self.repo_path.join(org).join(repo).join(name);

        // Create destination directory
        fs::create_dir_all(&dest_path)?;

        // Copy all files from source to destination
        copy_dir_recursive(source_dir, &dest_path)?;

        // Stage the changes
        let output = Command::new("git")
            .args(["add", "."])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute git add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to stage changes: {}", stderr.trim());
        }

        Ok(dest_path)
    }

    /// Check if there are staged changes.
    pub fn has_staged_changes(&self) -> Result<bool> {
        let output = Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute git diff")?;

        // Exit code 0 means no changes, 1 means changes
        Ok(!output.status.success())
    }

    /// Commit staged changes.
    pub fn commit(&self, message: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute git commit")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // "nothing to commit" is not an error
            if !stderr.contains("nothing to commit") {
                bail!("Failed to commit: {}", stderr.trim());
            }
        }

        Ok(())
    }

    /// Push to remote.
    pub fn push(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["push"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute git push")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to push: {}", stderr.trim());
        }

        Ok(())
    }
}

/// Get the default path for the overlay repository clone.
pub fn default_overlay_repo_path() -> Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("", "", "repoverlay")
        .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;

    Ok(proj_dirs.data_dir().join(OVERLAY_REPO_DIR))
}

/// Copy a directory recursively.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() {
        bail!("Source is not a directory: {}", src.display());
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            // Skip .git directory
            if entry.file_name() == ".git" {
                continue;
            }
            fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Parse an overlay reference in the format "org/repo/name".
pub fn parse_overlay_reference(s: &str) -> Option<(String, String, String)> {
    // Must have exactly 3 parts separated by /
    let parts: Vec<_> = s.split('/').collect();
    if parts.len() != 3 {
        return None;
    }

    // Must not look like a path or URL
    if s.starts_with('.') || s.starts_with('/') || s.contains("://") {
        return None;
    }

    // Each part must be non-empty
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }

    Some((
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_overlay_reference_valid() {
        let result = parse_overlay_reference("microsoft/FluidFramework/claude-config");
        assert!(result.is_some());
        let (org, repo, name) = result.unwrap();
        assert_eq!(org, "microsoft");
        assert_eq!(repo, "FluidFramework");
        assert_eq!(name, "claude-config");
    }

    #[test]
    fn test_parse_overlay_reference_invalid_path() {
        assert!(parse_overlay_reference("./local/path").is_none());
        assert!(parse_overlay_reference("/absolute/path/here").is_none());
    }

    #[test]
    fn test_parse_overlay_reference_invalid_url() {
        assert!(parse_overlay_reference("https://github.com/owner/repo").is_none());
    }

    #[test]
    fn test_parse_overlay_reference_wrong_parts() {
        assert!(parse_overlay_reference("org/repo").is_none());
        assert!(parse_overlay_reference("org/repo/name/extra").is_none());
        assert!(parse_overlay_reference("single").is_none());
    }

    #[test]
    fn test_parse_overlay_reference_empty_parts() {
        assert!(parse_overlay_reference("org//name").is_none());
        assert!(parse_overlay_reference("/repo/name").is_none());
        assert!(parse_overlay_reference("org/repo/").is_none());
    }

    #[test]
    fn test_default_overlay_repo_path() {
        let path = default_overlay_repo_path();
        assert!(path.is_ok());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains("repoverlay"));
        assert!(path.ends_with("overlay-repo"));
    }
}
