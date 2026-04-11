# Overlay Library Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add in-repo overlay library storage (`.repoverlay/library/`) with import/export/move commands and auto-registration as an overlay source.

**Architecture:** New `OverlaySource::Library` variant for state tracking. Library is discovered at runtime from the working directory and registered as a highest-priority implicit source (`@library`). New `library` subcommand group and top-level `move` command. Library path is configurable via per-repo `repoverlay.ccl`.

**Tech Stack:** Rust, clap (CLI), CCL via sickle (config/state format)

**Spec:** [docs/specs/2026-03-15-overlay-library-design.md](../specs/2026-03-15-overlay-library-design.md)

---

## File Structure

### New Files
- `src/library.rs` — Library management: path resolution, listing, import, export, remove, gitignore warning detection
- Tests embedded in `src/library.rs` (unit tests) and `tests/cli.rs` (integration/CLI tests)

### Modified Files
- `src/lib.rs` — Add `mod library;` declaration, integrate library resolution into `resolve_source()`
- `src/state.rs` — Add `OverlaySource::Library` variant, update `SourceResolver` impl, serialization/deserialization
- `src/cli.rs` — Add `Library` and `Move` subcommands, wire up to library functions
- `src/config.rs` — Add `library_path` field to per-repo config parsing
- `src/reference.rs` — No changes needed (bare names already parse as `OnePart`, library resolution happens in `resolve_source()`)
- `src/sources.rs` — Add `@library` name reservation validation in source add

---

## Chunk 1: State Layer — `OverlaySource::Library` Variant

### Task 1: Add `OverlaySource::Library` variant to state.rs

**Files:**
- Modify: `src/state.rs:36-79` (OverlaySource enum)
- Modify: `src/state.rs:81-205` (OverlaySource impl block)

- [ ] **Step 1: Write failing test for Library variant construction**

```rust
#[test]
fn library_source_construction() {
    let source = OverlaySource::library("claude-config".to_string());
    assert!(source.is_library());
    if let OverlaySource::Library { name } = &source {
        assert_eq!(name, "claude-config");
    } else {
        panic!("Expected Library variant");
    }
}
```

Add this test in the existing `#[cfg(test)] mod tests` block in `state.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test library_source_construction -- --nocapture`
Expected: FAIL — `OverlaySource::Library` doesn't exist yet.

- [ ] **Step 3: Add Library variant to OverlaySource enum**

In `src/state.rs`, add the new variant to the `OverlaySource` enum (after `OverlayRepo`):

```rust
/// Overlay from the in-repo library (.repoverlay/library/)
Library {
    /// Overlay name within the library directory
    name: String,
},
```

- [ ] **Step 4: Add constructor and query methods**

In the `impl OverlaySource` block, add:

```rust
/// Create a new library source.
pub(crate) fn library(name: String) -> Self {
    Self::Library { name }
}

/// Check if this is a library source.
pub(crate) const fn is_library(&self) -> bool {
    matches!(self, Self::Library { .. })
}

/// Get the library overlay name (for library sources only).
pub(crate) fn library_name(&self) -> Option<&str> {
    match self {
        Self::Library { name } => Some(name),
        _ => None,
    }
}
```

- [ ] **Step 5: Update the `display()` method**

Add a match arm for `Library` in the `display()` method:

```rust
Self::Library { name } => format!("{name} (library)"),
```

- [ ] **Step 6: Update `local_path()` to handle Library**

Add match arm in `local_path()`:

```rust
Self::Library { .. } => None, // Resolved at runtime via library path config
```

- [ ] **Step 7: Fix exhaustiveness — update all match expressions on OverlaySource**

The compiler will flag every incomplete match. Add `Self::Library { .. }` arms to:
- `is_github()` — return `false`
- `is_overlay_repo()` — return `false`
- Any other match expressions in `state.rs`

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test library_source_construction -- --nocapture`
Expected: PASS

- [ ] **Step 9: Commit**

```bash
git add src/state.rs
git commit -m "feat(state): add OverlaySource::Library variant

Add new Library variant to OverlaySource enum for tracking overlays
applied from the in-repo library. Includes constructor, query methods,
and display formatting."
```

### Task 2: Implement SourceResolver for Library variant

**Files:**
- Modify: `src/state.rs` (SourceResolver trait impl)

- [ ] **Step 1: Write failing test for SourceResolver on Library**

```rust
#[test]
fn library_source_resolver_is_mutable() {
    let source = OverlaySource::library("test-overlay".to_string());
    // Library sources are mutable (files live in the repo)
    assert!(source.is_mutable());
}

