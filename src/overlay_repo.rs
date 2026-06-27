//! Overlay repository management for repoverlay.
//!
//! Handles cloning, updating, and managing a shared overlay repository.
//! The overlay repository stores overlays organized by target repository:
//! `<org>/<repo>/<overlay-name>/`

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::OverlayRepoConfig;
use crate::state::ResolvedVia;
use crate::upstream::UpstreamInfo;

/// Default subdirectory name for the overlay repo clone.
const OVERLAY_REPO_DIR: &str = "overlay-repo";

/// Validate that a path component (org, repo, or overlay name) does not contain
/// path traversal characters that could escape the overlay repository directory.
///
/// Reserved namespaces (e.g. `@global`, `@library`) are rejected so they cannot
/// be addressed as literal `org`/`repo`/`name` segments.
fn validate_path_component(s: &str, label: &str) -> Result<()> {
    if s.is_empty() {
        bail!("Invalid {label}: must not be empty");
    }
    if s == "." || s == ".." || s.contains('/') || s.contains('\\') {
        bail!("Invalid {label}: '{s}' contains path traversal characters");
    }
    if crate::library::is_reserved_namespace(s) {
        bail!("Invalid {label}: '{s}' is a reserved namespace");
    }
    Ok(())
}

/// Metadata file name for the overlay repo.
const OVERLAY_REPO_META: &str = ".repoverlay-overlay-repo-meta.ccl";

/// Metadata about the overlay repository clone.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OverlayRepoMeta {
    /// The clone URL
    pub(crate) clone_url: String,
    /// When the repo was last fetched
    pub(crate) last_fetched: DateTime<Utc>,
    /// The current commit SHA
    pub(crate) commit: String,
}

/// Information about an available overlay in the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableOverlay {
    /// Target organization (e.g., "microsoft")
    pub(crate) org: String,
    /// Target repository (e.g., `FluidFramework`)
    pub(crate) repo: String,
    /// Overlay name (e.g., "claude-config")
    pub(crate) name: String,
    /// Whether the overlay has a repoverlay.ccl config file
    pub(crate) has_config: bool,
    /// Whether this overlay comes from a flat (non-nested) source layout.
    ///
    /// Flat overlays use a simpler directory structure without org/repo nesting.
    flat: bool,
    /// Whether this is a global overlay (lives in the `@global/` namespace and
    /// applies to any repository).
    global: bool,
    /// Source-relative path to the overlay directory.
    source_relative_path: PathBuf,
}

impl std::fmt::Display for AvailableOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_global() {
            write!(f, "*/{}", self.name)
        } else if self.is_flat() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}/{}/{}", self.org, self.repo, self.name)
        }
    }
}

impl AvailableOverlay {
    /// Format the overlay path for display with the overlay name in bold.
    pub(crate) fn display_bold(&self) -> String {
        use colored::Colorize;
        if self.is_global() {
            format!("{}/{}", "*".yellow(), self.name.bold())
        } else if self.is_flat() {
            self.name.bold().to_string()
        } else {
            format!("{}/{}/{}", self.org, self.repo, self.name.bold())
        }
    }

    /// Create metadata for an overlay in structured `org/repo/name` layout.
    pub(crate) fn structured(org: String, repo: String, name: String, has_config: bool) -> Self {
        let source_relative_path = PathBuf::from(&org).join(&repo).join(&name);
        Self {
            org,
            repo,
            name,
            has_config,
            flat: false,
            global: false,
            source_relative_path,
        }
    }

    /// Create metadata for an overlay in flat source layout.
    pub(crate) const fn flat(
        name: String,
        source_relative_path: PathBuf,
        has_config: bool,
    ) -> Self {
        Self {
            org: String::new(),
            repo: String::new(),
            name,
            has_config,
            flat: true,
            global: false,
            source_relative_path,
        }
    }

    /// Create metadata for a synthetic flat overlay namespace, such as the in-repo library.
    pub(crate) const fn synthetic_flat(
        org: String,
        name: String,
        source_relative_path: PathBuf,
        has_config: bool,
    ) -> Self {
        Self {
            org,
            repo: String::new(),
            name,
            has_config,
            flat: true,
            global: false,
            source_relative_path,
        }
    }

    /// Create metadata for a global overlay (in the `@global/` namespace).
    ///
    /// The `org` is set to the reserved [`GLOBAL_NAMESPACE`][crate::library::GLOBAL_NAMESPACE]
    /// so callers can distinguish global overlays; `repo` is empty.
    pub(crate) fn global(name: String, has_config: bool) -> Self {
        let source_relative_path = PathBuf::from(crate::library::GLOBAL_NAMESPACE).join(&name);
        Self {
            org: crate::library::GLOBAL_NAMESPACE.to_string(),
            repo: String::new(),
            name,
            has_config,
            flat: false,
            global: true,
            source_relative_path,
        }
    }

    /// Whether this overlay comes from a flat source layout.
    pub(crate) const fn is_flat(&self) -> bool {
        self.flat
    }

    /// Whether this is a global overlay (applies to any repository).
    pub(crate) const fn is_global(&self) -> bool {
        self.global
    }

    /// Returns the relative path from the source base directory to this overlay.
    ///
    /// For structured overlays: `org/repo/name`
    /// For flat overlays: just `name` (or empty if the base dir itself is the overlay)
    pub(crate) fn source_relative_path(&self) -> PathBuf {
        self.source_relative_path.clone()
    }
}

/// Scan the reserved `@global/` namespace in a source base directory.
///
/// Each immediate visible subdirectory of `<base>/@global/` that contains at
/// least one file is returned as a global [`AvailableOverlay`]. Returns an empty
/// vector when the `@global` directory is absent.
pub(crate) fn scan_global_overlays(base: &Path) -> Result<Vec<AvailableOverlay>> {
    let global_dir = base.join(crate::library::GLOBAL_NAMESPACE);
    if !global_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut overlays = Vec::new();
    for entry in
        fs::read_dir(&global_dir).with_context(|| format!("reading {}", global_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') {
            continue;
        }
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading file type for {}", path.display()))?;
        if !file_type.is_dir() || !crate::sources::contains_any_file(&path)? {
            continue;
        }

        let has_config = path.join("repoverlay.ccl").exists();
        overlays.push(AvailableOverlay::global(name, has_config));
    }

    overlays.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(overlays)
}

