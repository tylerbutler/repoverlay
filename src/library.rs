//! In-repo overlay library management.
//!
//! Handles the `.repoverlay/library/` directory for storing shareable overlays
//! within a repository. The library is auto-discovered and registered as an
//! implicit source with highest priority.

use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config;
use crate::overlay_repo::copy_dir_recursive;
use crate::state::STATE_DIR;

/// Default library subdirectory within .repoverlay/
const DEFAULT_LIBRARY_DIR: &str = "library";

/// Reserved source name for the library.
pub(crate) const LIBRARY_SOURCE_NAME: &str = "@library";

/// Reserved namespace for global overlays (overlays that apply to any repository).
///
/// On disk, global overlays live at `<source>/@global/<name>/`, a sibling of the
/// `org/` directories in a structured source.
pub(crate) const GLOBAL_NAMESPACE: &str = "@global";

/// Returns `true` if `name` is a reserved namespace segment that must not be used
/// as a literal `org`, `repo`, or overlay-name directory in a source.
pub(crate) fn is_reserved_namespace(name: &str) -> bool {
    name == LIBRARY_SOURCE_NAME || name == GLOBAL_NAMESPACE
}

/// Resolve the library path for a given repository root, loading repo config automatically.
pub(crate) fn get_library_path(repo_root: &Path) -> Result<PathBuf> {
    let repo_config = config::load_repo_config(repo_root)?;
    let config_path = repo_config.as_ref().and_then(|c| c.library_path.as_deref());
    resolve_library_path(repo_root, config_path)
}

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

/// An overlay found in the library.
#[derive(Debug, Clone)]
pub(crate) struct LibraryOverlay {
    /// Overlay name (directory name)
    pub(crate) name: String,
}

/// List all overlays in the library directory.
pub(crate) fn list_library_overlays(library_path: &Path) -> Result<Vec<LibraryOverlay>> {
    if !library_path.is_dir() {
        return Ok(Vec::new());
    }

    let mut overlays = Vec::new();
    for entry in fs::read_dir(library_path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            overlays.push(LibraryOverlay { name });
        }
    }
    overlays.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(overlays)
}

/// Import (copy) an overlay directory into the library.
///
/// Creates the library directory if it doesn't exist.
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
pub(crate) fn remove_from_library(library_path: &Path, name: &str) -> Result<()> {
    let overlay_path = library_path.join(name);
    if !overlay_path.is_dir() {
        bail!("Overlay '{name}' not found in library");
    }
    fs::remove_dir_all(&overlay_path)?;
    Ok(())
}

/// Export (copy) an overlay from the library to a destination.
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
pub(crate) fn check_library_gitignored(repo_root: &Path, library_path: &Path) -> bool {
    std::process::Command::new("git")
        .args(["check-ignore", "-q"])
        .arg(library_path)
        .current_dir(repo_root)
        .status()
        .map(|s| s.success()) // exit 0 means the path IS ignored
        .unwrap_or(false)
}

/// Ensure the library path is not gitignored.
///
/// If the library path is currently gitignored, appends a negation pattern
/// (e.g. `!.repoverlay/library/`) to the repo's `.gitignore` file so that
/// library overlays are tracked by git. Returns `true` if the `.gitignore`
/// was modified.
pub(crate) fn ensure_library_not_gitignored(repo_root: &Path, library_path: &Path) -> Result<bool> {
    if !check_library_gitignored(repo_root, library_path) {
        return Ok(false);
    }

    let relative_library = library_path.strip_prefix(repo_root).unwrap_or(library_path);
    let mut lib_pattern = relative_library.to_string_lossy().to_string();
    if !lib_pattern.ends_with('/') {
        lib_pattern.push('/');
    }
    let negation = format!("!{lib_pattern}");

    let gitignore_path = repo_root.join(".gitignore");
    let existing = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)?
    } else {
        String::new()
    };

    // Git ignores an entire directory with `dir/`, which prevents `!dir/child/` from
    // un-ignoring. We need to convert `dir/` to `dir/*` (ignore contents, not the
    // directory itself) so the negation pattern can take effect.
    let parent_dir = relative_library
        .parent()
        .map(|p| p.to_string_lossy().to_string());

    let mut lines: Vec<String> = existing.lines().map(String::from).collect();
    let mut modified = false;

    if let Some(ref parent) = parent_dir {
        let dir_pattern = format!("{parent}/");
        let star_pattern = format!("{parent}/*");

        for line in &mut lines {
            if line.trim() == dir_pattern {
                line.clone_from(&star_pattern);
                modified = true;
            }
        }
    }

    // Append negation if not already present
    if !lines.iter().any(|l| l.trim() == negation) {
        lines.push(negation);
        modified = true;
    }

    if modified {
        let mut content = lines.join("\n");
        if !content.ends_with('\n') {
            content.push('\n');
        }
        fs::write(&gitignore_path, content)?;
    }

    Ok(modified)
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

    fn init_git_repo(path: &Path) {
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .status()
            .unwrap();
    }

    #[test]
    fn ensure_not_gitignored_adds_negation() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());

        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(&library_path).unwrap();
        fs::write(tmp.path().join(".gitignore"), ".repoverlay/\n").unwrap();

        assert!(check_library_gitignored(tmp.path(), &library_path));

        let modified = ensure_library_not_gitignored(tmp.path(), &library_path).unwrap();
        assert!(modified);

        // Library should no longer be gitignored
        assert!(!check_library_gitignored(tmp.path(), &library_path));

        // .gitignore should contain the negation
        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains("!.repoverlay/library/"));
    }

    #[test]
    fn ensure_not_gitignored_noop_when_not_ignored() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());

        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(&library_path).unwrap();

        let modified = ensure_library_not_gitignored(tmp.path(), &library_path).unwrap();
        assert!(!modified);
    }

    #[test]
    fn ensure_not_gitignored_idempotent() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());

        let library_path = tmp.path().join(".repoverlay").join("library");
        fs::create_dir_all(&library_path).unwrap();
        fs::write(tmp.path().join(".gitignore"), ".repoverlay/\n").unwrap();

        ensure_library_not_gitignored(tmp.path(), &library_path).unwrap();
        // Second call should be a noop since library is no longer ignored
        let modified = ensure_library_not_gitignored(tmp.path(), &library_path).unwrap();
        assert!(!modified);

        // Should only have one negation line
        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        let count = content
            .lines()
            .filter(|l| l.contains("!.repoverlay/library/"))
            .count();
        assert_eq!(count, 1);
    }
}
