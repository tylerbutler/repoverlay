//! In-repo overlay library management.
//!
//! Handles the `.repoverlay/library/` directory for storing shareable overlays
//! within a repository. The library is auto-discovered and registered as an
//! implicit source with highest priority.

use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

use crate::overlay_repo::copy_dir_recursive;
use crate::state::STATE_DIR;

/// Default library subdirectory within .repoverlay/
const DEFAULT_LIBRARY_DIR: &str = "library";

/// Reserved source name for the library.
pub(crate) const LIBRARY_SOURCE_NAME: &str = "@library";

/// Resolve the library path for a given repository root.
///
/// Checks per-repo config for a custom path, falls back to default.
pub(crate) fn resolve_library_path(repo_root: &Path, config_path: Option<&str>) -> Result<PathBuf> {
    let library_path = match config_path {
        Some(custom) => {
            let path = PathBuf::from(custom);
            if path.is_absolute() {
                bail!("Library path must be relative, got: {}", path.display());
            }
            if path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                bail!(
                    "Library path must be within the repository root: {}",
                    path.display()
                );
            }
            repo_root.join(&path)
        }
        None => repo_root.join(STATE_DIR).join(DEFAULT_LIBRARY_DIR),
    };
    Ok(library_path)
}

/// Resolve the path to a specific overlay within the library.
pub(crate) fn resolve_library_overlay_path(
    repo_root: &Path,
    config_path: Option<&str>,
    overlay_name: &str,
) -> Result<PathBuf> {
    let library_path = resolve_library_path(repo_root, config_path)?;
    Ok(library_path.join(overlay_name))
}

/// Check if the library directory exists.
#[allow(dead_code)]
pub(crate) fn library_exists(repo_root: &Path, config_path: Option<&str>) -> bool {
    resolve_library_path(repo_root, config_path)
        .map(|p| p.is_dir())
        .unwrap_or(false)
}

/// An overlay found in the library.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct LibraryOverlay {
    /// Overlay name (directory name)
    pub(crate) name: String,
    /// Full path to the overlay directory
    pub(crate) path: PathBuf,
}

/// List all overlays in the library directory.
#[allow(dead_code)]
pub(crate) fn list_library_overlays(library_path: &Path) -> Result<Vec<LibraryOverlay>> {
    if !library_path.is_dir() {
        return Ok(Vec::new());
    }

    let mut overlays = Vec::new();
    for entry in fs::read_dir(library_path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            overlays.push(LibraryOverlay {
                name,
                path: entry.path(),
            });
        }
    }
    overlays.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(overlays)
}

/// Import (copy) an overlay directory into the library.
///
/// Creates the library directory if it doesn't exist.
#[allow(dead_code)]
pub(crate) fn import_to_library(
    source_path: &Path,
    library_path: &Path,
    name: &str,
    force: bool,
) -> Result<PathBuf> {
    let dest = library_path.join(name);

    if dest.exists() {
        if force {
            fs::remove_dir_all(&dest)?;
        } else {
            bail!(
                "Overlay '{}' already exists in library at {}. Use --force to overwrite or --name to rename.",
                name,
                dest.display()
            );
        }
    }

    // Create destination directory (including library parent if needed)
    fs::create_dir_all(&dest)?;

    // Copy overlay directory
    copy_dir_recursive(source_path, &dest)?;

    Ok(dest)
}

/// Remove an overlay from the library.
#[allow(dead_code)]
pub(crate) fn remove_from_library(library_path: &Path, name: &str) -> Result<()> {
    let overlay_path = library_path.join(name);
    if !overlay_path.is_dir() {
        bail!("Overlay '{name}' not found in library");
    }
    fs::remove_dir_all(&overlay_path)?;
    Ok(())
}

/// Export (copy) an overlay from the library to a destination.
#[allow(dead_code)]
pub(crate) fn export_from_library(library_path: &Path, name: &str, dest: &Path) -> Result<PathBuf> {
    let source = library_path.join(name);
    if !source.is_dir() {
        bail!("Overlay '{name}' not found in library");
    }

    let target = dest.join(name);
    if target.exists() {
        bail!(
            "Destination already exists: {}. Use --force to overwrite.",
            target.display()
        );
    }

    fs::create_dir_all(&target)?;
    copy_dir_recursive(&source, &target)?;
    Ok(target)
}