/// Wraps an [`AvailableOverlay`] with context about which overlays are already applied,
/// enabling conversion to a [`SelectableItem`][crate::selection::SelectableItem] for the
/// browse UI.
pub(crate) struct BrowseOverlayItem<'a> {
    /// The available overlay to display.
    pub(crate) overlay: &'a AvailableOverlay,
    /// Names of overlays already applied in the target repo.
    pub(crate) applied_overlays: &'a [crate::OverlayName],
}

impl crate::selection::ToSelectableItem for BrowseOverlayItem<'_> {
    fn to_selectable_item(&self, target: &Path) -> crate::selection::SelectableItem {
        let normalized = crate::state::normalize_overlay_name(&self.overlay.name).ok();
        let disabled = normalized
            .as_ref()
            .is_some_and(|n| self.applied_overlays.iter().any(|name| name == n.as_str()));
        let description = if disabled {
            let name = normalized
                .as_ref()
                .map_or(self.overlay.name.as_str(), |n| n.as_str());
            let desc = crate::load_overlay_state(target, name).ok().map_or_else(
                || "already applied".into(),
                |state| {
                    format!(
                        "last updated {}",
                        crate::state::format_relative_time(&state.applied_at)
                    )
                },
            );
            Some(desc)
        } else {
            None
        };
        crate::selection::SelectableItem {
            id: self.overlay.to_string(),
            label: self.overlay.to_string(),
            description,
            preselected: false,
            disabled,
        }
    }
}

/// Manager for the overlay repository.
pub(crate) struct OverlayRepoManager {
    /// Path to the cloned overlay repository
    repo_path: PathBuf,
    /// Configuration for the overlay repo
    config: OverlayRepoConfig,
}

impl OverlayRepoManager {
    /// Create a new overlay repository manager.
    pub(crate) fn new(config: OverlayRepoConfig) -> Result<Self> {
        let repo_path = match &config.local_path {
            Some(path) => path.clone(),
            None => default_overlay_repo_path()?,
        };

        Ok(Self { repo_path, config })
    }

    /// Get the path to the overlay repository.
    pub(crate) fn path(&self) -> &Path {
        &self.repo_path
    }

    /// Check if the overlay repository needs to be cloned.
    pub(crate) fn needs_clone(&self) -> bool {
        !self.repo_path.exists() || !self.repo_path.join(".git").exists()
    }

    /// Ensure the overlay repo is cloned.
    pub(crate) fn ensure_cloned(&self) -> Result<()> {
        if self.needs_clone() {
            self.clone_repo()?;
        }
        Ok(())
    }

    /// Validate that a URL is safe to pass to `git clone`.
    ///
    /// Rejects flag-like values and restricts to HTTPS/SSH schemes.
    fn validate_clone_url(url: &str) -> Result<()> {
        // Validate URL doesn't look like a flag (defense in depth)
        if url.starts_with('-') {
            bail!(
                "Invalid overlay repository URL: '{url}' starts with '-' (possible flag injection)"
            );
        }

        // Only allow HTTPS and SSH URLs to prevent local file access via file:// scheme.
        // Normalize to lowercase for comparison since URL schemes are case-insensitive (RFC 3986).
        let url_lower = url.to_ascii_lowercase();
        if !url_lower.starts_with("https://")
            && !url_lower.starts_with("git@")
            && !url_lower.starts_with("ssh://")
        {
            bail!(
                "Unsupported URL scheme: '{url}'. Only https://, ssh://, and git@ URLs are allowed"
            );
        }

        Ok(())
    }

    /// Clone the overlay repository.
    fn clone_repo(&self) -> Result<()> {
        Self::validate_clone_url(&self.config.url)?;

        // Create parent directories
        if let Some(parent) = self.repo_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let url = self.config.url.as_str();
        let repo_path_str = self.repo_path.to_string_lossy();
        let args = vec!["clone", "--depth", "1", "--", url, &repo_path_str];
        let message = format!("Cloning {}...", self.config.url);

        let (status, _) = crate::git::run_git_with_spinner(&args, None, &message, false)?;

        if !status.success() {
            bail!("Failed to clone overlay repository: {}", self.config.url);
        }

        self.save_meta()?;
        Ok(())
    }

    /// Pull latest changes from the remote.
    pub(crate) fn pull(&self) -> Result<()> {
        if !self.repo_path.exists() {
            bail!("Overlay repository not cloned. Run 'repoverlay source add <url>' first.");
        }

        let (status, _) = crate::git::run_git_with_spinner(
            &["pull", "--ff-only"],
            Some(&self.repo_path),
            "Pulling latest changes...",
            false,
        )?;

        if !status.success() {
            bail!("Failed to pull overlay repository");
        }

        self.save_meta()?;
        Ok(())
    }

    /// Get the current commit SHA.
    pub(crate) fn get_current_commit(&self) -> Result<String> {
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
    pub(crate) fn list_overlays(&self) -> Result<Vec<AvailableOverlay>> {
        if !self.repo_path.exists() {
            bail!("Overlay repository not cloned. Run 'repoverlay source add <url>' first.");
        }

        let mut overlays = Vec::new();

        // Walk the directory structure: org/repo/overlay-name/
        for org_entry in fs::read_dir(&self.repo_path)? {
            let org_entry = org_entry?;
            let org_path = org_entry.path();

            // Skip non-directories, hidden files, and reserved namespaces (e.g.
            // @global) so older clients tolerate sources that use them.
            if !org_path.is_dir()
                || org_entry.file_name().to_string_lossy().starts_with('.')
                || crate::library::is_reserved_namespace(&org_entry.file_name().to_string_lossy())
            {
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
                    overlays.push(AvailableOverlay::structured(
                        org_name.clone(),
                        repo_name.clone(),
                        overlay_name,
                        has_config,
                    ));
                }
            }
        }

        // Sort by org/repo/name
        overlays.sort_by(|a, b| (&a.org, &a.repo, &a.name).cmp(&(&b.org, &b.repo, &b.name)));

        // Surface overlays in the reserved @global namespace.
        overlays.extend(scan_global_overlays(&self.repo_path)?);

        Ok(overlays)
    }

