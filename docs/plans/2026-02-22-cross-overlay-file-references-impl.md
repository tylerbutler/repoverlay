# Cross-Overlay File References Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow mappings and directories in `repoverlay.ccl` to reference files from other overlays using `../` (relative) and `//` (repo-root) prefixes.

**Architecture:** Extend `collect_overlay_files` with a second pass that resolves external mapping keys to absolute paths, validates they're within the overlay repo root, and adds them to the file list. Add a `resolve_external_directory` helper for the directories loop in `apply_resolved_overlay`. Thread an optional `repo_root` parameter through both functions.

**Tech Stack:** Rust 2024, sickle (CCL parser), walkdir, tempfile (tests)

---

### Task 1: Add `resolve_mapping_source` helper function

Add a helper that resolves a mapping key to an absolute source path, handling all three forms (local, relative, repo-root).

**Files:**
- Modify: `src/lib.rs` (insert after `collect_overlay_files` at line ~1787)
- Test: `src/lib.rs` (add to `tests` module)

**Step 1: Write the failing tests**

Add a new test module `resolve_mapping_source_tests` inside the existing `mod tests` block (after `collect_overlay_files_tests` which ends at ~line 7328):

```rust
mod resolve_mapping_source_tests {
    use super::*;

    #[test]
    fn local_key_resolves_to_overlay_dir() {
        let overlay_dir = PathBuf::from("/repo/org/repo/overlay-a");
        let repo_root = PathBuf::from("/repo");
        let result = resolve_mapping_source("file.txt", &overlay_dir, Some(&repo_root));
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/repo/org/repo/overlay-a/file.txt")
        );
    }

    #[test]
    fn relative_key_resolves_to_sibling() {
        let overlay_dir = PathBuf::from("/repo/org/repo/overlay-b");
        let repo_root = PathBuf::from("/repo");
        let result =
            resolve_mapping_source("../overlay-a/CLAUDE.md", &overlay_dir, Some(&repo_root));
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/repo/org/repo/overlay-a/CLAUDE.md")
        );
    }

    #[test]
    fn repo_root_key_resolves_from_root() {
        let overlay_dir = PathBuf::from("/repo/org/repo/overlay-b");
        let repo_root = PathBuf::from("/repo");
        let result = resolve_mapping_source(
            "//microsoft/FluidFramework/claude-config/CLAUDE.md",
            &overlay_dir,
            Some(&repo_root),
        );
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/repo/microsoft/FluidFramework/claude-config/CLAUDE.md")
        );
    }

    #[test]
    fn relative_key_escaping_repo_root_fails() {
        let overlay_dir = PathBuf::from("/repo/org/repo/overlay-b");
        let repo_root = PathBuf::from("/repo");
        let result =
            resolve_mapping_source("../../../../etc/passwd", &overlay_dir, Some(&repo_root));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("escapes overlay repo"));
    }

    #[test]
    fn repo_root_key_without_repo_root_fails() {
        let overlay_dir = PathBuf::from("/repo/org/repo/overlay-b");
        let result =
            resolve_mapping_source("//org/repo/overlay-a/file.txt", &overlay_dir, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an overlay repo"));
    }

    #[test]
    fn relative_key_without_repo_root_fails() {
        let overlay_dir = PathBuf::from("/repo/org/repo/overlay-b");
        let result =
            resolve_mapping_source("../overlay-a/file.txt", &overlay_dir, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("requires an overlay repo"));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test resolve_mapping_source_tests -- --nocapture 2>&1 | head -30`
Expected: Compilation error — `resolve_mapping_source` not defined.

**Step 3: Write the implementation**

Insert after `collect_overlay_files` (after line ~1787 in `src/lib.rs`):