#[test]
fn library_source_resolver_not_syncable() {
    let source = OverlaySource::library("test-overlay".to_string());
    // Library sources can't be synced (no remote concept)
    assert!(!source.is_syncable());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test library_source_resolver -- --nocapture`
Expected: FAIL — incomplete match in SourceResolver impl

- [ ] **Step 3: Add Library arms to SourceResolver implementation**

Find the `impl SourceResolver for OverlaySource` block and add `Library` match arms:

In `resolve_local_path()`:
```rust
Self::Library { name } => {
    // Library path resolution requires repo context — this is handled
    // by the caller passing the resolved library path. Return an error
    // here since Library sources need external context to resolve.
    Err(anyhow::anyhow!(
        "Library source '{name}' requires repo context to resolve. Use library::resolve_library_overlay_path() instead."
    ))
}
```

In `is_mutable()`:
```rust
Self::Library { .. } => true,
```

In `is_syncable()`:
```rust
Self::Library { .. } => false,
```

In `is_updatable()`:
```rust
Self::Library { .. } => false,
```

In `source_type_label()`:
```rust
Self::Library { .. } => "library",
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test library_source_resolver -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run full test suite to check for exhaustiveness**

Run: `cargo test`
Expected: PASS (or fix any remaining exhaustiveness errors flagged by the compiler)

- [ ] **Step 6: Commit**

```bash
git add src/state.rs
git commit -m "feat(state): implement SourceResolver for Library variant

Library sources are mutable but not syncable. Path resolution requires
repo context and is delegated to the library module."
```

### Task 3: Implement Library source serialization/deserialization

**Files:**
- Modify: `src/state.rs` (CCL serialization)

- [ ] **Step 1: Write failing test for serialization round-trip**

Find the existing serialization tests (search for `serialize` or `to_ccl` in state.rs tests) and add:

```rust
#[test]
fn overlay_state_round_trip_library_source() {
    let state = OverlayState {
        name: OverlayName::new("test-overlay".to_string()),
        applied_at: Utc::now(),
        source: OverlaySource::library("test-overlay".to_string()),
        files: vec![FileEntry {
            source: PathBuf::from(".envrc"),
            target: PathBuf::from(".envrc"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::File,
        }],
        removed_at: None,
    };

    let serialized = sickle::to_string(&state).unwrap();
    let deserialized: OverlayState = sickle::from_str(&serialized).unwrap();

    assert!(deserialized.source.is_library());
    assert_eq!(deserialized.source.library_name(), Some("test-overlay"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test overlay_state_round_trip_library_source -- --nocapture`
Expected: FAIL — serialization doesn't handle Library variant yet.

- [ ] **Step 3: Verify serialization works via serde/sickle**

`OverlaySource` uses `#[derive(Serialize, Deserialize)]` with `#[serde(tag = "type")]`. Adding the `Library` variant should auto-serialize via sickle as `type = Library` with a `name` field. The existing serde machinery should handle this — verify by running the round-trip test. If sickle has issues with the new variant, check how the existing variants serialize and adjust the `#[serde]` attributes accordingly.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test overlay_state_round_trip_library_source -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/state.rs
git commit -m "feat(state): add CCL serialization for Library source

Serialized as 'library|<name>' in CCL state files. Supports full
round-trip serialization/deserialization."
```

---

## Chunk 2: Library Module — Core Operations

### Task 4: Create library module with path resolution

**Files:**
- Create: `src/library.rs`
- Modify: `src/lib.rs` (add `mod library;`)

- [ ] **Step 1: Write failing tests for library path resolution**

Create `src/library.rs` with tests:

```rust
//! In-repo overlay library management.
//!
//! Handles the `.repoverlay/library/` directory for storing shareable overlays
//! within a repository. The library is auto-discovered and registered as an
//! implicit source with highest priority.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

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
            // Must be relative
            if path.is_absolute() {
                bail!("Library path must be relative, got: {}", path.display());
            }
            // Must not escape repo root (reject any ParentDir components)
            if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                bail!("Library path must be within the repository root: {}", path.display());
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
pub(crate) fn library_exists(repo_root: &Path, config_path: Option<&str>) -> bool {
    resolve_library_path(repo_root, config_path)
        .map(|p| p.is_dir())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

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
    fn library_overlay_path_resolution() {
        let tmp = TempDir::new().unwrap();
        let path = resolve_library_overlay_path(tmp.path(), None, "claude-config").unwrap();
        assert_eq!(
            path,
            tmp.path().join(".repoverlay").join("library").join("claude-config")
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
}
```

- [ ] **Step 2: Add module declaration to lib.rs**

In `src/lib.rs`, add after the existing module declarations:

```rust
mod library;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test library:: -- --nocapture`
Expected: PASS (these are self-contained unit tests)

- [ ] **Step 4: Commit**

```bash
git add src/library.rs src/lib.rs
git commit -m "feat(library): add library module with path resolution

Core library path resolution with configurable paths, validation
(must be relative, within repo root), and existence checking."
```

### Task 5: Add library listing and overlay discovery

**Files:**
- Modify: `src/library.rs`

- [ ] **Step 1: Write failing test for listing overlays**

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test list_overlays -- --nocapture`
Expected: FAIL — `list_library_overlays` doesn't exist.

- [ ] **Step 3: Implement listing**

```rust
/// An overlay found in the library.
#[derive(Debug, Clone)]
pub(crate) struct LibraryOverlay {
    /// Overlay name (directory name)
    pub(crate) name: String,
    /// Full path to the overlay directory
    pub(crate) path: PathBuf,
}

/// List all overlays in the library directory.
pub(crate) fn list_library_overlays(library_path: &Path) -> Result<Vec<LibraryOverlay>> {
    if !library_path.is_dir() {
        return Ok(Vec::new());
    }

    let mut overlays = Vec::new();
    for entry in std::fs::read_dir(library_path)? {
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test list_overlays -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/library.rs
git commit -m "feat(library): add overlay listing

Lists overlay directories in the library, sorted by name. Ignores
files at the library root."
```

### Task 6: Add library import (copy overlay into library)

**Files:**
- Modify: `src/library.rs`

- [ ] **Step 1: Write failing test for import**

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test import_overlay -- --nocapture`
Expected: FAIL

- [ ] **Step 3: Implement import**

```rust
use crate::overlay_repo::copy_dir_recursive;

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

    // Create library directory if needed
    fs::create_dir_all(library_path)?;

    // Copy overlay directory
    copy_dir_recursive(source_path, &dest)?;

    Ok(dest)
}
```

Note: `copy_dir_recursive` is already available from `crate::overlay_repo`. Verify this is `pub(crate)` — if not, update its visibility.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test import_overlay -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/library.rs
git commit -m "feat(library): add import operation

Copy overlay directories into the library with name conflict detection
and --force support. Auto-creates library directory on first import."
```

### Task 7: Add library remove

**Files:**
- Modify: `src/library.rs`

- [ ] **Step 1: Write failing test for remove**

```rust
#[test]
fn remove_from_library() {
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test remove_from_library -- --nocapture`
Expected: FAIL

- [ ] **Step 3: Implement remove**

```rust
/// Remove an overlay from the library.
pub(crate) fn remove_from_library(library_path: &Path, name: &str) -> Result<()> {
    let overlay_path = library_path.join(name);
    if !overlay_path.is_dir() {
        bail!("Overlay '{}' not found in library", name);
    }
    fs::remove_dir_all(&overlay_path)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test remove_from_library -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/library.rs
git commit -m "feat(library): add remove operation

Remove overlay directories from the library with existence validation."
```

### Task 8: Add library export

**Files:**
- Modify: `src/library.rs`

- [ ] **Step 1: Write failing test for export**

```rust
#[test]
fn export_from_library() {
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test export_from_library -- --nocapture`
Expected: FAIL

- [ ] **Step 3: Implement export**

```rust
/// Export (copy) an overlay from the library to a destination.
pub(crate) fn export_from_library(
    library_path: &Path,
    name: &str,
    dest: &Path,
) -> Result<PathBuf> {
    let source = library_path.join(name);
    if !source.is_dir() {
        bail!("Overlay '{}' not found in library", name);
    }

    let target = dest.join(name);
    if target.exists() {
        bail!(
            "Destination already exists: {}. Use --force to overwrite.",
            target.display()
        );
    }

    copy_dir_recursive(&source, &target)?;
    Ok(target)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test export_from_library -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/library.rs
git commit -m "feat(library): add export operation

Copy overlay from library to destination path. Export is non-destructive
(overlay remains in library)."
```

---

## Chunk 3: Configuration and Source Integration

### Task 9: Add library path to per-repo config

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Identify per-repo config structure**

The per-repo config is loaded via `config::load_repo_config()`. Examine the existing per-repo config struct (likely `RepoverlayConfig` or a separate struct) in `src/config.rs`. The global config uses `RepoverlayConfig` with a `sources: Vec<Source>` field. Per-repo config may use the same or a different struct — check `load_repo_config()` to understand the type.

- [ ] **Step 2: Write failing test for library config parsing**

Add a test for the library path field. The exact test depends on the config struct:

```rust
#[test]
fn repo_config_with_library_path() {
    // Adapt to match actual config struct and parsing approach
    let ccl = "library\n  path = .overlays\n";
    let config: RepoConfig = sickle::from_str(ccl).unwrap();
    assert_eq!(config.library_path.as_deref(), Some(".overlays"));
}
```

Note: Use the actual struct name and verify sickle can parse nested CCL groups into struct fields.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test repo_config_with_library -- --nocapture`
Expected: FAIL

- [ ] **Step 4: Add library_path field to per-repo config**

Add a `library_path: Option<String>` (or a nested `LibraryConfig` struct with a `path` field) to the per-repo config struct. Use `#[serde(default)]` to make it optional. Also add a `load_repo_config()` function or method that returns the library path if present.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test parse_repo_config -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add library.path to per-repo config

Configurable library path via .repoverlay/repoverlay.ccl. Falls back
to default (.repoverlay/library/) when not set."
```

### Task 10: Add @library source name reservation

**Files:**
- Modify: `src/sources.rs` or `src/cli.rs` (wherever `source add` validation happens)

- [ ] **Step 1: Write failing test for @ prefix rejection**

```rust
#[test]
fn source_add_rejects_at_prefix() {
    // Test that source names starting with @ are rejected
    let result = validate_source_name("@library");
    assert!(result.is_err());

    let result = validate_source_name("@anything");
    assert!(result.is_err());

    // Normal names should be fine
    let result = validate_source_name("my-source");
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test source_add_rejects_at_prefix -- --nocapture`
Expected: FAIL

- [ ] **Step 3: Add validation**

Add a validation function (or extend existing validation) in the appropriate module:

```rust
/// Validate a source name.
///
/// Names starting with `@` are reserved for built-in sources.
pub(crate) fn validate_source_name(name: &str) -> Result<()> {
    if name.starts_with('@') {
        bail!("Source names starting with '@' are reserved. '@library' is a built-in source.");
    }
    Ok(())
}
```

Wire this into the `source add` command handler.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test source_add_rejects_at_prefix -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/sources.rs src/cli.rs
git commit -m "feat(sources): reserve @ prefix for built-in source names

Reject source names starting with '@' during 'source add'. The
@library name is reserved for the in-repo library source."
```

### Task 11: Integrate library into resolve_source()

**Files:**
- Modify: `src/lib.rs:379-485` (resolve_source function)

- [ ] **Step 1: Write failing integration test**

In `tests/cli.rs` or as an integration test:

```rust
#[test]
fn apply_resolves_from_library() {
    let (repo, _guard) = create_test_repo();
    let repo_path = repo.path();

    // Create a library overlay
    let library_path = repo_path.join(".repoverlay").join("library").join("test-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("test-file.txt"), "from library").unwrap();

    // Apply by bare name — should resolve from library
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["apply", "test-overlay", "--target", repo_path.to_str().unwrap()])
        .assert()
        .success();

    // Verify file was applied
    assert!(repo_path.join("test-file.txt").exists());

    // Verify state shows library source
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["status", "--json", "--target", repo_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("library"));
}
```

Note: Adapt this to match the existing test patterns in `tests/cli.rs` — use the same repo setup helpers.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test apply_resolves_from_library -- --nocapture`
Expected: FAIL — library isn't checked during resolution yet.

- [ ] **Step 3: Add library resolution to resolve_source()**

In `src/lib.rs`, in the `resolve_source()` function, add library lookup **before** the existing `SourceReference::parse()` call. For `OnePart` references (bare names like `my-overlay`), check the library first:

```rust
// Check library first for bare names (no slashes) or explicit @library source filter
let check_library = !source_str.contains('/') || source_filter == Some(library::LIBRARY_SOURCE_NAME);
if check_library {
    if let Some(target) = target_path {
        let repo_config = config::load_repo_config(target);
        let config_path = repo_config.as_ref().ok().and_then(|c| c.library_path.as_deref());
        let lookup_name = if source_str.contains('/') { source_str } else { source_str };
        if let Ok(overlay_path) = library::resolve_library_overlay_path(target, config_path, lookup_name) {
            if overlay_path.is_dir() {
                debug!("resolved '{source_str}' from library at {}", overlay_path.display());
                return Ok(ResolvedSources::Single(ResolvedSource {
                    path: overlay_path,
                    source_info: OverlaySource::library(source_str.to_string()),
                }));
            }
        }
    }
}
```

Note: `ResolvedSource` has fields `path: PathBuf` and `source_info: OverlaySource` (see `src/lib.rs:273-278`). Check the struct definition for any additional required fields and fill them in.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test apply_resolves_from_library -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs
git commit -m "feat: integrate library into overlay resolution

Bare overlay names are resolved from the library before checking
configured sources. Supports --from @library to explicitly target
the library source."
```

---

## Chunk 4: CLI Commands

### Task 12: Add `library` subcommand to CLI

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 1: Add Library subcommand enum variants**

In the `Commands` enum in `src/cli.rs`, add:

```rust
/// Manage the in-repo overlay library
Library {
    #[command(subcommand)]
    command: LibraryCommand,
},
```

Add the `LibraryCommand` enum:

```rust
#[derive(Subcommand)]
enum LibraryCommand {
    /// List overlays in the library
    List {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Import an overlay into the library
    Import {
        /// Overlay source (path, GitHub URL, applied name, or org/repo/name)
        source: String,

        /// Name for the imported overlay (defaults to source name)
        #[arg(long)]
        name: Option<String>,

        /// Force overwrite if overlay already exists
        #[arg(short, long)]
        force: bool,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Export an overlay from the library
    Export {
        /// Name of the overlay to export
        overlay: String,

        /// Destination path or source:<name>
        #[arg(long = "to")]
        dest: String,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },

    /// Remove an overlay from the library
    Remove {
        /// Name of the overlay to remove
        overlay: String,

        /// Force removal even if overlay is currently applied
        #[arg(short, long)]
        force: bool,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },
}
```

- [ ] **Step 2: Add command dispatch in the run() function**

In the main `match` on `Commands`, add:

```rust
Some(Commands::Library { command }) => {
    match command {
        LibraryCommand::List { target } => {
            let target_dir = target.unwrap_or_else(|| PathBuf::from("."));
            let target = canonicalize_path(&target_dir, "Target")?;
            validate_git_repo(&target)?;
            let repo_config = config::load_repo_config(&target);
            let config_path = repo_config.as_ref().ok().and_then(|c| c.library_path.as_deref());
            let library_path = library::resolve_library_path(&target, config_path)?;
            let overlays = library::list_library_overlays(&library_path)?;
            if overlays.is_empty() {
                println!("No overlays in library.");
            } else {
                for overlay in &overlays {
                    println!("  {}", overlay.name);
                }
            }
        }
        LibraryCommand::Import { source, name, force, target } => {
            bail!("library import not yet implemented");
        }
        LibraryCommand::Export { overlay, dest, target } => {
            bail!("library export not yet implemented");
        }
        LibraryCommand::Remove { overlay, force, target } => {
            bail!("library remove not yet implemented");
        }
    }
}
```

Note: `canonicalize_path()` takes `(&Path, &str)` — see `src/lib.rs:213`. Import/export/remove use `bail!()` as placeholders instead of `todo!()` to produce a user-friendly error rather than a panic.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: Compiles (bail!() placeholders are fine for now)

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs
git commit -m "feat(cli): add library subcommand structure

Add library list/import/export/remove subcommands with argument
definitions. List is fully implemented; import, export, and remove
return errors pending implementation in subsequent tasks."
```

### Task 13: Wire up `library list` CLI test

**Files:**
- Modify: `tests/cli.rs`

- [ ] **Step 1: Write CLI test for library list**

```rust
#[test]
fn library_list_empty() {
    let (repo, _guard) = create_test_repo();
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["library", "list", "--target", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("No overlays"));
}

#[test]
fn library_list_shows_overlays() {
    let (repo, _guard) = create_test_repo();
    let library_path = repo.path().join(".repoverlay").join("library");
    fs::create_dir_all(library_path.join("overlay-a")).unwrap();
    fs::create_dir_all(library_path.join("overlay-b")).unwrap();

    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["library", "list", "--target", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("overlay-a"))
        .stdout(predicates::str::contains("overlay-b"));
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test library_list -- --nocapture`
Expected: PASS (list is already implemented)

- [ ] **Step 3: Commit**

```bash
git add tests/cli.rs
git commit -m "test(cli): add library list integration tests

Tests empty library and populated library list output."
```

### Task 14: Wire up `library import` command

**Files:**
- Modify: `src/cli.rs` (replace bail!() in Import handler)

- [ ] **Step 1: Write CLI test for library import**

```rust
#[test]
fn library_import_from_local_path() {
    let (repo, _guard) = create_test_repo();
    let (overlay_path, _overlay_guard) = create_test_overlay("import-test");

    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "library", "import",
            overlay_path.to_str().unwrap(),
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify overlay is in library
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["library", "list", "--target", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("import-test"));
}
```

Note: Adapt `create_test_overlay` usage to match existing test helpers.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test library_import_from_local_path -- --nocapture`
Expected: FAIL (todo!() panics)

- [ ] **Step 3: Implement import command handler**

Replace the `bail!()` in the Import match arm with the actual implementation:
- Resolve the source (local path, GitHub URL, or applied overlay name)
- Determine overlay name (from --name flag, or infer from source directory name)
- Call `library::import_to_library()`
- Print success message

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test library_import_from_local_path -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat(cli): wire up library import command

Resolves source references and copies overlays into the library.
Supports local paths, GitHub URLs, and applied overlay names."
```

### Task 15: Wire up `library export` and `library remove` commands

**Files:**
- Modify: `src/cli.rs` (replace bail!() in Export and Remove handlers)

- [ ] **Step 1: Write CLI tests**

```rust
#[test]
fn library_export_to_path() {
    let (repo, _guard) = create_test_repo();
    let library_path = repo.path().join(".repoverlay").join("library").join("my-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("file.txt"), "content").unwrap();

    let dest = repo.path().join("exported");
    fs::create_dir_all(&dest).unwrap();

    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "library", "export", "my-overlay",
            "--to", dest.to_str().unwrap(),
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(dest.join("my-overlay").join("file.txt").exists());
}

#[test]
fn library_remove_overlay() {
    let (repo, _guard) = create_test_repo();
    let library_path = repo.path().join(".repoverlay").join("library").join("my-overlay");
    fs::create_dir_all(&library_path).unwrap();

    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "library", "remove", "my-overlay",
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(!library_path.exists());
}
```

- [ ] **Step 2: Implement export and remove handlers**

Replace the `bail!()` in both match arms with implementations that:
- **Export**: resolve destination, call `library::export_from_library()`
- **Remove**: check if applied (load overlay states, look for Library source matching name), block without --force, call `library::remove_from_library()`

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test library_export library_remove -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat(cli): wire up library export and remove commands

Export copies overlays to filesystem destinations. Remove checks for
applied overlays and blocks without --force."
```

### Task 16: Add `move` command

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/library.rs` (add move helper)

- [ ] **Step 1: Add Move to Commands enum**

```rust
/// Move an overlay to a different location
Move {
    /// Overlay name (applied overlay or path)
    overlay: String,

    /// Destination: 'library', 'source:<name>', or a filesystem path
    #[arg(long = "to")]
    dest: String,

    /// Target repository directory (defaults to current directory)
    #[arg(short, long)]
    target: Option<PathBuf>,

    /// Force overwrite at destination
    #[arg(short, long)]
    force: bool,
},
```

- [ ] **Step 2: Write CLI test for move to library**

```rust
#[test]
fn move_local_overlay_to_library() {
    let (repo, _guard) = create_test_repo();
    let (overlay_path, _overlay_guard) = create_test_overlay("move-test");

    // Apply from local path first
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "apply", overlay_path.to_str().unwrap(),
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Move to library
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "move", "move-test",
            "--to", "library",
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify overlay is in library
    assert!(repo.path().join(".repoverlay").join("library").join("move-test").exists());

    // Verify state was updated to Library source
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["status", "--json", "--target", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("library"));
}
```

- [ ] **Step 3: Implement move command handler**

The move operation is:
1. Resolve the overlay's current location (from applied state or path)
2. Copy to destination (using import_to_library for `library` dest, or copy_dir_recursive for paths)
3. Update applied state references (rewrite source field, update external backup)
4. Re-create symlinks if needed (for symlink entries pointing to old location)
5. Delete from source location

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test move_local_overlay_to_library -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs src/library.rs tests/cli.rs
git commit -m "feat(cli): add move command

Move overlays between locations (library, filesystem paths, sources).
Updates applied state references and re-creates symlinks when needed.
Ordering: copy → update state → re-link → delete source."
```

---

## Chunk 5: Create Integration and Status Display

### Task 17: Add `--into library` flag to create command

**Files:**
- Modify: `src/cli.rs` (Create command)

- [ ] **Step 1: Add --into flag to Create command**

In the `Create` variant of `Commands`:

```rust
/// Create overlay directly into the library
#[arg(long)]
into: Option<String>,

/// Skip the auto-apply prompt after creating into library
#[arg(long, requires = "into")]
no_apply: bool,
```

- [ ] **Step 2: Write CLI test**

```rust
#[test]
fn create_into_library() {
    let (repo, _guard) = create_test_repo();
    // Create a test file to include in the overlay
    fs::write(repo.path().join("CLAUDE.md"), "# Test").unwrap();

    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "create",
            "--into", "library",
            "--include", "CLAUDE.md",
            "--yes",
            "--no-apply",
            "--source", repo.path().to_str().unwrap(),
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify overlay was created in library
    let library_path = repo.path().join(".repoverlay").join("library");
    assert!(library_path.exists());
}
```

Note: Adapt to match the actual create command's argument handling.

- [ ] **Step 3: Implement --into library handling**

In the Create command handler, when `into` is `Some("library")`:
- Resolve library path
- Set output directory to `library_path/<name>`
- After creation, prompt to apply (unless `--yes` or `--no-apply`)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test create_into_library -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat(cli): add --into library flag to create command

Create overlays directly into the library. Prompts to apply after
creation (--yes auto-confirms, --no-apply skips)."
```

### Task 18: Update status display for library overlays

**Files:**
- Modify: `src/cli.rs` (status display logic)

- [ ] **Step 1: Write test for status output**

```rust
#[test]
fn status_shows_library_source() {
    let (repo, _guard) = create_test_repo();

    // Create library overlay and apply it
    let library_path = repo.path().join(".repoverlay").join("library").join("test-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("test-file.txt"), "content").unwrap();

    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["apply", "test-overlay", "--target", repo.path().to_str().unwrap()])
        .assert()
        .success();

    // Check status output
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["status", "--target", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("(library)"));
}
```

- [ ] **Step 2: Update status display code**

Find the status display logic and ensure `OverlaySource::Library` is handled in both text and JSON output. The `display()` method on `OverlaySource` already returns `"name (library)"` from Task 1, so this may already work. Verify JSON output includes `"source_type": "library"`.

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test status_shows_library_source -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat(cli): display library source type in status output

Status shows '(library)' for library-sourced overlays in both text
and JSON output modes."
```

### Task 19: Update update command to skip library overlays

**Files:**
- Modify: `src/lib.rs` or `src/cli.rs` (update command logic)

- [ ] **Step 1: Write test for update skipping library overlays**

```rust
#[test]
fn update_skips_library_overlays() {
    let (repo, _guard) = create_test_repo();

    // Create and apply library overlay
    let library_path = repo.path().join(".repoverlay").join("library").join("test-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("file.txt"), "content").unwrap();

    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["apply", "test-overlay", "--target", repo.path().to_str().unwrap()])
        .assert()
        .success();

    // Update should skip library overlays with a message
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args(["update", "--target", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicates::str::contains("Skipping").or(predicates::str::contains("library")));
}
```

- [ ] **Step 2: Add Library handling to update logic**

In the update command handler, add a check for `OverlaySource::Library`:

```rust
OverlaySource::Library { name } => {
    eprintln!("  Skipping '{}' (library overlay — update via git)", name);
    continue;
}
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test update_skips_library -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs src/cli.rs tests/cli.rs
git commit -m "feat: skip library overlays during update

Library overlays are managed via git, not repoverlay's update
mechanism. Shows a skip message for clarity."
```

---

## Chunk 6: Final Integration and Cleanup

### Task 20: Add gitignore warning detection

**Files:**
- Modify: `src/library.rs`

- [ ] **Step 1: Write test for gitignore warning**

```rust
#[test]
fn warns_when_library_path_gitignored() {
    let tmp = TempDir::new().unwrap();
    // Must be a git repo for `git check-ignore` to work
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
```

- [ ] **Step 2: Implement gitignore check**

```rust
/// Check if the library path would be excluded by gitignore rules.
///
/// Returns true if the library path appears to be gitignored, meaning
/// overlays stored there won't be tracked by git.
pub(crate) fn check_library_gitignored(repo_root: &Path, library_path: &Path) -> bool {
    // Use `git check-ignore` to test if the path would be ignored
    std::process::Command::new("git")
        .args(["check-ignore", "-q"])
        .arg(library_path)
        .current_dir(repo_root)
        .status()
        .map(|s| s.success()) // exit 0 means the path IS ignored
        .unwrap_or(false)
}
```

Integrate this check into `library import` and `create --into library` — print a warning if detected.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test warns_when_library no_warning_when -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/library.rs
git commit -m "feat(library): warn when library path is gitignored

Uses 'git check-ignore' to detect if the library directory would be
excluded from version control. Warns during import and create."
```

### Task 21: Run full test suite and lint

- [ ] **Step 1: Run all tests**

Run: `just test`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `just lint`
Expected: No warnings

- [ ] **Step 3: Run format check**

Run: `just fmt-check`
Expected: No formatting issues

- [ ] **Step 4: Run full check**

Run: `just check`
Expected: All checks pass

- [ ] **Step 5: Commit any remaining fixes**

```bash
git add -A
git commit -m "chore: fix lint and formatting issues"
```

### Task 22: Integrate library into browse command

**Files:**
- Modify: `src/cli.rs` (browse command handler)

- [ ] **Step 1: Write test for browse showing library overlays**

```rust
#[test]
fn browse_includes_library_overlays() {
    let (repo, _guard) = create_test_repo();
    let library_path = repo.path().join(".repoverlay").join("library").join("test-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("file.txt"), "content").unwrap();

    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "browse", "--no-interactive",
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("test-overlay"));
}
```

- [ ] **Step 2: Add library overlays to browse listing**

In the browse command handler, after listing overlays from configured sources, also list overlays from the library. Prefix library overlays with `@library` or a visual indicator to distinguish them.

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test browse_includes_library -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat(cli): include library overlays in browse listing

Browse now shows overlays from the in-repo library alongside
configured source overlays."
```

### Task 23: Add `--from @library` test

**Files:**
- Modify: `tests/cli.rs`

- [ ] **Step 1: Write test for explicit library targeting**

```rust
#[test]
fn apply_from_library_explicit() {
    let (repo, _guard) = create_test_repo();

    // Create a library overlay
    let library_path = repo.path().join(".repoverlay").join("library").join("test-overlay");
    fs::create_dir_all(&library_path).unwrap();
    fs::write(library_path.join("file.txt"), "content").unwrap();

    // Apply with explicit --from @library
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "apply", "test-overlay",
            "--from", "@library",
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(repo.path().join("file.txt").exists());
}
```

- [ ] **Step 2: Verify the test passes**

This should already work from the library resolution in Task 11 which handles `source_filter == Some("@library")`.

Run: `cargo test apply_from_library_explicit -- --nocapture`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/cli.rs
git commit -m "test(cli): add --from @library integration test

Verify explicit library targeting via --from @library flag."
```

### Task 24: Add move symlink re-creation test

**Files:**
- Modify: `tests/cli.rs`

- [ ] **Step 1: Write test verifying symlinks are updated after move**

```rust
#[test]
fn move_to_library_recreates_symlinks() {
    let (repo, _guard) = create_test_repo();
    let (overlay_path, _overlay_guard) = create_test_overlay("symlink-test");

    // Apply from local path (creates symlinks)
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "apply", overlay_path.to_str().unwrap(),
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify symlink points to original location
    let applied_file = repo.path().join("test-file.txt"); // adapt to actual overlay files
    assert!(applied_file.is_symlink());

    // Move to library
    Command::cargo_bin("repoverlay")
        .unwrap()
        .args([
            "move", "symlink-test",
            "--to", "library",
            "--target", repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify symlink now points to library location
    assert!(applied_file.is_symlink());
    let link_target = std::fs::read_link(&applied_file).unwrap();
    assert!(
        link_target.to_string_lossy().contains(".repoverlay/library"),
        "Symlink should point to library: {}",
        link_target.display()
    );
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test move_to_library_recreates_symlinks -- --nocapture`
Expected: PASS (this tests the symlink re-creation from Task 16)

- [ ] **Step 3: Commit**

```bash
git add tests/cli.rs
git commit -m "test(cli): verify symlinks are re-created after move

Ensure move to library updates symlink targets to point to the new
library location instead of the old source."
```