    /// List overlays for a specific target repository.
    ///
    /// Global overlays (in the `@global/` namespace) are always included, since
    /// they apply to any repository.
    pub(crate) fn list_overlays_for_repo(
        &self,
        org: &str,
        repo: &str,
    ) -> Result<Vec<AvailableOverlay>> {
        let all = self.list_overlays()?;
        Ok(all
            .into_iter()
            .filter(|o| {
                o.is_global()
                    || (o.org.eq_ignore_ascii_case(org) && o.repo.eq_ignore_ascii_case(repo))
            })
            .collect())
    }

    /// Get the path to a specific overlay.
    pub(crate) fn get_overlay_path(&self, org: &str, repo: &str, name: &str) -> Result<PathBuf> {
        validate_path_component(org, "org")?;
        validate_path_component(repo, "repo")?;
        validate_path_component(name, "overlay name")?;
        let path = self.repo_path.join(org).join(repo).join(name);

        if !path.exists() {
            bail!("Overlay not found: {org}/{repo}/{name}");
        }

        Ok(path)
    }

    /// Get the path to a global overlay in the `@global/` namespace.
    ///
    /// Returns `Ok(None)` when the overlay is absent. The overlay name is
    /// validated for path safety.
    pub(crate) fn get_global_overlay_path(&self, name: &str) -> Result<Option<PathBuf>> {
        validate_path_component(name, "overlay name")?;
        let path = self
            .repo_path
            .join(crate::library::GLOBAL_NAMESPACE)
            .join(name);
        if path.is_dir() {
            Ok(Some(path))
        } else {
            Ok(None)
        }
    }

    /// Find the path to a specific overlay with upstream fallback.
    ///
    /// Returns `Ok(None)` only when the overlay is absent. Invalid input and
    /// filesystem/config errors are returned as errors so callers do not treat
    /// them as "not found".
    pub(crate) fn find_overlay_path_with_fallback(
        &self,
        org: &str,
        repo: &str,
        name: &str,
        upstream: Option<&UpstreamInfo>,
    ) -> Result<Option<(PathBuf, ResolvedVia)>> {
        validate_path_component(org, "org")?;
        validate_path_component(repo, "repo")?;
        validate_path_component(name, "overlay name")?;
        // Try exact match first
        let direct_path = self.repo_path.join(org).join(repo).join(name);
        if direct_path.exists() {
            return Ok(Some((direct_path, ResolvedVia::Direct)));
        }

        // Try upstream fallback if available
        if let Some(up) = upstream {
            validate_path_component(&up.org, "upstream org")?;
            validate_path_component(&up.repo, "upstream repo")?;
            let upstream_path = self.repo_path.join(&up.org).join(&up.repo).join(name);
            if upstream_path.exists() {
                return Ok(Some((upstream_path, ResolvedVia::Upstream)));
            }
        }

        Ok(None)
    }