```rust
/// Check whether a mapping key is an external reference (cross-overlay).
fn is_external_mapping(key: &str) -> bool {
    key.starts_with("//") || key.starts_with("../")
}

/// Resolve a mapping key to an absolute source path.
///
/// Handles three forms:
/// - Plain key (`file.txt`): resolved relative to `overlay_dir`
/// - Relative key (`../sibling/file.txt`): resolved relative to `overlay_dir`, validated against `repo_root`
/// - Repo-root key (`//org/repo/name/file.txt`): resolved from `repo_root`
///
/// Returns an error if the resolved path escapes the overlay repo root.
fn resolve_mapping_source(
    key: &str,
    overlay_dir: &Path,
    repo_root: Option<&Path>,
) -> Result<PathBuf> {
    if key.starts_with("//") {
        let Some(root) = repo_root else {
            bail!(
                "Cross-overlay reference '{key}' requires an overlay repo root, \
                 but this overlay was not resolved from an overlay repository"
            );
        };
        let rel = key.strip_prefix("//").unwrap();
        let resolved = root.join(rel);
        // Normalize to remove any .. components
        let normalized = normalize_path(&resolved);
        if !normalized.starts_with(root) {
            bail!("Cross-overlay reference escapes overlay repo: {key}");
        }
        Ok(normalized)
    } else if key.starts_with("../") {
        let Some(root) = repo_root else {
            bail!(
                "Cross-overlay reference '{key}' requires an overlay repo root, \
                 but this overlay was not resolved from an overlay repository"
            );
        };
        let resolved = overlay_dir.join(key);
        let normalized = normalize_path(&resolved);
        if !normalized.starts_with(root) {
            bail!("Cross-overlay reference escapes overlay repo: {key}");
        }
        Ok(normalized)
    } else {
        Ok(overlay_dir.join(key))
    }
}

/// Normalize a path by resolving `.` and `..` components without touching the filesystem.
/// Unlike `canonicalize()`, this works on paths that don't exist yet.
fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other),
        }
    }
    normalized
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test resolve_mapping_source_tests -- --nocapture`
Expected: All 6 tests pass.

**Step 5: Commit**

```bash
git add src/lib.rs
git commit -m "feat: add resolve_mapping_source helper for cross-overlay paths

Supports three mapping key forms: local (file.txt), relative
(../sibling/file.txt), and repo-root (//org/repo/name/file.txt).
Validates that resolved paths stay within the overlay repo root."
```

---

### Task 2: Update `collect_overlay_files` to handle external mappings

Change the signature to accept an optional `repo_root` parameter and add a second pass for external mapping keys.

**Files:**
- Modify: `src/lib.rs` — `collect_overlay_files` (lines 1743-1787)
- Test: `src/lib.rs` — `collect_overlay_files_tests` module (lines 7226-7328)

**Step 1: Write the failing tests**

Add to the existing `collect_overlay_files_tests` module:

```rust
#[test]
fn external_mapping_relative_resolves_sibling_file() {
    let temp = TempDir::new().unwrap();
    // Create overlay-a with a file
    let overlay_a = temp.path().join("org/repo/overlay-a");
    fs::create_dir_all(&overlay_a).unwrap();
    fs::write(overlay_a.join("CLAUDE.md"), "# AI config").unwrap();

    // Create overlay-b with a config referencing overlay-a
    let overlay_b = temp.path().join("org/repo/overlay-b");
    fs::create_dir_all(&overlay_b).unwrap();
    fs::write(overlay_b.join("local.txt"), "local content").unwrap();

    let mut mappings = std::collections::HashMap::new();
    mappings.insert(
        "../overlay-a/CLAUDE.md".to_string(),
        ".ai/instructions.md".to_string(),
    );
    let config = OverlayConfig {
        mappings,
        ..Default::default()
    };

    let files = collect_overlay_files(&overlay_b, &config, Some(temp.path()));
    assert_eq!(files.len(), 2);

    let external = files.iter().find(|(_, t)| t == ".ai/instructions.md");
    assert!(external.is_some(), "external mapping should be in results");
    let (source_path, _) = external.unwrap();
    assert!(
        source_path.is_absolute(),
        "external source should be absolute path"
    );
}

#[test]
fn external_mapping_repo_root_resolves() {
    let temp = TempDir::new().unwrap();
    // Create a file at repo root path
    let source_file = temp
        .path()
        .join("microsoft/FluidFramework/claude-config/CLAUDE.md");
    fs::create_dir_all(source_file.parent().unwrap()).unwrap();
    fs::write(&source_file, "# AI config").unwrap();

    // Create overlay-b
    let overlay_b = temp.path().join("org/repo/overlay-b");
    fs::create_dir_all(&overlay_b).unwrap();

    let mut mappings = std::collections::HashMap::new();
    mappings.insert(
        "//microsoft/FluidFramework/claude-config/CLAUDE.md".to_string(),
        ".cursor/instructions.md".to_string(),
    );
    let config = OverlayConfig {
        mappings,
        ..Default::default()
    };

    let files = collect_overlay_files(&overlay_b, &config, Some(temp.path()));
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].1, ".cursor/instructions.md");
}

