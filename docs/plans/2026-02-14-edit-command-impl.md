# Edit Command Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an `edit` command for modifying existing overlays (add files, remove files, interactive re-selection), subsuming the existing `add` command.

**Architecture:** New `Edit` variant in the `Commands` enum with `--add`, `--remove`, and `--interactive` flags. Core logic split into `edit_overlay_add`, `edit_overlay_remove`, and `edit_overlay_interactive` functions. The existing `Add` command is deprecated with a warning. A new `remove_file` method is added to `OverlayState`.

**Tech Stack:** Rust (clap for CLI, anyhow for errors), existing overlay state/exclude infrastructure.

---

### Task 1: Add `remove_file` method to `OverlayState`

**Files:**
- Modify: `src/state.rs:244-271` (impl OverlayState block)
- Test: `src/state.rs` (inline unit tests)

**Step 1: Write the failing test**

Add a test module at the end of `src/state.rs` (or find the existing test module if present). The test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn remove_file_returns_matching_entry() {
        let mut state = OverlayState::new(
            "test".to_string(),
            OverlaySource::Local { path: PathBuf::from("/tmp") },
        );
        state.add_file(FileEntry {
            source: PathBuf::from("a.txt"),
            target: PathBuf::from("a.txt"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::File,
        });
        state.add_file(FileEntry {
            source: PathBuf::from("b.txt"),
            target: PathBuf::from("b.txt"),
            link_type: LinkType::Symlink,
            entry_type: EntryType::File,
        });

        let removed = state.remove_file(&PathBuf::from("a.txt"));
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().target, PathBuf::from("a.txt"));
        assert_eq!(state.file_count(), 1);
    }

    #[test]
    fn remove_file_returns_none_for_missing() {
        let mut state = OverlayState::new(
            "test".to_string(),
            OverlaySource::Local { path: PathBuf::from("/tmp") },
        );
        let removed = state.remove_file(&PathBuf::from("nonexistent.txt"));
        assert!(removed.is_none());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib state::tests -- --nocapture`
Expected: FAIL — `remove_file` method does not exist.

**Step 3: Write minimal implementation**

Add to `impl OverlayState` in `src/state.rs:257-270` (after `add_file`, before `file_count`):

```rust
    /// Remove a file entry by target path. Returns the removed entry, or None if not found.
    pub fn remove_file(&mut self, target: &Path) -> Option<FileEntry> {
        if let Some(pos) = self.files.iter().position(|f| f.target == target) {
            Some(self.files.remove(pos))
        } else {
            None
        }
    }
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib state::tests -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add src/state.rs
git commit -m "feat(state): add remove_file method to OverlayState"
```

---

### Task 2: Add `Edit` command variant to CLI

**Files:**
- Modify: `src/cli.rs:81` (Commands enum — add Edit variant after Add)
- Modify: `src/cli.rs:351-374` (Add variant — mark hidden, add deprecation alias)

**Step 1: Write the failing test**

Add unit tests for CLI argument parsing in `src/cli.rs` tests module (near line 5305):

```rust
        #[test]
        fn edit_requires_name() {
            let result = Cli::try_parse_from(["repoverlay", "edit"]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_requires_operation() {
            // edit with just a name and no flags should fail
            let result = Cli::try_parse_from(["repoverlay", "edit", "my-overlay"]);
            // This succeeds at parse time but we validate at runtime
            // Just check it parses for now
            assert!(result.is_ok());
        }

        #[test]
        fn edit_parses_add_flag() {
            let cli = Cli::try_parse_from([
                "repoverlay", "edit", "my-overlay", "--add", "file1.txt", "file2.txt",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit { name, add, remove, interactive, dry_run, .. }) => {
                    assert_eq!(name, "my-overlay");
                    assert_eq!(add, vec![PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]);
                    assert!(remove.is_empty());
                    assert!(!interactive);
                    assert!(!dry_run);
                }
                _ => panic!("Expected Edit command"),
            }
        }

        #[test]
        fn edit_parses_remove_flag() {
            let cli = Cli::try_parse_from([
                "repoverlay", "edit", "my-overlay", "--remove", "file1.txt",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit { remove, .. }) => {
                    assert_eq!(remove, vec![PathBuf::from("file1.txt")]);
                }
                _ => panic!("Expected Edit command"),
            }
        }

        #[test]
        fn edit_parses_combined_add_remove() {
            let cli = Cli::try_parse_from([
                "repoverlay", "edit", "my-overlay",
                "--add", "new.txt",
                "--remove", "old.txt",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit { add, remove, .. }) => {
                    assert_eq!(add, vec![PathBuf::from("new.txt")]);
                    assert_eq!(remove, vec![PathBuf::from("old.txt")]);
                }
                _ => panic!("Expected Edit command"),
            }
        }

        #[test]
        fn edit_interactive_conflicts_with_add() {
            let result = Cli::try_parse_from([
                "repoverlay", "edit", "my-overlay", "--interactive", "--add", "file.txt",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_interactive_conflicts_with_remove() {
            let result = Cli::try_parse_from([
                "repoverlay", "edit", "my-overlay", "--interactive", "--remove", "file.txt",
            ]);
            assert!(result.is_err());
        }

        #[test]
        fn edit_parses_dry_run() {
            let cli = Cli::try_parse_from([
                "repoverlay", "edit", "my-overlay", "--add", "f.txt", "--dry-run",
            ])
            .unwrap();

            match cli.command {
                Some(Commands::Edit { dry_run, .. }) => {
                    assert!(dry_run);
                }
                _ => panic!("Expected Edit command"),
            }
        }
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib cli::tests -- edit --nocapture`
Expected: FAIL — `Commands::Edit` does not exist.

**Step 3: Write the Edit variant**

Add to `Commands` enum in `src/cli.rs` (after the `Add` variant, around line 374):

```rust
    /// Edit an existing applied overlay (add files, remove files, or re-select interactively)
    ///
    /// Examples:
    ///   repoverlay edit my-overlay --add newfile.txt
    ///   repoverlay edit my-overlay --remove oldfile.txt
    ///   repoverlay edit my-overlay --add new.txt --remove old.txt
    ///   repoverlay edit org/repo/my-overlay --interactive
    Edit {
        /// Overlay name or full path (org/repo/name)
        ///
        /// Short form: `my-overlay` - detects org/repo from git remote
        /// Full form: `org/repo/name` - uses explicit values
        name: String,

        /// Files to add to the overlay
        #[arg(short, long, value_name = "FILE", num_args = 1..)]
        add: Vec<PathBuf>,

        /// Files to remove from the overlay
        #[arg(short, long, value_name = "FILE", num_args = 1..)]
        remove: Vec<PathBuf>,

        /// Re-run interactive file selection with current files pre-selected
        #[arg(short, long, conflicts_with_all = ["add", "remove"])]
        interactive: bool,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show what would change without making changes
        #[arg(long)]
        dry_run: bool,
    },
```

Also mark the existing `Add` variant as hidden (deprecation):

```rust
    /// Add files to an existing applied overlay
    ///
    /// Deprecated: use `repoverlay edit --add` instead.
    #[command(hide = true)]
    Add {
        // ... existing fields unchanged
    },
```

Add the command dispatch in the `run()` function (around line 624). Add before the `Commands::Add` match arm:

```rust
        Commands::Edit {
            name,
            add,
            remove,
            interactive,
            target,
            dry_run,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            edit_overlay(&name, &target, &add, &remove, interactive, dry_run)?;
        }
```

Update the `Commands::Add` dispatch to print a deprecation warning:

```rust
        Commands::Add {
            name,
            files,
            target,
            dry_run,
        } => {
            eprintln!(
                "{} 'repoverlay add' is deprecated. Use 'repoverlay edit --add' instead.",
                "Warning:".yellow().bold()
            );
            eprintln!();
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            add_files_to_overlay(&name, &target, &files, dry_run)?;
        }
```

Add a stub `edit_overlay` function to make it compile:

```rust
fn edit_overlay(
    name_arg: &str,
    target: &std::path::Path,
    add_files: &[PathBuf],
    remove_files: &[PathBuf],
    interactive: bool,
    dry_run: bool,
) -> Result<()> {
    let _ = (name_arg, target, add_files, remove_files, interactive, dry_run);
    bail!("edit command not yet implemented");
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib cli::tests -- edit --nocapture`
Expected: PASS for all edit parsing tests.

**Step 5: Commit**

```bash
git add src/cli.rs
git commit -m "feat(cli): add Edit command variant with --add/--remove/--interactive flags

Deprecate the Add command (hidden, shows warning). Add stub edit_overlay
function. All CLI parsing tests pass."
```

---

### Task 3: Implement `edit --add` (delegate to existing logic)

**Files:**
- Modify: `src/cli.rs` (the `edit_overlay` function)

**Step 1: Write the failing integration test**

Add to `tests/cli.rs`:

```rust
#[test]
fn edit_add_fails_when_overlay_not_applied() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "org/repo/nonexistent-overlay", "--add", "some-file.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not currently applied"));
}

#[test]
fn edit_add_fails_when_file_does_not_exist() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "org/repo/test-overlay", "--add", "nonexistent-file.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("File does not exist"));
}

#[test]
fn edit_fails_when_no_operation_specified() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "org/repo/my-overlay"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("specify at least one"));
}
```

**Step 2: Run test to verify they fail**

Run: `cargo test --test cli edit -- --nocapture`
Expected: FAIL — "edit command not yet implemented" or similar.

**Step 3: Implement `edit_overlay` with add support**

Replace the stub `edit_overlay` function:

```rust
fn edit_overlay(
    name_arg: &str,
    target: &std::path::Path,
    add_files: &[PathBuf],
    remove_files: &[PathBuf],
    interactive: bool,
    dry_run: bool,
) -> Result<()> {
    // Validate at least one operation
    if add_files.is_empty() && remove_files.is_empty() && !interactive {
        bail!(
            "No operation specified. Please specify at least one of:\n  \
             --add <file>      Add files to the overlay\n  \
             --remove <file>   Remove files from the overlay\n  \
             --interactive     Re-select files interactively"
        );
    }

    // Handle add
    if !add_files.is_empty() {
        add_files_to_overlay(name_arg, target, add_files, dry_run)?;
    }

    // Handle remove
    if !remove_files.is_empty() {
        remove_files_from_overlay(name_arg, target, remove_files, dry_run)?;
    }

    // Interactive mode is handled in Task 5
    if interactive {
        bail!("Interactive edit mode is not yet implemented");
    }

    Ok(())
}
```

Add a stub for `remove_files_from_overlay`:

```rust
fn remove_files_from_overlay(
    _name_arg: &str,
    _target: &std::path::Path,
    _files: &[PathBuf],
    _dry_run: bool,
) -> Result<()> {
    bail!("remove from overlay not yet implemented");
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --test cli edit -- --nocapture`
Expected: PASS for the validation test, PASS for add error tests (they hit the add path).

**Step 5: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat(edit): implement edit --add by delegating to existing add logic

Validates that at least one operation is specified. The --add flag
delegates to add_files_to_overlay. Remove and interactive are stubs."
```

---

### Task 4: Implement `edit --remove`

**Files:**
- Modify: `src/cli.rs` (replace `remove_files_from_overlay` stub)

**Step 1: Write the failing integration tests**

Add to `tests/cli.rs`:

```rust
#[test]
fn edit_remove_removes_file_from_overlay() {
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        ("extra.txt", "extra content"),
    ]);

    // Apply overlay with both files
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists("extra.txt"));

    // Remove one file
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "test-overlay", "--remove", "extra.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 file"));

    // Verify extra.txt is gone but .envrc remains
    assert!(!ctx.file_exists("extra.txt"));
    assert!(ctx.file_exists(".envrc"));

    // Verify overlay state still exists (overlay not fully removed)
    assert!(ctx.overlay_state_exists("test-overlay"));

    // Verify git exclude still has .envrc but not extra.txt
    let exclude = ctx.git_exclude_content();
    assert!(exclude.contains(".envrc"));
    assert!(!exclude.contains("extra.txt"));
}

#[test]
fn edit_remove_fails_when_file_not_in_overlay() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["edit", "test-overlay", "--remove", "nonexistent.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not managed by overlay"));
}

#[test]
fn edit_remove_dry_run_does_not_modify() {
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        ("extra.txt", "extra content"),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Dry run remove
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "test-overlay", "--remove", "extra.txt", "--dry-run"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));

    // File should still exist
    assert!(ctx.file_exists("extra.txt"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --test cli edit_remove -- --nocapture`
Expected: FAIL — "remove from overlay not yet implemented".

**Step 3: Implement `remove_files_from_overlay`**

Replace the stub in `src/cli.rs`:

```rust
fn remove_files_from_overlay(
    name_arg: &str,
    target: &std::path::Path,
    files: &[PathBuf],
    dry_run: bool,
) -> Result<()> {
    use crate::state::EntryType;
    use crate::{
        load_overlay_state, normalize_overlay_name, list_applied_overlays,
        save_overlay_state, save_external_state, update_git_exclude,
    };

    let target = canonicalize_path(target, "Target directory")?;
    if !target.join(".git").exists() {
        bail!("Target directory is not a git repository: {}", target.display());
    }

    // Parse name and verify overlay is applied
    let normalized_name = if name_arg.contains('/') {
        let parts: Vec<&str> = name_arg.split('/').collect();
        if parts.len() == 3 {
            crate::normalize_overlay_name(parts[2])?
        } else {
            bail!("Invalid overlay path: {name_arg}");
        }
    } else {
        crate::normalize_overlay_name(name_arg)?
    };

    let applied_overlays = list_applied_overlays(&target)?;
    if !applied_overlays.contains(&normalized_name) {
        bail!(
            "Overlay '{}' is not currently applied.\n\n\
             Applied overlays: {}",
            name_arg,
            if applied_overlays.is_empty() { "(none)".to_string() } else { applied_overlays.join(", ") }
        );
    }

    let mut state = load_overlay_state(&target, &normalized_name)?;

    // Validate all files are managed by this overlay
    for file in files {
        let file_normalized = file.to_string_lossy().replace('\\', "/");
        if !state.file_entries().iter().any(|e| {
            e.target.to_string_lossy().replace('\\', "/") == file_normalized
        }) {
            let managed: Vec<String> = state.file_entries().iter()
                .map(|e| e.target.to_string_lossy().into_owned())
                .collect();
            bail!(
                "File '{}' is not managed by overlay '{}'.\n\n\
                 Files in this overlay: {}",
                file.display(),
                normalized_name,
                managed.join(", ")
            );
        }
    }

    println!(
        "{} files from overlay: {}",
        "Removing".red().bold(),
        normalized_name
    );

    if dry_run {
        println!("  Target: {}", target.display());
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        println!("\nFiles that would be removed:");
        for file in files {
            println!("  {} {}", "-".red(), file.display());
        }
        return Ok(());
    }

    let mut removed_count = 0;

    for file in files {
        let file_path = target.join(file);

        // Remove symlink/copy
        if file_path.exists() || file_path.is_symlink() {
            // Determine entry type from state
            let entry = state.file_entries().iter().find(|e| e.target == *file).unwrap();
            match entry.entry_type {
                EntryType::Directory => {
                    if file_path.is_symlink() {
                        #[cfg(unix)]
                        fs::remove_file(&file_path)?;
                        #[cfg(windows)]
                        fs::remove_dir(&file_path)?;
                    } else {
                        fs::remove_dir_all(&file_path)?;
                    }
                    println!("  {} {}/", "-".red(), file.display());
                }
                EntryType::File => {
                    fs::remove_file(&file_path)
                        .with_context(|| format!("Failed to remove: {}", file_path.display()))?;
                    println!("  {} {}", "-".red(), file.display());
                }
            }

            // Clean up empty parent directories
            let mut parent = file_path.parent();
            while let Some(dir) = parent {
                if dir == target {
                    break;
                }
                if dir
                    .read_dir()
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
                {
                    fs::remove_dir(dir).ok();
                    parent = dir.parent();
                } else {
                    break;
                }
            }
        }

        // Remove from state
        state.remove_file(file);
        removed_count += 1;
    }

    // Rebuild exclude entries from remaining files
    let remaining_entries: Vec<String> = state.file_entries().iter()
        .map(|e| {
            let path = e.target.to_string_lossy().replace('\\', "/");
            match e.entry_type {
                EntryType::Directory => format!("{path}/"),
                EntryType::File => path,
            }
        })
        .collect();

    // Update git exclude - rewrite with remaining entries
    update_git_exclude(&target, &normalized_name, &remaining_entries, true)?;

    // Save updated state
    save_overlay_state(&target, &state)?;

    // Save external backup
    if let Err(e) = save_external_state(&target, &normalized_name, &state) {
        eprintln!(
            "  {} Could not save external backup: {}",
            "Warning:".yellow(),
            e
        );
    }

    println!(
        "\n{} Removed {} file(s) from overlay '{}'",
        "✓".green().bold(),
        removed_count,
        normalized_name
    );

    Ok(())
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --test cli edit_remove -- --nocapture`
Expected: PASS

Also run the full test suite:
Run: `cargo test`
Expected: PASS

**Step 5: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat(edit): implement edit --remove for removing files from overlays

Removes files from target repo, updates overlay state, rewrites git
exclude with remaining entries. Validates files are managed by the
overlay before removal."
```

---

### Task 5: Implement `edit --interactive` (stretch goal)

**Files:**
- Modify: `src/cli.rs` (replace interactive stub in `edit_overlay`)
- Uses: `src/selection.rs` (existing selection UI)

**Note:** This task depends on understanding how the selection UI is invoked elsewhere. The interactive mode is more complex because it requires:
1. Resolving the overlay source to a local directory
2. Listing all files in that directory
3. Converting them to `DetectedFile` format for the selection UI
4. Pre-selecting currently applied files
5. Computing a diff after selection

This is the most complex part and may warrant further exploration of `detection.rs` and `selection.rs` to understand the `DetectedFile` and `SelectionConfig` types.

**Step 1: Research the existing interactive patterns**

Read the `create` command's interactive flow to understand how `DetectedFile`, `select_files`, and `SelectionConfig` work. Key files:
- `src/selection.rs` — `select_files()` function signature and `SelectionConfig`
- `src/detection.rs` — `DetectedFile` struct
- `src/cli.rs` — the `create_overlay_command` function that uses them

**Step 2: Write the failing integration test**

```rust
#[test]
fn edit_interactive_fails_for_local_overlay() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success();

    // Interactive mode requires overlay repo source — local overlays should fail gracefully
    cargo_bin_cmd!("repoverlay")
        .args(["edit", "test-overlay", "--interactive"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Interactive edit"));
}
```

**Step 3: Implement interactive mode**

This requires more codebase exploration. The implementation should:
1. Load the overlay state
2. Resolve the overlay source to a local path (for overlay repo: use `OverlayRepoManager`; for local: use the stored path; for GitHub: error)
3. Walk the source directory to find all files
4. Create `DetectedFile` entries with pre-selection
5. Launch `select_files()` with appropriate config
6. Compare selections with current state to compute adds/removes
7. Apply the changes

**Step 4-5:** Standard test + commit cycle.

**Note for implementer:** If the selection UI's `DetectedFile` type doesn't support pre-selection well, you may need to add a `preselected` field or use the `selections` field in `SelectionState`. Study `selection.rs` carefully before implementing.

---

### Task 6: Update existing `add` CLI tests to handle deprecation warning

**Files:**
- Modify: `tests/cli.rs` (existing add tests)
- Modify: `src/cli.rs` (existing add unit tests)

**Step 1: Update integration tests**

The existing `add` integration tests will now print deprecation warnings to stderr. Tests that check stderr for specific error messages should still work since `predicate::str::contains` is a substring match. But verify by running:

Run: `cargo test --test cli add -- --nocapture`

If any tests fail due to the deprecation warning in stdout/stderr, adjust them.

**Step 2: Add deprecation warning test**

```rust
#[test]
fn add_shows_deprecation_warning() {
    let ctx = TestContext::new();

    cargo_bin_cmd!("repoverlay")
        .args(["add", "org/repo/nonexistent-overlay", "some-file.txt"])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("deprecated"));
}
```

**Step 3: Run full test suite**

Run: `cargo test`
Expected: PASS — all existing tests still pass, new tests pass.

**Step 4: Commit**

```bash
git add tests/cli.rs src/cli.rs
git commit -m "test(edit): add deprecation test for old add command

Verify existing add tests still pass with deprecation warning."
```

---

### Task 7: Run full checks and final cleanup

**Step 1: Run all checks**

Run: `just check`
Expected: format, lint, and test all pass.

**Step 2: Fix any clippy/format issues**

If clippy complains about anything in the new code, fix it.

**Step 3: Final commit (if needed)**

```bash
git add -A
git commit -m "chore: address clippy and formatting in edit command"
```