    /// Stage an overlay for publishing.
    ///
    /// Copies files from `source_dir` to the overlay repo at org/repo/name/
    /// Returns the destination path.
    #[cfg(test)]
    pub(crate) fn stage_overlay(
        &self,
        org: &str,
        repo: &str,
        name: &str,
        source_dir: &Path,
    ) -> Result<PathBuf> {
        validate_path_component(org, "org")?;
        validate_path_component(repo, "repo")?;
        validate_path_component(name, "overlay name")?;
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
    pub(crate) fn has_staged_changes(&self) -> Result<bool> {
        let output = Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute git diff")?;

        // Exit code 0 means no changes, 1 means changes
        Ok(!output.status.success())
    }

    /// Commit staged changes.
    pub(crate) fn commit(&self, message: &str) -> Result<()> {
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
}

/// Get the default path for the overlay repository clone.
///
/// Returns `~/.config/repoverlay/overlay-repo/` - stored alongside config
/// since it's user-managed content.
pub(crate) fn default_overlay_repo_path() -> Result<PathBuf> {
    Ok(crate::config::config_dir()?.join(OVERLAY_REPO_DIR))
}

/// Maximum recursion depth for directory copying to prevent stack overflow
/// from circular symlinks or excessively nested directories.
const MAX_COPY_DEPTH: usize = 64;

/// Copy a directory recursively.
///
/// Rejects symlinks that point outside the source root to prevent
/// exfiltration of files from the host filesystem via malicious overlays.
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    let canonical_root = src
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize source root: {}", src.display()))?;
    copy_dir_recursive_inner(src, dst, &canonical_root, 0)
}

fn copy_dir_recursive_inner(
    src: &Path,
    dst: &Path,
    canonical_root: &Path,
    depth: usize,
) -> Result<()> {
    if depth > MAX_COPY_DEPTH {
        bail!(
            "Maximum directory depth ({MAX_COPY_DEPTH}) exceeded copying {}: possible circular symlinks",
            src.display()
        );
    }

    if !src.is_dir() {
        bail!("Source is not a directory: {}", src.display());
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        // Check for symlinks that escape the source root
        let metadata = fs::symlink_metadata(&src_path)
            .with_context(|| format!("Failed to read metadata: {}", src_path.display()))?;
        if metadata.file_type().is_symlink() {
            match src_path.canonicalize() {
                Ok(canonical) => {
                    if !canonical.starts_with(canonical_root) {
                        bail!(
                            "Symlink escape detected: {} points outside source directory",
                            src_path.display()
                        );
                    }
                }
                Err(_) => {
                    // Dangling symlink - target doesn't exist. Skip it rather than
                    // failing the entire copy, since a broken symlink can't exfiltrate data.
                    continue;
                }
            }
        }

        if src_path.is_dir() {
            // Skip .git directory
            if entry.file_name() == ".git" {
                continue;
            }
            fs::create_dir_all(&dst_path)?;
            copy_dir_recursive_inner(&src_path, &dst_path, canonical_root, depth + 1)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Validate that a directory tree contains no symlinks escaping `root`.
///
/// Used before symlinking an overlay directory into a target repository: the
/// directory symlink exposes the overlay's contents as-is, so an embedded
/// symlink pointing outside the overlay would let a malicious overlay expose
/// arbitrary host paths through the target repository. Copy mode gets the
/// equivalent protection inside `copy_dir_recursive`.
pub(crate) fn ensure_no_escaping_symlinks(root: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize directory: {}", root.display()))?;
    ensure_no_escaping_symlinks_inner(&canonical_root, &canonical_root, 0)
}

fn ensure_no_escaping_symlinks_inner(
    dir: &Path,
    canonical_root: &Path,
    depth: usize,
) -> Result<()> {
    if depth > MAX_COPY_DEPTH {
        bail!(
            "Maximum directory depth ({MAX_COPY_DEPTH}) exceeded scanning {}: possible circular symlinks",
            dir.display()
        );
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name() == ".git" {
            continue;
        }

        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("Failed to read metadata: {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            let contained = if let Ok(canonical) = path.canonicalize() {
                canonical.starts_with(canonical_root)
            } else {
                // Dangling symlink. It cannot exfiltrate data today, but it
                // becomes live if its target is created later, so reject it
                // unless the target stays lexically within the root.
                let link_target = fs::read_link(&path)
                    .with_context(|| format!("Failed to read symlink: {}", path.display()))?;
                lexically_contained(&path, &link_target, canonical_root)
            };
            if !contained {
                bail!(
                    "Symlink escape detected: {} points outside the overlay directory",
                    path.display()
                );
            }
        } else if metadata.is_dir() {
            ensure_no_escaping_symlinks_inner(&path, canonical_root, depth + 1)?;
        }
    }

    Ok(())
}

/// Lexically resolve a symlink target and check it stays within `root`.
///
/// Used for dangling symlinks, where canonicalization is impossible. This is
/// best-effort by nature (it cannot follow chains of dangling links), which is
/// fine: it only ever rejects, never grants, access beyond the canonical check.
fn lexically_contained(link_path: &Path, target: &Path, root: &Path) -> bool {
    use std::path::Component;

    let mut resolved = if target.is_absolute() {
        PathBuf::new()
    } else {
        link_path.parent().unwrap_or(root).to_path_buf()
    };
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() {
                    return false;
                }
            }
            other => resolved.push(other),
        }
    }
    resolved.starts_with(root)
}