#[test]
fn external_mapping_missing_file_is_skipped_with_warning() {
    let temp = TempDir::new().unwrap();
    let overlay_b = temp.path().join("org/repo/overlay-b");
    fs::create_dir_all(&overlay_b).unwrap();

    let mut mappings = std::collections::HashMap::new();
    mappings.insert(
        "../overlay-a/nonexistent.txt".to_string(),
        "target.txt".to_string(),
    );
    let config = OverlayConfig {
        mappings,
        ..Default::default()
    };

    let files = collect_overlay_files(&overlay_b, &config, Some(temp.path()));
    assert!(files.is_empty(), "missing external file should be skipped");
}

#[test]
fn local_mappings_still_work_with_repo_root() {
    let temp = TempDir::new().unwrap();
    let overlay = temp.path().join("org/repo/my-overlay");
    fs::create_dir_all(&overlay).unwrap();
    fs::write(overlay.join("source.txt"), "content").unwrap();

    let mut mappings = std::collections::HashMap::new();
    mappings.insert("source.txt".to_string(), "target.txt".to_string());
    let config = OverlayConfig {
        mappings,
        ..Default::default()
    };

    let files = collect_overlay_files(&overlay, &config, Some(temp.path()));
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, PathBuf::from("source.txt"));
    assert_eq!(files[0].1, "target.txt");
}

#[test]
fn existing_tests_work_with_none_repo_root() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("file1.txt"), "content1").unwrap();
    fs::write(temp.path().join("file2.txt"), "content2").unwrap();

    let config = OverlayConfig::default();
    let files = collect_overlay_files(temp.path(), &config, None);
    assert_eq!(files.len(), 2);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test collect_overlay_files_tests -- --nocapture 2>&1 | head -30`
Expected: Compilation error — `collect_overlay_files` takes 2 args, 3 given.

**Step 3: Update `collect_overlay_files` signature and implementation**

Change the function signature to accept `repo_root: Option<&Path>` and add the external references pass:

```rust
fn collect_overlay_files(
    source: &Path,
    config: &OverlayConfig,
    repo_root: Option<&Path>,
) -> Vec<(PathBuf, String)> {
    let dir_set: std::collections::HashSet<PathBuf> =
        config.directories.iter().map(PathBuf::from).collect();

    let mut files = Vec::new();

    // Phase 1: Walk local files (existing behavior)
    for entry in WalkDir::new(source)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let Ok(rel_path) = entry.path().strip_prefix(source) else {
            continue;
        };

        let rel_str = rel_path.to_string_lossy();
        if rel_path == Path::new(CONFIG_FILE)
            || rel_str.starts_with(".git/")
            || rel_str.starts_with(".git\\")
            || rel_str == ".git"
            || rel_str == ".repoverlay-cache-meta.ccl"
        {
            continue;
        }

        if dir_set.iter().any(|dir| rel_path.starts_with(dir)) {
            continue;
        }

        let rel_string = rel_str.to_string();
        let target_rel = config
            .mappings
            .get(&rel_string)
            .map_or_else(|| rel_string.clone(), Clone::clone);

        files.push((rel_path.to_path_buf(), target_rel));
    }

    // Phase 2: Resolve external mapping references
    for (key, target) in &config.mappings {
        if !is_external_mapping(key) {
            continue;
        }

        match resolve_mapping_source(key, source, repo_root) {
            Ok(resolved_path) => {
                if !resolved_path.exists() {
                    eprintln!(
                        "  {} Cross-overlay file not found, skipping: {}",
                        "Warning:".yellow(),
                        key
                    );
                    continue;
                }
                if !resolved_path.is_file() {
                    eprintln!(
                        "  {} Cross-overlay path is not a file, skipping: {}",
                        "Warning:".yellow(),
                        key
                    );
                    continue;
                }
                files.push((resolved_path, target.clone()));
            }
            Err(e) => {
                eprintln!("  {} {}", "Warning:".yellow(), e);
            }
        }
    }

    files
}
```

**Step 4: Fix all existing call sites**

Update every call to `collect_overlay_files` to pass the new parameter. There are 4 call sites:

1. `apply_resolved_overlay` (~line 1338): `collect_overlay_files(source, &config, None)` — will be updated in Task 4 to pass the real root.
2. `check_overlay_conflicts` (~line 1834): `collect_overlay_files(source, &config, None)`
3. A second call in conflict checking (~line 1924): `collect_overlay_files(source, config, None)`

For now, pass `None` at all call sites. Task 4 will thread the real repo root.

**Step 5: Fix existing tests**

Update all existing tests in `collect_overlay_files_tests` to pass `None` as the third argument. Every call like `collect_overlay_files(temp.path(), &config)` becomes `collect_overlay_files(temp.path(), &config, None)`.

**Step 6: Run tests to verify they pass**

Run: `cargo test collect_overlay_files_tests -- --nocapture`
Expected: All tests pass (both old and new).

**Step 7: Commit**

```bash
git add src/lib.rs
git commit -m "feat: extend collect_overlay_files with cross-overlay mapping resolution

