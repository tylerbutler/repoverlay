//! Path safety validation for repoverlay.
//!
//! Provides comprehensive validation for paths from overlay configurations
//! and state files to prevent directory traversal, symlink attacks, and
//! path injection vulnerabilities.

use anyhow::{Context, Result, bail};
use std::io::ErrorKind;
use std::path::{Component, Path};

/// Validate that a path is safe for use within a repository.
///
/// This function performs comprehensive safety checks to prevent:
/// - Absolute paths
/// - Parent directory traversal (`..`)
/// - Current directory references (`.`)
/// - Empty path components
/// - Control characters in paths
///
/// The path may not exist yet; this validates the path string itself.
///
/// # Errors
///
/// Returns an error if the path fails any safety check.
///
/// For example, `src/config.rs` is accepted, while `/etc/passwd`,
/// `../../../etc/passwd`, and empty paths are rejected.
pub(crate) fn validate_repo_relative_path(path: &Path) -> Result<()> {
    // Reject empty paths
    if path.as_os_str().is_empty() {
        bail!("Path cannot be empty");
    }

    let raw_path = path.as_os_str().to_string_lossy();
    if raw_path.ends_with('/') || raw_path.ends_with('\\') {
        bail!("Path contains empty component: {}", path.display());
    }
    if raw_path.contains("//") || raw_path.contains("\\\\") {
        bail!("Path contains empty component: {}", path.display());
    }

    // Check for absolute paths
    if path.is_absolute() {
        bail!("Absolute paths are not allowed: {}", path.display());
    }

    // Validate each component
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part_str = part.to_string_lossy();

                // Check for empty components (shouldn't happen, but be defensive)
                if part_str.is_empty() {
                    bail!("Path contains empty component: {}", path.display());
                }

                // Check for control characters
                if part_str.chars().any(char::is_control) {
                    bail!("Path contains control characters: {}", path.display());
                }
            }
            Component::ParentDir => {
                bail!(
                    "Path contains parent directory reference (..): {}",
                    path.display()
                );
            }
            Component::CurDir => {
                bail!(
                    "Path contains current directory reference (.): {}",
                    path.display()
                );
            }
            Component::Prefix(_) | Component::RootDir => {
                bail!("Path must be relative: {}", path.display());
            }
        }
    }

    Ok(())
}

/// Check if a path or any of its ancestors is a symlink.
///
/// This walks from the repo root up to the target path (or its nearest
/// existing parent) and checks each component using `symlink_metadata`
/// to detect symlinks.
///
/// This is important for preventing symlink attacks where an attacker
/// places a symlink in the path to redirect writes outside the repo.
///
/// # Arguments
///
/// * `repo_root` - The canonical repository root
/// * `target` - The target path (relative to `repo_root`; may not exist yet)
///
/// # Returns
///
/// Returns `Ok(())` if no symlinks are found, or an error if any ancestor is a symlink.
///
/// # Errors
///
/// Returns an error if:
/// - `repo_root` is not a directory
/// - Any existing ancestor of `target` is a symlink
/// - I/O errors occur during traversal
pub(crate) fn check_no_symlink_ancestors(repo_root: &Path, target: &Path) -> Result<()> {
    validate_repo_relative_path(target)?;

    // Ensure repo_root exists and is a directory
    if !repo_root.is_dir() {
        bail!(
            "Repository root is not a directory: {}",
            repo_root.display()
        );
    }

    // Build the full target path
    let full_target = repo_root.join(target);

    // Find the nearest existing ancestor
    let mut current = full_target.as_path();

    while !path_exists_or_symlink(current)? {
        if let Some(parent) = current.parent() {
            current = parent;
        } else {
            // Reached root without finding existing path
            bail!(
                "Could not find existing ancestor for path: {}",
                target.display()
            );
        }
    }

    // Walk from repo_root to the existing ancestor, checking each component
    let relative = current
        .strip_prefix(repo_root)
        .context("Target is not within repository")?;

    let mut current_path = repo_root.to_path_buf();

    // Check repo_root itself
    let metadata = std::fs::symlink_metadata(&current_path)?;
    if metadata.is_symlink() {
        bail!("Repository root is a symlink: {}", current_path.display());
    }

    // Check each component
    for component in relative.components() {
        current_path.push(component);

        if let Some(metadata) = symlink_metadata_if_present(&current_path)?
            && metadata.is_symlink()
        {
            bail!(
                "Path contains symlink ancestor: {} is a symlink",
                current_path.display()
            );
        }
    }

    Ok(())
}