/// Parse an overlay reference in the format "org/repo/name".
#[allow(dead_code)] // Kept for backward compatibility; new code uses reference::SourceReference
pub(crate) fn parse_overlay_reference(s: &str) -> Option<(String, String, String)> {
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
    use tempfile::TempDir;

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

    #[test]
    fn test_overlay_repo_meta_roundtrip() {
        let meta = OverlayRepoMeta {
            clone_url: "https://github.com/org/repo.git".to_string(),
            last_fetched: Utc::now(),
            commit: "abc123def456".to_string(),
        };

        let serialized = sickle::to_string(&meta).unwrap();
        let deserialized: OverlayRepoMeta = sickle::from_str(&serialized).unwrap();

        assert_eq!(deserialized.clone_url, meta.clone_url);
        assert_eq!(deserialized.commit, meta.commit);
    }

    #[test]
    fn test_available_overlay_clone() {
        let overlay = AvailableOverlay::structured(
            "microsoft".to_string(),
            "FluidFramework".to_string(),
            "claude-config".to_string(),
            true,
        );

        let cloned = overlay.clone();
        assert_eq!(cloned.org, overlay.org);
        assert_eq!(cloned.repo, overlay.repo);
        assert_eq!(cloned.name, overlay.name);
        assert_eq!(cloned.has_config, overlay.has_config);
    }

    #[test]
    fn available_overlay_structured_computes_source_relative_path() {
        let overlay = AvailableOverlay::structured(
            "owner".to_string(),
            "repo".to_string(),
            "config".to_string(),
            true,
        );

        assert_eq!(overlay.to_string(), "owner/repo/config");
        assert_eq!(
            overlay.source_relative_path(),
            PathBuf::from("owner").join("repo").join("config")
        );
        assert!(!overlay.is_flat());
    }

    #[test]
    fn available_overlay_flat_uses_supplied_source_relative_path() {
        let overlay = AvailableOverlay::flat(
            "config-a".to_string(),
            PathBuf::from("nested/config-a"),
            false,
        );

        assert_eq!(overlay.to_string(), "config-a");
        assert_eq!(
            overlay.source_relative_path(),
            PathBuf::from("nested/config-a")
        );
        assert!(overlay.is_flat());
    }

    #[test]
    fn available_overlay_global_renders_star_prefix() {
        let overlay = AvailableOverlay::global("dotfiles".to_string(), true);

        assert_eq!(overlay.to_string(), "*/dotfiles");
        assert_eq!(overlay.org, crate::library::GLOBAL_NAMESPACE);
        assert!(overlay.repo.is_empty());
        assert!(overlay.is_global());
        assert!(!overlay.is_flat());
        assert_eq!(
            overlay.source_relative_path(),
            PathBuf::from(crate::library::GLOBAL_NAMESPACE).join("dotfiles")
        );
    }

    #[test]
    fn list_overlays_for_repo_includes_globals_for_any_repo() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        // One repo-scoped overlay and one global overlay.
        fs::create_dir_all(repo_path.join("acme/widget/cfg")).unwrap();
        fs::write(repo_path.join("acme/widget/cfg/.envrc"), "x").unwrap();
        fs::create_dir_all(repo_path.join("@global/dotfiles")).unwrap();
        fs::write(repo_path.join("@global/dotfiles/.gitconfig"), "x").unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };
        let manager = OverlayRepoManager::new(config).unwrap();

        // The global appears regardless of which repo we ask for.
        let matching = manager.list_overlays_for_repo("acme", "widget").unwrap();
        assert!(matching.iter().any(|o| o.name == "cfg" && !o.is_global()));
        assert!(
            matching
                .iter()
                .any(|o| o.name == "dotfiles" && o.is_global())
        );

        let unrelated = manager.list_overlays_for_repo("other", "thing").unwrap();
        assert_eq!(unrelated.len(), 1);
        assert!(unrelated[0].is_global());
        assert_eq!(unrelated[0].name, "dotfiles");
    }

    #[test]
    fn get_global_overlay_path_resolves_present_and_absent() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join("@global/dotfiles")).unwrap();
        fs::write(repo_path.join("@global/dotfiles/.gitconfig"), "x").unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path.clone()),
        };
        let manager = OverlayRepoManager::new(config).unwrap();

        let found = manager.get_global_overlay_path("dotfiles").unwrap();
        assert_eq!(found, Some(repo_path.join("@global").join("dotfiles")));
        assert!(
            manager
                .get_global_overlay_path("missing")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_overlay_repo_manager_needs_clone_no_path() {
        let temp = TempDir::new().unwrap();
        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(temp.path().join("nonexistent")),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        assert!(manager.needs_clone());
    }

    #[test]
    fn test_overlay_repo_manager_needs_clone_no_git_dir() {
        let temp = TempDir::new().unwrap();
        // Create directory but not .git subdirectory
        fs::create_dir_all(temp.path().join("overlay-repo")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(temp.path().join("overlay-repo")),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        assert!(manager.needs_clone());
    }

    #[test]
    fn test_overlay_repo_manager_does_not_need_clone_with_git_dir() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        assert!(!manager.needs_clone());
    }

    #[test]
    fn test_copy_dir_recursive_basic() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("file1.txt"), "content1").unwrap();
        fs::write(src.join("file2.txt"), "content2").unwrap();

        fs::create_dir_all(&dst).unwrap();
        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("file1.txt").exists());
        assert!(dst.join("file2.txt").exists());
        assert_eq!(
            fs::read_to_string(dst.join("file1.txt")).unwrap(),
            "content1"
        );
    }

    #[test]
    fn test_copy_dir_recursive_nested() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        fs::create_dir_all(src.join("subdir")).unwrap();
        fs::write(src.join("root.txt"), "root").unwrap();
        fs::write(src.join("subdir/nested.txt"), "nested").unwrap();

        fs::create_dir_all(&dst).unwrap();
        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("root.txt").exists());
        assert!(dst.join("subdir/nested.txt").exists());
        assert_eq!(
            fs::read_to_string(dst.join("subdir/nested.txt")).unwrap(),
            "nested"
        );
    }

    #[test]
    fn test_copy_dir_recursive_skips_git_dir() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        fs::create_dir_all(src.join(".git")).unwrap();
        fs::write(src.join("file.txt"), "content").unwrap();
        fs::write(src.join(".git/config"), "git config").unwrap();

        fs::create_dir_all(&dst).unwrap();
        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("file.txt").exists());
        assert!(!dst.join(".git").exists());
    }

    #[test]
    fn test_copy_dir_recursive_fails_on_non_directory() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("file.txt");
        fs::write(&file_path, "content").unwrap();

        let result = copy_dir_recursive(&file_path, temp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a directory"));
    }

    #[test]
    fn test_get_overlay_path_nonexistent() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let result = manager.get_overlay_path("org", "repo", "missing");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_get_overlay_path_exists() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();
        fs::create_dir_all(repo_path.join("org/repo/overlay-name")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path.clone()),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let result = manager
            .get_overlay_path("org", "repo", "overlay-name")
            .unwrap();

        assert_eq!(result, repo_path.join("org/repo/overlay-name"));
    }

    #[test]
    fn test_overlay_repo_manager_path_getter() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path.clone()),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        assert_eq!(manager.path(), repo_path);
    }

    #[test]
    fn test_ensure_cloned_when_already_cloned() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        // Should succeed without attempting to clone
        assert!(manager.ensure_cloned().is_ok());
    }

    #[test]
    fn test_list_overlays_not_cloned() {
        let temp = TempDir::new().unwrap();
        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(temp.path().join("nonexistent")),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let result = manager.list_overlays();

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not cloned"));
    }

    #[test]
    fn test_list_overlays_empty_repo() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let overlays = manager.list_overlays().unwrap();

        assert!(overlays.is_empty());
    }

    #[test]
    fn test_list_overlays_with_overlays() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        // Create overlay directories
        fs::create_dir_all(repo_path.join("microsoft/FluidFramework/claude-config")).unwrap();
        fs::create_dir_all(repo_path.join("microsoft/FluidFramework/copilot-config")).unwrap();
        fs::create_dir_all(repo_path.join("other-org/other-repo/test-overlay")).unwrap();

        // Add a config file to one overlay
        fs::write(
            repo_path.join("microsoft/FluidFramework/claude-config/repoverlay.ccl"),
            "# config",
        )
        .unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let overlays = manager.list_overlays().unwrap();

        assert_eq!(overlays.len(), 3);

        // Should be sorted
        assert_eq!(overlays[0].org, "microsoft");
        assert_eq!(overlays[0].repo, "FluidFramework");
        assert_eq!(overlays[0].name, "claude-config");
        assert!(overlays[0].has_config);

        assert_eq!(overlays[1].org, "microsoft");
        assert_eq!(overlays[1].repo, "FluidFramework");
        assert_eq!(overlays[1].name, "copilot-config");
        assert!(!overlays[1].has_config);

        assert_eq!(overlays[2].org, "other-org");
        assert_eq!(overlays[2].repo, "other-repo");
        assert_eq!(overlays[2].name, "test-overlay");
    }

    #[test]
    fn test_list_overlays_skips_hidden_dirs() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        // Create a normal overlay and hidden directories at various levels
        fs::create_dir_all(repo_path.join("org/repo/overlay")).unwrap();
        fs::create_dir_all(repo_path.join(".hidden-org/repo/overlay")).unwrap();
        fs::create_dir_all(repo_path.join("org/.hidden-repo/overlay")).unwrap();
        fs::create_dir_all(repo_path.join("org/repo/.hidden-overlay")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let overlays = manager.list_overlays().unwrap();

        // Only the non-hidden overlay should be found
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].org, "org");
        assert_eq!(overlays[0].repo, "repo");
        assert_eq!(overlays[0].name, "overlay");
    }

    #[test]
    fn test_list_overlays_surfaces_global_namespace() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        // A normal overlay plus a reserved @global namespace entry.
        fs::create_dir_all(repo_path.join("org/repo/overlay")).unwrap();
        fs::create_dir_all(repo_path.join("@global/my-global")).unwrap();
        fs::write(repo_path.join("@global/my-global/.envrc"), "export FOO=1").unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        // @global must never be mis-parsed as a literal org; instead it is
        // surfaced as a global overlay via the dedicated scan pass.
        let manager = OverlayRepoManager::new(config).unwrap();
        let overlays = manager.list_overlays().unwrap();

        assert_eq!(overlays.len(), 2);
        assert!(overlays.iter().all(|o| o.is_global() || o.org != "@global"));
        let structured = overlays.iter().find(|o| !o.is_global()).unwrap();
        assert_eq!(structured.org, "org");
        assert_eq!(structured.repo, "repo");
        assert_eq!(structured.name, "overlay");
        let global = overlays.iter().find(|o| o.is_global()).unwrap();
        assert_eq!(global.name, "my-global");
        assert!(global.is_global());
    }

    #[test]
    fn test_get_overlay_path_rejects_reserved_namespace() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join("@global/repo/name")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };
        let manager = OverlayRepoManager::new(config).unwrap();

        assert!(manager.get_overlay_path("@global", "repo", "name").is_err());
        assert!(
            manager
                .get_overlay_path("@library", "repo", "name")
                .is_err()
        );
    }

    #[test]
    fn test_list_overlays_skips_files() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        // Create a normal overlay
        fs::create_dir_all(repo_path.join("org/repo/overlay")).unwrap();

        // Create files at various levels that should be skipped
        fs::write(repo_path.join("README.md"), "readme").unwrap();
        fs::write(repo_path.join("org/README.md"), "readme").unwrap();
        fs::write(repo_path.join("org/repo/README.md"), "readme").unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let overlays = manager.list_overlays().unwrap();

        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].name, "overlay");
    }

    #[test]
    fn test_list_overlays_for_repo_filters_correctly() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        fs::create_dir_all(repo_path.join("microsoft/FluidFramework/claude-config")).unwrap();
        fs::create_dir_all(repo_path.join("microsoft/FluidFramework/copilot-config")).unwrap();
        fs::create_dir_all(repo_path.join("other-org/other-repo/test-overlay")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let overlays = manager
            .list_overlays_for_repo("microsoft", "FluidFramework")
            .unwrap();

        assert_eq!(overlays.len(), 2);
        assert!(overlays.iter().all(|o| o.org == "microsoft"));
        assert!(overlays.iter().all(|o| o.repo == "FluidFramework"));
    }

    #[test]
    fn test_list_overlays_for_repo_case_insensitive() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        fs::create_dir_all(repo_path.join("Microsoft/FluidFramework/overlay")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();

        // Search with different casing
        let overlays = manager
            .list_overlays_for_repo("microsoft", "fluidframework")
            .unwrap();

        assert_eq!(overlays.len(), 1);
    }

    #[test]
    fn test_list_overlays_for_repo_no_matches() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        fs::create_dir_all(repo_path.join("org/repo/overlay")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let overlays = manager
            .list_overlays_for_repo("nonexistent", "repo")
            .unwrap();

        assert!(overlays.is_empty());
    }

    #[test]
    fn test_pull_not_cloned() {
        let temp = TempDir::new().unwrap();
        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(temp.path().join("nonexistent")),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let result = manager.pull();

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not cloned"));
    }

    #[test]
    fn test_find_overlay_path_with_fallback_direct_match() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();
        fs::create_dir_all(repo_path.join("tylerbutler/FluidFramework/claude-config")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let upstream = Some(crate::upstream::UpstreamInfo {
            org: "microsoft".to_string(),
            repo: "FluidFramework".to_string(),
            remote_name: "upstream".to_string(),
        });

        let (path, resolved_via) = manager
            .find_overlay_path_with_fallback(
                "tylerbutler",
                "FluidFramework",
                "claude-config",
                upstream.as_ref(),
            )
            .unwrap()
            .expect("overlay should be found");

        assert!(path.ends_with("tylerbutler/FluidFramework/claude-config"));
        assert_eq!(resolved_via, crate::state::ResolvedVia::Direct);
    }

    #[test]
    fn test_find_overlay_path_with_fallback_uses_upstream() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();
        // Only upstream overlay exists, not fork-specific
        fs::create_dir_all(repo_path.join("microsoft/FluidFramework/claude-config")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let upstream = Some(crate::upstream::UpstreamInfo {
            org: "microsoft".to_string(),
            repo: "FluidFramework".to_string(),
            remote_name: "upstream".to_string(),
        });

        let (path, resolved_via) = manager
            .find_overlay_path_with_fallback(
                "tylerbutler",
                "FluidFramework",
                "claude-config",
                upstream.as_ref(),
            )
            .unwrap()
            .expect("upstream overlay should be found");

        assert!(path.ends_with("microsoft/FluidFramework/claude-config"));
        assert_eq!(resolved_via, crate::state::ResolvedVia::Upstream);
    }

    #[test]
    fn test_find_overlay_path_with_fallback_no_upstream_returns_none() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();
        // No overlays exist

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();

        let result = manager
            .find_overlay_path_with_fallback("tylerbutler", "FluidFramework", "claude-config", None)
            .unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn find_overlay_path_with_fallback_returns_none_for_absent_overlay() {
        let temp = TempDir::new().unwrap();
        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(temp.path().to_path_buf()),
        };
        let manager = OverlayRepoManager::new(config).unwrap();

        let found = manager
            .find_overlay_path_with_fallback("org", "repo", "missing", None)
            .unwrap();

        assert!(found.is_none());
    }

    #[test]
    fn find_overlay_path_with_fallback_propagates_invalid_components() {
        let temp = TempDir::new().unwrap();
        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(temp.path().to_path_buf()),
        };
        let manager = OverlayRepoManager::new(config).unwrap();

        let err = manager
            .find_overlay_path_with_fallback("org", "repo", "../escape", None)
            .unwrap_err();

        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn test_get_current_commit_on_non_git_directory() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("not-a-repo");
        fs::create_dir_all(&repo_path).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let result = manager.get_current_commit();
        assert!(result.is_err());
    }

    #[test]
    fn test_get_current_commit_on_valid_repo() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("repo");

        // Initialize a real git repo with a commit
        std::process::Command::new("git")
            .args(["init"])
            .arg(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        fs::write(repo_path.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let commit = manager.get_current_commit().unwrap();
        assert_eq!(commit.len(), 40);
        assert!(commit.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_stage_overlay_with_source_files() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");

        // Initialize a real git repo
        std::process::Command::new("git")
            .args(["init"])
            .arg(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Create initial commit
        fs::write(repo_path.join("README.md"), "repo").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path.clone()),
        };

        let manager = OverlayRepoManager::new(config).unwrap();

        // Create source overlay directory
        let source = TempDir::new().unwrap();
        fs::write(source.path().join(".envrc"), "export FOO=bar").unwrap();

        let dest = manager
            .stage_overlay("org", "repo", "my-overlay", source.path())
            .unwrap();

        assert_eq!(dest, repo_path.join("org/repo/my-overlay"));
        assert!(dest.join(".envrc").exists());
    }

    #[test]
    fn test_stage_overlay_with_nonexistent_source() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");

        // Initialize a real git repo
        std::process::Command::new("git")
            .args(["init"])
            .arg(&repo_path)
            .output()
            .unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();

        // Try to stage from non-existent source
        let result = manager.stage_overlay(
            "org",
            "repo",
            "overlay",
            Path::new("/nonexistent/source/path/xyz123"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_has_staged_changes_empty_repo() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("repo");

        // Initialize a real git repo with a commit
        std::process::Command::new("git")
            .args(["init"])
            .arg(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        fs::write(repo_path.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();

        // No staged changes
        let has_changes = manager.has_staged_changes().unwrap();
        assert!(!has_changes);
    }

    #[test]
    fn test_has_staged_changes_with_staged_files() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("repo");

        // Initialize a real git repo with a commit
        std::process::Command::new("git")
            .args(["init"])
            .arg(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        fs::write(repo_path.join("file.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // Add a new file and stage it
        fs::write(repo_path.join("new.txt"), "new content").unwrap();
        std::process::Command::new("git")
            .args(["add", "new.txt"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();

        let has_changes = manager.has_staged_changes().unwrap();
        assert!(has_changes);
    }

    #[test]
    fn test_list_overlays_for_repo_multiple_overlays() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();
        fs::create_dir_all(repo_path.join("microsoft/FluidFramework/claude-config")).unwrap();
        fs::create_dir_all(repo_path.join("microsoft/FluidFramework/vscode-setup")).unwrap();
        fs::create_dir_all(repo_path.join("google/chromium/dev-tools")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();

        let overlays = manager
            .list_overlays_for_repo("microsoft", "FluidFramework")
            .unwrap();
        assert_eq!(overlays.len(), 2);

        // Different repo should not be included
        let chrome_overlays = manager
            .list_overlays_for_repo("google", "chromium")
            .unwrap();
        assert_eq!(chrome_overlays.len(), 1);
    }

    #[test]
    fn test_list_overlays_skips_hidden_and_files() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();
        fs::create_dir_all(repo_path.join("org/repo/visible-overlay")).unwrap();
        fs::create_dir_all(repo_path.join("org/repo/.hidden-overlay")).unwrap();
        fs::write(repo_path.join("org/repo/not-a-dir.txt"), "file").unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        let overlays = manager.list_overlays().unwrap();
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].name, "visible-overlay");
    }

    // --- Security fix tests ---

    #[test]
    fn test_validate_path_component_rejects_traversal() {
        assert!(validate_path_component("..", "test").is_err());
        assert!(validate_path_component(".", "test").is_err());
        assert!(validate_path_component("../etc", "test").is_err());
        assert!(validate_path_component("foo/..", "test").is_err());
        assert!(validate_path_component("foo/bar", "test").is_err());
        assert!(validate_path_component("foo\\bar", "test").is_err());
        assert!(validate_path_component("", "test").is_err());
    }

    #[test]
    fn test_validate_path_component_allows_valid() {
        assert!(validate_path_component("microsoft", "test").is_ok());
        assert!(validate_path_component("FluidFramework", "test").is_ok());
        assert!(validate_path_component("claude-config", "test").is_ok());
        assert!(validate_path_component("my_overlay", "test").is_ok());
        assert!(validate_path_component(".github", "test").is_ok());
        assert!(validate_path_component(".hidden", "test").is_ok());
    }

    #[test]
    fn test_get_overlay_path_rejects_traversal() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_path.join(".git")).unwrap();

        let config = OverlayRepoConfig {
            url: "https://github.com/org/overlays".to_string(),
            local_path: Some(repo_path),
        };

        let manager = OverlayRepoManager::new(config).unwrap();
        assert!(manager.get_overlay_path("..", "repo", "name").is_err());
        assert!(manager.get_overlay_path("org", "..", "name").is_err());
        assert!(manager.get_overlay_path("org", "repo", "..").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_copy_dir_recursive_rejects_symlink_escape() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("safe.txt"), "safe").unwrap();

        // Create a symlink that escapes the source directory
        std::os::unix::fs::symlink("/etc/hosts", src.join("escape.txt")).unwrap();

        fs::create_dir_all(&dst).unwrap();
        let result = copy_dir_recursive(&src, &dst);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Symlink escape detected") || err.contains("points outside"),
            "Expected symlink escape error, got: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_copy_dir_recursive_allows_internal_symlinks() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        fs::create_dir_all(src.join("subdir")).unwrap();
        fs::write(src.join("subdir/target.txt"), "content").unwrap();

        // Create a symlink within the source directory
        std::os::unix::fs::symlink(src.join("subdir/target.txt"), src.join("internal-link.txt"))
            .unwrap();

        fs::create_dir_all(&dst).unwrap();
        let result = copy_dir_recursive(&src, &dst);
        assert!(result.is_ok());
        assert!(dst.join("internal-link.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_copy_dir_recursive_skips_dangling_symlinks() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("real.txt"), "content").unwrap();

        // Create a symlink to a non-existent target within the source tree
        std::os::unix::fs::symlink(src.join("nonexistent.txt"), src.join("dangling.txt")).unwrap();

        fs::create_dir_all(&dst).unwrap();
        let result = copy_dir_recursive(&src, &dst);
        assert!(
            result.is_ok(),
            "Dangling symlinks should be skipped, not cause failure"
        );
        assert!(dst.join("real.txt").exists());
        assert!(
            !dst.join("dangling.txt").exists(),
            "Dangling symlink should not be copied"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_ensure_no_escaping_symlinks_rejects_escape() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("nested")).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", root.join("nested/escape")).unwrap();

        let result = ensure_no_escaping_symlinks(&root);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Symlink escape detected")
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_ensure_no_escaping_symlinks_allows_internal() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("subdir")).unwrap();
        fs::write(root.join("subdir/target.txt"), "content").unwrap();
        std::os::unix::fs::symlink("subdir/target.txt", root.join("link.txt")).unwrap();

        assert!(ensure_no_escaping_symlinks(&root).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_ensure_no_escaping_symlinks_allows_dangling_internal() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(&root).unwrap();
        // Dangling but lexically inside the root: harmless even if created later
        std::os::unix::fs::symlink("not-yet-created.txt", root.join("dangling")).unwrap();

        assert!(ensure_no_escaping_symlinks(&root).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_ensure_no_escaping_symlinks_rejects_dangling_escape() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(&root).unwrap();
        // Dangling relative link that would resolve outside the root if its
        // target is ever created
        std::os::unix::fs::symlink("../outside/secret.txt", root.join("dangling")).unwrap();

        let result = ensure_no_escaping_symlinks(&root);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Symlink escape detected")
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_ensure_no_escaping_symlinks_rejects_dangling_absolute() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink("/nonexistent/path/secret", root.join("dangling")).unwrap();

        let result = ensure_no_escaping_symlinks(&root);
        assert!(result.is_err());
    }

    #[test]
    fn test_copy_dir_recursive_depth_limit() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        // Create deeply nested directories exceeding MAX_COPY_DEPTH
        let mut deepest = src.clone();
        for i in 0..=MAX_COPY_DEPTH + 1 {
            deepest = deepest.join(format!("level{i}"));
        }
        fs::create_dir_all(&deepest).unwrap();
        fs::write(deepest.join("deep.txt"), "deep").unwrap();

        fs::create_dir_all(&dst).unwrap();
        let result = copy_dir_recursive(&src, &dst);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Maximum directory depth")
        );
    }

    #[test]
    fn test_validate_clone_url_rejects_flag() {
        let result = OverlayRepoManager::validate_clone_url("--upload-pack=evil");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("flag injection"));
    }

    #[test]
    fn test_validate_clone_url_rejects_file_scheme() {
        let result = OverlayRepoManager::validate_clone_url("file:///etc/shadow");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unsupported URL scheme")
        );
    }

    #[test]
    fn test_validate_clone_url_allows_https() {
        OverlayRepoManager::validate_clone_url("https://github.com/org/repo.git").unwrap();
    }

    #[test]
    fn test_validate_clone_url_allows_ssh() {
        OverlayRepoManager::validate_clone_url("git@github.com:org/repo.git").unwrap();
        OverlayRepoManager::validate_clone_url("ssh://git@github.com/org/repo.git").unwrap();
    }

    #[test]
    fn available_overlay_display() {
        let o = AvailableOverlay::structured(
            "microsoft".to_string(),
            "FluidFramework".to_string(),
            "vscode-setup".to_string(),
            true,
        );
        assert_eq!(o.to_string(), "microsoft/FluidFramework/vscode-setup");
    }

    #[test]
    fn available_overlay_display_bold_contains_name() {
        let o = AvailableOverlay::structured(
            "microsoft".to_string(),
            "FluidFramework".to_string(),
            "vscode-setup".to_string(),
            true,
        );
        let bold = o.display_bold();
        assert!(bold.contains("microsoft"));
        assert!(bold.contains("FluidFramework"));
        assert!(bold.contains("vscode-setup"));
    }

    #[test]
    fn snapshot_available_overlay_display() {
        let overlays = [
            AvailableOverlay::structured(
                "microsoft".to_string(),
                "FluidFramework".to_string(),
                "claude-config".to_string(),
                true,
            ),
            AvailableOverlay::structured(
                "owner".to_string(),
                "repo".to_string(),
                "my-overlay".to_string(),
                false,
            ),
        ];
        let output: Vec<String> = overlays.iter().map(|o| format!("{o}")).collect();
        insta::assert_snapshot!(output.join("\n"));
    }
}