/// Check if the library path would be excluded by gitignore rules.
///
/// Returns true if the library path appears to be gitignored, meaning
/// overlays stored there won't be tracked by git.
#[allow(dead_code)]
pub(crate) fn check_library_gitignored(repo_root: &Path, library_path: &Path) -> bool {
    std::process::Command::new("git")
        .args(["check-ignore", "-q"])
        .arg(library_path)
        .current_dir(repo_root)
        .status()
        .map(|s| s.success()) // exit 0 means the path IS ignored
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_library_path() {
        let tmp = TempDir::new().unwrap();
        let path = resolve_library_path(tmp.path(), None).unwrap();
        assert_eq!(path, tmp.path().join(".repoverlay").join("library"));
    }

    #[test]
    fn custom_library_path() {
        let tmp = TempDir::new().unwrap();
        let path = resolve_library_path(tmp.path(), Some(".overlays")).unwrap();
        assert_eq!(path, tmp.path().join(".overlays"));
    }

    #[test]
    fn absolute_library_path_rejected() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_library_path(tmp.path(), Some("/absolute/path"));
        assert!(result.is_err());
    }

    #[test]
    fn parent_dir_escape_rejected() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_library_path(tmp.path(), Some("../escape"));
        assert!(result.is_err());
    }

    #[test]
    fn library_overlay_path_resolution() {
        let tmp = TempDir::new().unwrap();
        let path = resolve_library_overlay_path(tmp.path(), None, "claude-config").unwrap();
        assert_eq!(
            path,
            tmp.path()
                .join(".repoverlay")
                .join("library")
                .join("claude-config")
        );
    }

    #[test]
    fn library_exists_false_when_no_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(!library_exists(tmp.path(), None));
    }

    #[test]
    fn library_exists_true_when_dir_present() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".repoverlay").join("library")).unwrap();
        assert!(library_exists(tmp.path(), None));
    }

    #[test]
    fn list_overlays_empty_library() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(&library_path).unwrap();
        let overlays = list_library_overlays(&library_path).unwrap();
        assert!(overlays.is_empty());
    }

    #[test]
    fn list_overlays_finds_directories() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(library_path.join("claude-config")).unwrap();
        fs::create_dir_all(library_path.join("dev-env")).unwrap();
        // Files at library root should be ignored (only directories are overlays)
        fs::write(library_path.join("README.md"), "ignore me").unwrap();
        let overlays = list_library_overlays(&library_path).unwrap();
        assert_eq!(overlays.len(), 2);
        assert!(overlays.iter().any(|o| o.name == "claude-config"));
        assert!(overlays.iter().any(|o| o.name == "dev-env"));
    }

    #[test]
    fn list_overlays_nonexistent_dir_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join("nonexistent");
        let overlays = list_library_overlays(&library_path).unwrap();
        assert!(overlays.is_empty());
    }

    #[test]
    fn import_overlay_to_library() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");

        // Create a source overlay
        let source = tmp.path().join("source-overlay");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join(".envrc"), "use flake").unwrap();
        fs::write(source.join("CLAUDE.md"), "# Config").unwrap();

        import_to_library(&source, &library_path, "my-overlay", false).unwrap();

        let dest = library_path.join("my-overlay");
        assert!(dest.is_dir());
        assert!(dest.join(".envrc").exists());
        assert!(dest.join("CLAUDE.md").exists());
    }

    #[test]
    fn import_overlay_name_conflict_errors() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");

        let source = tmp.path().join("source-overlay");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("file.txt"), "content").unwrap();

        import_to_library(&source, &library_path, "my-overlay", false).unwrap();
        // Second import should fail
        let result = import_to_library(&source, &library_path, "my-overlay", false);
        assert!(result.is_err());
    }

    #[test]
    fn import_overlay_force_overwrites() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");

        let source = tmp.path().join("source-overlay");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("file.txt"), "v1").unwrap();

        import_to_library(&source, &library_path, "my-overlay", false).unwrap();

        fs::write(source.join("file.txt"), "v2").unwrap();
        import_to_library(&source, &library_path, "my-overlay", true).unwrap();

        let content = fs::read_to_string(library_path.join("my-overlay").join("file.txt")).unwrap();
        assert_eq!(content, "v2");
    }

    #[test]
    fn remove_from_library_works() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");
        let overlay_path = library_path.join("my-overlay");
        fs::create_dir_all(&overlay_path).unwrap();
        fs::write(overlay_path.join("file.txt"), "content").unwrap();

        remove_from_library(&library_path, "my-overlay").unwrap();
        assert!(!overlay_path.exists());
    }

    #[test]
    fn remove_nonexistent_overlay_errors() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(&library_path).unwrap();

        let result = remove_from_library(&library_path, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn export_from_library_works() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");
        let overlay_path = library_path.join("my-overlay");
        fs::create_dir_all(&overlay_path).unwrap();
        fs::write(overlay_path.join("file.txt"), "content").unwrap();

        let dest = tmp.path().join("exported");
        fs::create_dir_all(&dest).unwrap();

        export_from_library(&library_path, "my-overlay", &dest).unwrap();

        assert!(dest.join("my-overlay").join("file.txt").exists());
        // Original should still exist (export is a copy)
        assert!(overlay_path.join("file.txt").exists());
    }

    #[test]
    fn export_nonexistent_overlay_errors() {
        let tmp = TempDir::new().unwrap();
        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(&library_path).unwrap();

        let dest = tmp.path().join("exported");
        let result = export_from_library(&library_path, "nonexistent", &dest);
        assert!(result.is_err());
    }

    #[test]
    fn warns_when_library_path_gitignored() {
        let tmp = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .status()
            .unwrap();

        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(&library_path).unwrap();

        // Create a .gitignore that excludes .repoverlay/
        fs::write(tmp.path().join(".gitignore"), ".repoverlay/\n").unwrap();

        assert!(check_library_gitignored(tmp.path(), &library_path));
    }

    #[test]
    fn no_warning_when_library_not_gitignored() {
        let tmp = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .status()
            .unwrap();

        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(&library_path).unwrap();

        assert!(!check_library_gitignored(tmp.path(), &library_path));
    }
}
