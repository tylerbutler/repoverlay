//! Filesystem utility functions.
//!
//! Provides atomic write operations and other filesystem helpers.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

/// The kind of filesystem entry a symlink points to.
///
/// Only meaningful on Windows, where file and directory symlinks are distinct
/// system objects; on Unix the distinction is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SymlinkKind {
    File,
    Dir,
}

/// Create a symlink at `link` pointing to `original`, cross-platform.
///
/// Returns the raw `io::Result` so call sites can attach their own context.
pub(crate) fn create_symlink(
    original: &Path,
    link: &Path,
    kind: SymlinkKind,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = kind;
        std::os::unix::fs::symlink(original, link)
    }
    #[cfg(windows)]
    {
        match kind {
            SymlinkKind::File => std::os::windows::fs::symlink_file(original, link),
            SymlinkKind::Dir => std::os::windows::fs::symlink_dir(original, link),
        }
    }
}

/// Write content to a file atomically using write-then-rename.
///
/// Creates a temporary file in the same directory as the target path,
/// writes the content, then atomically renames it into place. This
/// prevents corruption if the process is interrupted mid-write.
///
/// # Errors
///
/// Returns an error if:
/// - The parent directory does not exist
/// - Writing to the temporary file fails
/// - Persisting (renaming) the file fails
pub(crate) fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let dir = path
        .parent()
        .context("Target file has no parent directory")?;
    let mut tmp = NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.persist(path)
        .context("Failed to atomically persist file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_creates_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");

        atomic_write(&file, "test content").unwrap();

        assert!(file.exists());
        let content = fs::read_to_string(&file).unwrap();
        assert_eq!(content, "test content");
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");

        fs::write(&file, "old content").unwrap();
        atomic_write(&file, "new content").unwrap();

        let content = fs::read_to_string(&file).unwrap();
        assert_eq!(content, "new content");
    }

    #[test]
    fn test_atomic_write_no_parent_fails() {
        let result = atomic_write(Path::new("/"), "content");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no parent directory")
        );
    }

    #[test]
    fn test_atomic_write_missing_parent_dir_fails() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("missing").join("test.txt");

        let result = atomic_write(&missing, "content");
        assert!(result.is_err());
    }

    #[test]
    fn test_atomic_write_unicode_content() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("unicode.txt");

        let unicode_content = "Hello 世界 🦀 Rust!";
        atomic_write(&file, unicode_content).unwrap();

        let content = fs::read_to_string(&file).unwrap();
        assert_eq!(content, unicode_content);
    }

    #[test]
    fn test_atomic_write_large_content() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("large.txt");

        let large_content = "x".repeat(10_000);
        atomic_write(&file, &large_content).unwrap();

        let content = fs::read_to_string(&file).unwrap();
        assert_eq!(content, large_content);
    }
}