fn path_exists_or_symlink(path: &Path) -> Result<bool> {
    Ok(symlink_metadata_if_present(path)?.is_some())
}

fn symlink_metadata_if_present(path: &Path) -> Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("Failed to inspect path: {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_validate_repo_relative_path_valid() {
        assert!(validate_repo_relative_path(Path::new("src/config.rs")).is_ok());
        assert!(validate_repo_relative_path(Path::new("file.txt")).is_ok());
        assert!(validate_repo_relative_path(Path::new("a/b/c/d.txt")).is_ok());
        assert!(validate_repo_relative_path(Path::new("my-file")).is_ok());
        assert!(validate_repo_relative_path(Path::new("name_with_underscore.rs")).is_ok());
    }

    #[test]
    fn test_validate_repo_relative_path_absolute() {
        let result = validate_repo_relative_path(Path::new("/etc/passwd"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Absolute"));
    }

    #[test]
    fn test_validate_repo_relative_path_parent_dir() {
        let result = validate_repo_relative_path(Path::new("../etc/passwd"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(".."));

        let result = validate_repo_relative_path(Path::new("src/../../etc/passwd"));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_repo_relative_path_current_dir() {
        let result = validate_repo_relative_path(Path::new("./src/file.txt"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains('.'));
    }

    #[test]
    fn test_validate_repo_relative_path_empty() {
        let result = validate_repo_relative_path(Path::new(""));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_repo_relative_path_empty_components() {
        let result = validate_repo_relative_path(Path::new("a//b"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));

        let result = validate_repo_relative_path(Path::new("a/"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_repo_relative_path_control_chars() {
        // Path with null byte
        let path_with_null = PathBuf::from("file\0name.txt");
        let result = validate_repo_relative_path(&path_with_null);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("control"));
    }

    #[test]
    fn test_check_no_symlink_ancestors_valid() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();

        // Create nested directory structure
        let nested = repo_root.join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();

        // Check existing path
        assert!(check_no_symlink_ancestors(repo_root, Path::new("a/b/c")).is_ok());

        // Check non-existing leaf (parent exists)
        assert!(check_no_symlink_ancestors(repo_root, Path::new("a/b/c/file.txt")).is_ok());
    }

    #[test]
    fn test_check_no_symlink_ancestors_with_symlink() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();

        // Create target directory
        let target = repo_root.join("target");
        fs::create_dir(&target).unwrap();

        // Create symlink
        let link = repo_root.join("link");
        symlink(&target, &link).unwrap();

        // Check path through symlink
        let result = check_no_symlink_ancestors(repo_root, Path::new("link/file.txt"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("symlink"));
    }

    #[test]
    fn test_check_no_symlink_ancestors_with_broken_symlink() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();

        let missing_target = repo_root.join("missing-target");
        let link = repo_root.join("link");
        symlink(&missing_target, &link).unwrap();

        let result = check_no_symlink_ancestors(repo_root, Path::new("link/file.txt"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("symlink"));
    }

    #[test]
    fn test_check_no_symlink_ancestors_nested_symlink() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();

        // Create nested structure: a/target/
        let nested = repo_root.join("a").join("target");
        fs::create_dir_all(&nested).unwrap();

        // Create symlink at a/link -> target
        let link = repo_root.join("a").join("link");
        symlink(&nested, &link).unwrap();

        // Check path through nested symlink
        let result = check_no_symlink_ancestors(repo_root, Path::new("a/link/file.txt"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("symlink"));
    }

    #[test]
    fn test_check_no_symlink_ancestors_non_existent_parent() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();

        // Create only first level
        let first = repo_root.join("a");
        fs::create_dir(&first).unwrap();

        // Check path where middle components don't exist yet
        assert!(check_no_symlink_ancestors(repo_root, Path::new("a/b/c/d.txt")).is_ok());
    }

    #[test]
    fn test_check_no_symlink_ancestors_invalid_repo_root() {
        let dir = TempDir::new().unwrap();
        let fake_root = dir.path().join("nonexistent");

        let result = check_no_symlink_ancestors(&fake_root, Path::new("file.txt"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a directory"));
    }
}