Adds optional repo_root parameter and a second pass that resolves
external mapping keys (../ and //) to absolute paths within the
overlay repository."
```

---

### Task 3: Handle external directory references in `apply_resolved_overlay`

Update the directories loop in `apply_resolved_overlay` to resolve `../` and `//` directory entries.

**Files:**
- Modify: `src/lib.rs` — `apply_resolved_overlay` (lines 1100-1601), specifically the directories loop starting at ~line 1160
- Test: `src/lib.rs` — add integration tests

**Step 1: Write the failing tests**

Add a new test module `external_directory_tests`:

```rust
mod external_directory_tests {
    use super::*;

    #[test]
    fn relative_directory_reference_is_symlinked() {
        let temp = TempDir::new().unwrap();

        // Create overlay-a with a .claude directory
        let overlay_a = temp.path().join("overlay-repo/org/repo/overlay-a");
        fs::create_dir_all(overlay_a.join(".claude")).unwrap();
        fs::write(overlay_a.join(".claude/config.json"), "{}").unwrap();

        // Create overlay-b that references overlay-a's .claude dir
        let overlay_b = temp.path().join("overlay-repo/org/repo/overlay-b");
        fs::create_dir_all(&overlay_b).unwrap();
        fs::write(overlay_b.join("repoverlay.ccl"), "directories =\n  = ../overlay-a/.claude").unwrap();

        // Create target repo
        let target = temp.path().join("target-repo");
        fs::create_dir_all(target.join(".git")).unwrap();

        let resolved = ResolvedSource {
            path: overlay_b.clone(),
            source_info: OverlaySource::Local {
                path: overlay_b,
            },
        };

        let result = apply_resolved_overlay(
            &resolved,
            &target,
            false,
            None,
            ConflictStrategy::Fail,
            false,
        );

        assert!(result.is_ok(), "apply should succeed: {:?}", result.err());
        assert!(target.join(".claude").exists(), ".claude should exist in target");
    }

    #[test]
    fn repo_root_directory_reference_is_symlinked() {
        let temp = TempDir::new().unwrap();

        // Create source overlay with a directory
        let source_overlay = temp.path().join("overlay-repo/microsoft/FluidFramework/base");
        fs::create_dir_all(source_overlay.join("scratch")).unwrap();
        fs::write(source_overlay.join("scratch/notes.md"), "# Notes").unwrap();

        // Create overlay-b that references via //
        let overlay_b = temp.path().join("overlay-repo/org/repo/overlay-b");
        fs::create_dir_all(&overlay_b).unwrap();
        fs::write(overlay_b.join("repoverlay.ccl"), "directories =\n  = //microsoft/FluidFramework/base/scratch").unwrap();
        // Also need a file so the overlay isn't empty
        fs::write(overlay_b.join("marker.txt"), "marker").unwrap();

        // Create target repo
        let target = temp.path().join("target-repo");
        fs::create_dir_all(target.join(".git")).unwrap();

        let resolved = ResolvedSource {
            path: overlay_b.clone(),
            source_info: OverlaySource::Local {
                path: overlay_b,
            },
        };

        let result = apply_resolved_overlay(
            &resolved,
            &target,
            false,
            None,
            ConflictStrategy::Fail,
            false,
        );

        assert!(result.is_ok(), "apply should succeed: {:?}", result.err());
        assert!(target.join("scratch").exists(), "scratch should exist in target");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test external_directory_tests -- --nocapture 2>&1 | head -30`
Expected: Tests fail — directories not found because external directory resolution isn't implemented yet.

**Step 3: Add `resolve_directory_source` helper**

Insert near `resolve_mapping_source`:

```rust
/// Resolve a directory entry to an absolute source path and a target-relative path.
///
/// For external references (`../` or `//`), resolves to the absolute path of the
/// directory in the overlay repo. The target path is the last component(s) of the
/// original directory name (e.g., `../overlay-a/.claude` targets `.claude`).
///
/// Returns `(absolute_source_path, target_relative_path)`.
fn resolve_directory_source(
    dir_name: &str,
    overlay_dir: &Path,
    repo_root: Option<&Path>,
) -> Result<(PathBuf, PathBuf)> {
    if is_external_mapping(dir_name) {
        let resolved = resolve_mapping_source(dir_name, overlay_dir, repo_root)?;
        // The target path is derived from the referenced directory.
        // For "../overlay-a/.claude" → target is ".claude"
        // For "//org/repo/name/scratch" → target is "scratch"
        // We use the last path component as the target directory name.
        let target = if let Some(file_name) = resolved.file_name() {
            PathBuf::from(file_name)
        } else {
            bail!("Cross-overlay directory reference has no directory name: {dir_name}");
        };
        Ok((resolved, target))
    } else {
        let source = overlay_dir.join(dir_name);
        let target = PathBuf::from(dir_name);
        Ok((source, target))
    }
}
```

**Step 4: Update the directories loop in `apply_resolved_overlay`**

In `apply_resolved_overlay`, the directories loop currently does:
```rust
for dir_name in &config.directories {
    let dir_path = PathBuf::from(dir_name);
    let source_dir = source.join(&dir_path);
    // ...
```

Change to:
```rust
// Determine repo_root for cross-overlay references
let repo_root = infer_repo_root(source);

for dir_name in &config.directories {
    let (source_dir, dir_path) = match resolve_directory_source(dir_name, source, repo_root.as_deref()) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("  {} {}", "Warning:".yellow(), e);
            continue;
        }
    };
    // ... rest of the loop uses source_dir and dir_path as before
```

Also add a simple `infer_repo_root` helper that walks up from the overlay source directory to find the overlay repo root (the directory containing `.git`):

```rust
/// Infer the overlay repo root by walking up from the overlay source directory.
/// Returns None if no .git directory is found (e.g., for local overlays).
fn infer_repo_root(overlay_dir: &Path) -> Option<PathBuf> {
    let mut current = overlay_dir;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}
```

**Step 5: Run tests to verify they pass**

Run: `cargo test external_directory_tests -- --nocapture`
Expected: Both tests pass.

**Step 6: Commit**

```bash
git add src/lib.rs
git commit -m "feat: support cross-overlay directory references in directories section

Adds resolve_directory_source helper and updates the directories
loop in apply_resolved_overlay to handle ../ and // prefixes."
```

---

### Task 4: Thread `repo_root` through `collect_overlay_files` call sites

Update `apply_resolved_overlay` and `check_overlay_conflicts` to pass the inferred repo root to `collect_overlay_files`.

**Files:**
- Modify: `src/lib.rs` — call sites at ~lines 1338, 1834, 1924

**Step 1: Write the failing integration test**

Add to the tests module:

```rust
mod cross_overlay_integration_tests {
    use super::*;

    #[test]
    fn apply_overlay_with_external_file_mapping() {
        let temp = TempDir::new().unwrap();

        // Create overlay repo with .git
        let repo_root = temp.path().join("overlay-repo");
        fs::create_dir_all(repo_root.join(".git")).unwrap();

        // Create overlay-a with source file
        let overlay_a = repo_root.join("org/repo/overlay-a");
        fs::create_dir_all(&overlay_a).unwrap();
        fs::write(overlay_a.join("CLAUDE.md"), "# AI Config").unwrap();

        // Create overlay-b that references overlay-a's file
        let overlay_b = repo_root.join("org/repo/overlay-b");
        fs::create_dir_all(&overlay_b).unwrap();
        fs::write(
            overlay_b.join("repoverlay.ccl"),
            "mappings =\n  ../overlay-a/CLAUDE.md = .cursor/rules.md",
        )
        .unwrap();
        // Need at least one local file or directory
        fs::write(overlay_b.join("local.txt"), "local").unwrap();

        // Create target repo
        let target = temp.path().join("target-repo");
        fs::create_dir_all(target.join(".git")).unwrap();

        let resolved = ResolvedSource {
            path: overlay_b.clone(),
            source_info: OverlaySource::Local {
                path: overlay_b,
            },
        };

        let result = apply_resolved_overlay(
            &resolved,
            &target,
            true, // force copy so we don't need real symlinks
            None,
            ConflictStrategy::Fail,
            false,
        );

        assert!(result.is_ok(), "apply should succeed: {:?}", result.err());
        assert!(
            target.join(".cursor/rules.md").exists(),
            "cross-overlay file should be applied at mapped target"
        );
        let content = fs::read_to_string(target.join(".cursor/rules.md")).unwrap();
        assert_eq!(content, "# AI Config");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test cross_overlay_integration_tests -- --nocapture 2>&1 | head -30`
Expected: Test fails — `.cursor/rules.md` not created because `collect_overlay_files` receives `None` for repo_root.

**Step 3: Update call sites**

In `apply_resolved_overlay`, after loading the config (~line 1125), add:
```rust
let repo_root = infer_repo_root(source);
```

Then update the `collect_overlay_files` call (~line 1338):
```rust
for (rel_path, target_rel_str) in collect_overlay_files(source, &config, repo_root.as_deref()) {
```

In `check_overlay_conflicts` (~line 1794), add the same inference and update both calls:
```rust
let repo_root = infer_repo_root(source);
// ...
for (_rel_path, target_rel) in collect_overlay_files(source, &config, repo_root.as_deref()) {
```

And the second conflict check function that also calls `collect_overlay_files` (~line 1924):
```rust
let repo_root = infer_repo_root(source);
// ...
for (_rel_path, target_rel) in collect_overlay_files(source, config, repo_root.as_deref()) {
```

**Step 4: Run tests to verify they pass**

Run: `cargo test cross_overlay_integration_tests -- --nocapture`
Expected: PASS

**Step 5: Run the full test suite**

Run: `cargo test`
Expected: All tests pass.

**Step 6: Commit**

```bash
git add src/lib.rs
git commit -m "feat: thread repo_root through collect_overlay_files call sites

Uses infer_repo_root to discover the overlay repo root and passes
it to collect_overlay_files, enabling cross-overlay file references
in apply and conflict-check paths."
```

---

### Task 5: Handle source path in state tracking for external files

Currently `apply_resolved_overlay` records `rel_path` (relative to overlay dir) as the `source` in `FileEntry`. For external files, the source path is absolute. Update state tracking to record the repo-relative path for external sources.

**Files:**
- Modify: `src/lib.rs` — the file entry creation in `apply_resolved_overlay` (~line 1543)

**Step 1: Write the failing test**

Add to `cross_overlay_integration_tests`:

```rust
#[test]
fn state_records_repo_relative_source_for_external_files() {
    let temp = TempDir::new().unwrap();

    // Create overlay repo
    let repo_root = temp.path().join("overlay-repo");
    fs::create_dir_all(repo_root.join(".git")).unwrap();

    // Create overlay-a with source file
    let overlay_a = repo_root.join("org/repo/overlay-a");
    fs::create_dir_all(&overlay_a).unwrap();
    fs::write(overlay_a.join("CLAUDE.md"), "# AI Config").unwrap();

    // Create overlay-b referencing overlay-a
    let overlay_b = repo_root.join("org/repo/overlay-b");
    fs::create_dir_all(&overlay_b).unwrap();
    fs::write(
        overlay_b.join("repoverlay.ccl"),
        "mappings =\n  ../overlay-a/CLAUDE.md = .ai/config.md",
    )
    .unwrap();
    fs::write(overlay_b.join("local.txt"), "local").unwrap();

    // Create target repo
    let target = temp.path().join("target-repo");
    fs::create_dir_all(target.join(".git")).unwrap();

    let resolved = ResolvedSource {
        path: overlay_b.clone(),
        source_info: OverlaySource::Local { path: overlay_b },
    };

    apply_resolved_overlay(&resolved, &target, true, None, ConflictStrategy::Fail, false)
        .unwrap();

    // Read the state file and check source path
    let state = load_overlay_state(&target.join(STATE_DIR).join(OVERLAYS_DIR).join("overlay-b.ccl")).unwrap();
    let external_entry = state
        .files
        .iter()
        .find(|f| f.target == PathBuf::from(".ai/config.md"));
    assert!(external_entry.is_some(), "should have entry for external file");

    let entry = external_entry.unwrap();
    // Source should be repo-relative, not absolute
    assert!(
        !entry.source.is_absolute(),
        "source should be relative, got: {}",
        entry.source.display()
    );
    assert_eq!(
        entry.source,
        PathBuf::from("org/repo/overlay-a/CLAUDE.md"),
        "source should be repo-relative"
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test state_records_repo_relative -- --nocapture`
Expected: Assertion failure — source is absolute path instead of repo-relative.

**Step 3: Update FileEntry creation for external files**

In `apply_resolved_overlay`, where `FileEntry` is created for files (~line 1543), update the source field logic:

```rust
// Determine the source path to record in state
let source_for_state = if rel_path.is_absolute() {
    // External file — record repo-relative path if possible
    if let Some(root) = repo_root.as_deref() {
        rel_path
            .strip_prefix(root)
            .unwrap_or(&rel_path)
            .to_path_buf()
    } else {
        rel_path.clone()
    }
} else {
    rel_path.clone()
};

state.add_file(FileEntry {
    source: source_for_state,
    target: target_rel.clone(),
    link_type,
    entry_type: EntryType::File,
});
```

Apply the same logic for directory entries from external sources.

**Step 4: Run tests**

Run: `cargo test state_records_repo_relative -- --nocapture`
Expected: PASS

Run: `cargo test`
Expected: All tests pass.

**Step 5: Commit**

```bash
git add src/lib.rs
git commit -m "feat: record repo-relative source paths for cross-overlay files

External file references now store their source path relative to
the overlay repo root in state files, making state portable and
readable."
```

---

### Task 6: Run full checks and update documentation

**Files:**
- Modify: `website/src/content/docs/guides/creating.md` — add cross-overlay reference docs
- Run: `just check`

**Step 1: Run full check suite**

Run: `just check`
Expected: All checks pass (format, lint, test).

**Step 2: Fix any clippy or format issues**

Run: `cargo fmt` and `cargo clippy -- -D warnings` if needed.

**Step 3: Update documentation**

Add a new subsection to `website/src/content/docs/guides/creating.md` after the "Directories" subsection (~line 85):

```markdown
### Cross-overlay references

Mappings and directories can reference files from other overlays in the same overlay repository using two prefixes:

- **`../`** — resolve relative to the current overlay directory (for sibling overlays)
- **`//`** — resolve from the overlay repository root (for cross-org/repo references)

```
mappings =
  /= Reference a file from a sibling overlay
  ../claude-config/CLAUDE.md = .cursor/instructions.md

  /= Reference a file from any overlay in the repo
  //microsoft/FluidFramework/base/.envrc = .envrc

directories =
  /= Reference a directory from a sibling overlay
  = ../claude-config/.claude
```

This is useful when multiple overlays need the same files at different target paths — for example, when different AI agents expect configuration in different locations.

Cross-overlay references must stay within the overlay repository. Paths that escape the repo root are rejected.
```

**Step 4: Commit**

```bash
git add src/lib.rs website/src/content/docs/guides/creating.md
git commit -m "docs: add cross-overlay file references documentation

Documents the ../ and // prefixes for referencing files and
directories from other overlays in mappings and directories."
```

**Step 5: Run full suite one more time**

Run: `just check`
Expected: All green.
