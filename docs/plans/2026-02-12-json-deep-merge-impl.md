# JSON Deep Merge Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `--merge` flag and `REPOVERLAY_MERGE` env var that deep merges `.json` files instead of treating them as conflicts during overlay application.

**Architecture:** A new `merge` boolean flows alongside the existing `ConflictStrategy` through the apply chain. At each conflict detection point, if `merge` is true and the file is `.json`, we read both files, deep merge them (overlay wins), and write the result as a regular file. A new `json_merge` module in `src/json_merge.rs` contains the merge logic and logging. State tracks merged files via a new `LinkType::Merged` variant.

**Tech Stack:** Rust, serde_json (new dep), clap (existing)

**Design doc:** `docs/plans/2026-02-12-json-deep-merge-design.md`

---

### Task 1: Add serde_json dependency

**Files:**
- Modify: `Cargo.toml:21-38` (dependencies section)

**Step 1: Add the dependency**

Add `serde_json` to the `[dependencies]` section in `Cargo.toml`:

```toml
serde_json = "1.0"
```

Place it after the existing `serde` line (line 27).

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: SUCCESS (no code uses it yet, just confirming the dep resolves)

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(deps): add serde_json for JSON deep merge support"
```

---

### Task 2: Create the json_merge module with deep merge function

**Files:**
- Create: `src/json_merge.rs`
- Modify: `src/lib.rs` (add `mod json_merge;` declaration)

**Step 1: Write the failing test**

Create `src/json_merge.rs` with the merge function signature and tests at the bottom:

```rust
//! JSON deep merge utilities for overlay application.

use serde_json::Value;
use std::path::Path;

/// Result of merging two JSON values, with statistics for logging.
#[derive(Debug, Default)]
pub(crate) struct MergeResult {
    pub merged: Value,
    pub keys_added: usize,
    pub keys_overridden: usize,
    pub type_mismatches: Vec<TypeMismatch>,
}

/// A type mismatch encountered during merge.
#[derive(Debug)]
pub(crate) struct TypeMismatch {
    pub key_path: String,
    pub base_type: String,
    pub overlay_type: String,
}

/// Deep merge two JSON values. Overlay wins for scalars, arrays, and type mismatches.
/// Objects are recursively merged.
pub(crate) fn deep_merge(base: &Value, overlay: &Value) -> MergeResult {
    let mut result = MergeResult::default();
    result.merged = merge_values(base, overlay, String::new(), &mut result);
    result
}

fn merge_values(
    base: &Value,
    overlay: &Value,
    path: String,
    stats: &mut MergeResult,
) -> Value {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            let mut merged = base_map.clone();
            for (key, overlay_val) in overlay_map {
                let key_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                match base_map.get(key) {
                    Some(base_val) => {
                        let merged_val =
                            merge_values(base_val, overlay_val, key_path, stats);
                        merged.insert(key.clone(), merged_val);
                    }
                    None => {
                        stats.keys_added += 1;
                        merged.insert(key.clone(), overlay_val.clone());
                    }
                }
            }
            Value::Object(merged)
        }
        _ => {
            // Non-object cases: overlay wins
            if base != overlay {
                let current_path = if path.is_empty() {
                    "(root)".to_string()
                } else {
                    path
                };

                if std::mem::discriminant(base) != std::mem::discriminant(overlay) {
                    stats.type_mismatches.push(TypeMismatch {
                        key_path: current_path,
                        base_type: json_type_name(base).to_string(),
                        overlay_type: json_type_name(overlay).to_string(),
                    });
                } else {
                    stats.keys_overridden += 1;
                }
            }
            overlay.clone()
        }
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Check if a file path has a .json extension.
pub(crate) fn is_json_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

/// Read a file, parse as JSON, and return the parsed value.
/// Returns an error if the file can't be read or isn't valid JSON.
pub(crate) fn read_json_file(path: &Path) -> anyhow::Result<Value> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read JSON file: {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON file: {}", path.display()))
}

/// Merge overlay JSON into base JSON and write the result to the target path.
/// Returns the MergeResult with statistics for logging.
pub(crate) fn merge_json_files(
    base_path: &Path,
    overlay_path: &Path,
    target_path: &Path,
) -> anyhow::Result<MergeResult> {
    let base = read_json_file(base_path)?;
    let overlay = read_json_file(overlay_path)?;
    let result = deep_merge(&base, &overlay);

    let output = serde_json::to_string_pretty(&result.merged)
        .context("Failed to serialize merged JSON")?;
    std::fs::write(target_path, output)
        .with_context(|| format!("Failed to write merged JSON: {}", target_path.display()))?;

    Ok(result)
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_disjoint_objects() {
        let base = json!({"a": 1});
        let overlay = json!({"b": 2});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1, "b": 2}));
        assert_eq!(result.keys_added, 1);
        assert_eq!(result.keys_overridden, 0);
        assert!(result.type_mismatches.is_empty());
    }

    #[test]
    fn merge_overlapping_scalars_overlay_wins() {
        let base = json!({"a": 1, "b": 2});
        let overlay = json!({"b": 99});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1, "b": 99}));
        assert_eq!(result.keys_overridden, 1);
    }

    #[test]
    fn merge_nested_objects_recursively() {
        let base = json!({"outer": {"a": 1, "b": 2}});
        let overlay = json!({"outer": {"b": 99, "c": 3}});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"outer": {"a": 1, "b": 99, "c": 3}}));
        assert_eq!(result.keys_overridden, 1); // outer.b
        assert_eq!(result.keys_added, 1); // outer.c
    }

    #[test]
    fn merge_arrays_overlay_replaces() {
        let base = json!({"list": [1, 2, 3]});
        let overlay = json!({"list": [4, 5]});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"list": [4, 5]}));
        assert_eq!(result.keys_overridden, 1);
    }

    #[test]
    fn merge_type_mismatch_overlay_wins_and_logs() {
        let base = json!({"key": "string_value"});
        let overlay = json!({"key": {"nested": true}});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"key": {"nested": true}}));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].key_path, "key");
        assert_eq!(result.type_mismatches[0].base_type, "string");
        assert_eq!(result.type_mismatches[0].overlay_type, "object");
    }

    #[test]
    fn merge_deeply_nested_type_mismatch_has_full_path() {
        let base = json!({"a": {"b": {"c": 42}}});
        let overlay = json!({"a": {"b": {"c": "now a string"}}});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": {"b": {"c": "now a string"}}}));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].key_path, "a.b.c");
    }

    #[test]
    fn merge_empty_base_takes_overlay() {
        let base = json!({});
        let overlay = json!({"a": 1, "b": [2]});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1, "b": [2]}));
        assert_eq!(result.keys_added, 2);
    }

    #[test]
    fn merge_empty_overlay_preserves_base() {
        let base = json!({"a": 1, "b": 2});
        let overlay = json!({});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1, "b": 2}));
        assert_eq!(result.keys_added, 0);
        assert_eq!(result.keys_overridden, 0);
    }

    #[test]
    fn is_json_file_detects_extension() {
        assert!(is_json_file(Path::new("settings.json")));
        assert!(is_json_file(Path::new("path/to/config.JSON")));
        assert!(!is_json_file(Path::new("file.txt")));
        assert!(!is_json_file(Path::new("file.jsonl")));
        assert!(!is_json_file(Path::new("file")));
    }

    #[test]
    fn merge_null_values() {
        let base = json!({"a": null});
        let overlay = json!({"a": 1});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1}));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].base_type, "null");
    }
}
```

**Step 2: Add module declaration to lib.rs**

In `src/lib.rs`, add after the existing module declarations (around line 10-15, near the other `mod` lines):

```rust
mod json_merge;
```

**Step 3: Run tests to verify they pass**

Run: `cargo test json_merge`
Expected: All 9 tests PASS

**Step 4: Run clippy**

Run: `cargo clippy`
Expected: No warnings

**Step 5: Commit**

```bash
git add src/json_merge.rs src/lib.rs
git commit -m "feat: add json_merge module with deep merge logic and tests

Implements recursive JSON deep merge where overlay wins for scalars,
arrays, and type mismatches. Tracks merge statistics (keys added,
overridden, type mismatches) for verbose logging."
```

---

### Task 3: Add `Merged` variant to `LinkType` enum

**Files:**
- Modify: `src/state.rs:288` (LinkType enum)
- Modify: `src/lib.rs` (match arms on LinkType)

**Step 1: Add the variant**

In `src/state.rs`, add `Merged` to the `LinkType` enum at line 290:

```rust
pub enum LinkType {
    Symlink,
    Copy,
    Merged,
}
```

**Step 2: Fix all match arms**

After adding the variant, `cargo check` will show exhaustive match errors. Fix each one:

1. `src/lib.rs` ~line 1200 (file operation match): Add `LinkType::Merged` — this arm should be unreachable at this point (we'll wire it up in Task 5), but add a placeholder:

```rust
LinkType::Merged => {
    // Merged files are handled earlier in the conflict resolution path.
    // If we reach here, it's a bug.
    unreachable!("Merged link type should not reach file copy path");
}
```

2. Any other match arms on `LinkType` — search for them and add the `Merged` variant. For `restore_files()` in lib.rs, treat `Merged` the same as `Copy` (it's a regular file, just delete it):

Find the match in the restore path and add:
```rust
LinkType::Merged => {
    // Merged files are regular files, remove like copies
    if target_file.exists() {
        fs::remove_file(&target_file)?;
    }
}
```

**Step 3: Run tests**

Run: `cargo test`
Expected: All existing tests PASS (no behavior change yet)

**Step 4: Run clippy**

Run: `cargo clippy`
Expected: No warnings

**Step 5: Commit**

```bash
git add src/state.rs src/lib.rs
git commit -m "feat(state): add Merged variant to LinkType enum

Tracks files that were produced by JSON deep merge. Treated as regular
files for removal/restore purposes."
```

---

### Task 4: Add `--merge` flag to CLI commands and thread it through

**Files:**
- Modify: `src/cli.rs` (Apply, Restore, Update, Switch command structs + handlers)
- Modify: `src/lib.rs` (function signatures: `apply_overlay`, `apply_resolved_overlay`, `apply_multiple_overlays`)

**Step 1: Add --merge flag to Apply command struct**

In `src/cli.rs`, add the `merge` field to the `Apply` variant (after `skip_conflicts`, around line 120):

```rust
/// Deep merge conflicting JSON files instead of failing
#[arg(long, env = "REPOVERLAY_MERGE")]
merge: bool,
```

Note: `env = "REPOVERLAY_MERGE"` makes clap auto-read the env var. Values `1`, `true`, `TRUE`, etc. are truthy.

**Step 2: Add --merge to Restore, Update, Switch**

Add the same field to each command struct:

- `Restore` (after `skip_conflicts`, ~line 180):
```rust
/// Deep merge conflicting JSON files instead of failing
#[arg(long, env = "REPOVERLAY_MERGE")]
merge: bool,
```

- `Update` (after `skip_conflicts`, ~line 202):
```rust
/// Deep merge conflicting JSON files instead of failing
#[arg(long, env = "REPOVERLAY_MERGE")]
merge: bool,
```

- `Switch` (after `skip_conflicts`, ~line 295):
```rust
/// Deep merge conflicting JSON files instead of failing
#[arg(long, env = "REPOVERLAY_MERGE")]
merge: bool,
```

**Step 3: Thread `merge` through function signatures**

Update function signatures in `src/lib.rs`:

`apply_overlay` (line 828): Add `merge: bool` parameter after `conflict_strategy`:
```rust
pub(crate) fn apply_overlay(
    source_str: &str,
    target: &Path,
    force_copy: bool,
    name_override: Option<String>,
    ref_override: Option<&str>,
    update_cache: bool,
    conflict_strategy: ConflictStrategy,
    merge: bool,
    source_filter: Option<&str>,
    // ... rest
```

`apply_resolved_overlay` (line 896): Add `merge: bool` parameter:
```rust
fn apply_resolved_overlay(
    resolved: &ResolvedSource,
    target: &Path,
    force_copy: bool,
    name_override: Option<String>,
    conflict_strategy: ConflictStrategy,
    merge: bool,
) -> Result<()> {
```

`apply_multiple_overlays` (line 1277): Add `merge: bool` parameter:
```rust
fn apply_multiple_overlays(
    sources: &[ResolvedSource],
    target: &Path,
    force_copy: bool,
    dry_run: bool,
    conflict_strategy: ConflictStrategy,
    merge: bool,
) -> Result<()> {
```

**Step 4: Update all call sites**

Pass the `merge` value through at each call site:

1. CLI handler for Apply (~line 481): Add `merge` to the destructure pattern and pass to `apply_overlay`
2. CLI handler for Restore, Update, Switch: Same pattern
3. `apply_overlay` → `apply_resolved_overlay` call (~line 883): Pass `merge`
4. `apply_multiple_overlays` → `apply_resolved_overlay` call (~line 1348): Pass `merge`
5. Any other callers of these functions (search for all call sites)

For now, the `merge` parameter is accepted but unused — we wire it up in Task 5.

**Step 5: Add CLI parsing tests**

Add to the test section in `src/cli.rs` (near line 4663):

```rust
#[test]
fn apply_parses_merge_flag() {
    let cli = Cli::try_parse_from(["repoverlay", "apply", "./overlay", "--merge"]).unwrap();
    match cli.command {
        Some(Commands::Apply { merge, .. }) => {
            assert!(merge);
        }
        _ => panic!("Expected Apply command"),
    }
}

#[test]
fn apply_merge_combinable_with_force() {
    let cli = Cli::try_parse_from([
        "repoverlay", "apply", "./overlay", "--merge", "--force",
    ]).unwrap();
    match cli.command {
        Some(Commands::Apply { merge, force, .. }) => {
            assert!(merge);
            assert!(force);
        }
        _ => panic!("Expected Apply command"),
    }
}

#[test]
fn apply_merge_combinable_with_skip_conflicts() {
    let cli = Cli::try_parse_from([
        "repoverlay", "apply", "./overlay", "--merge", "--skip-conflicts",
    ]).unwrap();
    match cli.command {
        Some(Commands::Apply { merge, skip_conflicts, .. }) => {
            assert!(merge);
            assert!(skip_conflicts);
        }
        _ => panic!("Expected Apply command"),
    }
}
```

**Step 6: Run tests**

Run: `cargo test`
Expected: All tests PASS

**Step 7: Run clippy**

Run: `cargo clippy`
Expected: May warn about unused `merge` parameter — that's fine, we'll use it in Task 5. Add `#[allow(unused)]` temporarily if needed, or just accept the warning.

**Step 8: Commit**

```bash
git add src/cli.rs src/lib.rs
git commit -m "feat(cli): add --merge flag and REPOVERLAY_MERGE env var

Adds --merge to apply, restore, update, and switch commands. Threads
the merge boolean through apply_overlay, apply_resolved_overlay, and
apply_multiple_overlays. Flag is combinable with --force/--skip-conflicts.
Env var REPOVERLAY_MERGE=1 implies --merge."
```

---

### Task 5: Wire up merge logic at conflict detection points

This is the core task — integrate the merge module into the apply flow.

**Files:**
- Modify: `src/lib.rs` (conflict detection sections in `apply_resolved_overlay`)

**Step 1: Add the use import**

At the top of `src/lib.rs`, add:

```rust
use json_merge::{is_json_file, merge_json_files};
```

**Step 2: Modify cross-overlay file conflict (lines 1133-1154)**

Replace the existing cross-overlay file conflict block with merge-aware logic. The key change: when `merge` is true and the file is `.json`, merge instead of failing.

Current code (lines 1133-1154):
```rust
if let Some(conflicting_overlay) = existing_targets.get(&target_rel_str) {
    match conflict_strategy {
        ConflictStrategy::SkipConflicts => { /* skip */ }
        ConflictStrategy::Fail | ConflictStrategy::Force => { /* bail */ }
    }
}
```

Replace with:
```rust
if let Some(conflicting_overlay) = existing_targets.get(&target_rel_str) {
    if merge && is_json_file(&target_rel) && target_file.exists() {
        // Deep merge JSON files
        eprintln!(
            "  {} Merging '{}' (managed by overlay '{}')",
            "Merge:".cyan(),
            target_rel.display(),
            conflicting_overlay
        );
        match merge_json_files(&target_file, &source_file, &target_file) {
            Ok(result) => {
                log_merge_result(&target_rel, &result);
                state.add_file(FileEntry {
                    source: rel_path.clone(),
                    target: target_rel.clone(),
                    link_type: LinkType::Merged,
                    entry_type: EntryType::File,
                });
                let exclude_path = target_rel.to_string_lossy().replace('\\', "/");
                exclude_entries.push(exclude_path);
                continue;
            }
            Err(e) => {
                eprintln!(
                    "  {} JSON merge failed for '{}': {e}",
                    "Warning:".yellow(),
                    target_rel.display()
                );
                // Fall through to existing conflict handling
            }
        }
    }
    match conflict_strategy {
        ConflictStrategy::SkipConflicts => {
            eprintln!(
                "  {} Skipping file '{}' (managed by overlay '{}')",
                "Skip:".yellow(),
                target_rel.display(),
                conflicting_overlay
            );
            continue;
        }
        ConflictStrategy::Fail | ConflictStrategy::Force => {
            bail!(
                "Conflict: file '{}' is already managed by overlay '{}'\n\
                 Remove that overlay first, use --skip-conflicts, or use different file mappings.",
                target_rel.display(),
                conflicting_overlay
            );
        }
    }
}
```

**Step 3: Modify repo-file conflict (lines 1156-1185)**

Add merge handling before the existing conflict strategy match. Insert the merge block before the existing `match conflict_strategy`:

```rust
if target_file.exists() {
    if merge && is_json_file(&target_rel) {
        eprintln!(
            "  {} Merging '{}' with existing repo file",
            "Merge:".cyan(),
            target_rel.display()
        );
        match merge_json_files(&target_file, &source_file, &target_file) {
            Ok(result) => {
                log_merge_result(&target_rel, &result);
                state.add_file(FileEntry {
                    source: rel_path.clone(),
                    target: target_rel.clone(),
                    link_type: LinkType::Merged,
                    entry_type: EntryType::File,
                });
                let exclude_path = target_rel.to_string_lossy().replace('\\', "/");
                exclude_entries.push(exclude_path);
                continue;
            }
            Err(e) => {
                eprintln!(
                    "  {} JSON merge failed for '{}': {e}",
                    "Warning:".yellow(),
                    target_rel.display()
                );
                // Fall through to existing conflict handling
            }
        }
    }
    match conflict_strategy {
        // ... existing Force/SkipConflicts/Fail branches unchanged
    }
}
```

**Step 4: Add the log_merge_result helper**

Add this function somewhere in `src/lib.rs` (near the bottom, before tests):

```rust
/// Log detailed merge results for a JSON file.
fn log_merge_result(path: &Path, result: &json_merge::MergeResult) {
    use colored::Colorize;

    println!(
        "  {} {} ({} added, {} overridden, {} type {})",
        "~".cyan(),
        path.display(),
        result.keys_added,
        result.keys_overridden,
        result.type_mismatches.len(),
        if result.type_mismatches.len() == 1 { "mismatch" } else { "mismatches" }
    );

    for mismatch in &result.type_mismatches {
        eprintln!(
            "    {} Type mismatch at '{}': {} -> {} (overlay wins)",
            "Warning:".yellow(),
            mismatch.key_path,
            mismatch.base_type,
            mismatch.overlay_type
        );
    }
}
```

**Step 5: Run tests**

Run: `cargo test`
Expected: All existing tests PASS (merge is off by default, so no behavior change)

**Step 6: Run clippy**

Run: `cargo clippy`
Expected: No warnings

**Step 7: Commit**

```bash
git add src/lib.rs
git commit -m "feat: wire up JSON deep merge at conflict detection points

When --merge is active and a .json file conflicts (cross-overlay or
repo-file), reads both files, deep merges them, and writes the result
as a regular file. Falls through to existing conflict handling if merge
fails or file is not JSON. Logs merge statistics and type mismatches."
```

---

### Task 6: Integration tests for JSON merge

**Files:**
- Modify: `tests/cli.rs` (add new test functions)

**Step 1: Write test for repo-file JSON merge**

```rust
#[test]
fn merge_json_with_existing_repo_file() {
    let ctx = TestContext::new()
        .with_overlay(&[("settings.json", r#"{"overlay_key": "overlay_value", "shared": "from_overlay"}"#)]);

    // Create existing JSON in repo
    ctx.create_repo_file("settings.json", r#"{"repo_key": "repo_value", "shared": "from_repo"}"#);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--copy", "--merge"])
        .assert()
        .success();

    let content: serde_json::Value =
        serde_json::from_str(&ctx.read_file("settings.json")).unwrap();
    assert_eq!(content["repo_key"], "repo_value"); // preserved from base
    assert_eq!(content["overlay_key"], "overlay_value"); // added from overlay
    assert_eq!(content["shared"], "from_overlay"); // overlay wins
}
```

**Step 2: Write test for merge without flag fails normally**

```rust
#[test]
fn json_conflict_without_merge_flag_fails() {
    let ctx = TestContext::new()
        .with_overlay(&[("settings.json", r#"{"key": "value"}"#)]);

    ctx.create_repo_file("settings.json", r#"{"existing": true}"#);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--copy"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Conflict"));
}
```

**Step 3: Write test for merge with non-JSON file falls through**

```rust
#[test]
fn merge_flag_ignored_for_non_json_files() {
    let ctx = TestContext::new()
        .with_overlay(&[("config.txt", "overlay content")]);

    ctx.create_repo_file("config.txt", "repo content");

    // --merge doesn't help non-JSON files, still fails
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--copy", "--merge"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Conflict"));
}
```

**Step 4: Write test for merge + force combo**

```rust
#[test]
fn merge_with_force_merges_json_and_forces_others() {
    let ctx = TestContext::new()
        .with_overlay(&[
            ("settings.json", r#"{"overlay": true}"#),
            ("readme.txt", "overlay readme"),
        ]);

    ctx.create_repo_file("settings.json", r#"{"repo": true}"#);
    ctx.create_repo_file("readme.txt", "repo readme");

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--copy", "--merge", "--force"])
        .assert()
        .success();

    // JSON was merged
    let json: serde_json::Value =
        serde_json::from_str(&ctx.read_file("settings.json")).unwrap();
    assert_eq!(json["repo"], true);
    assert_eq!(json["overlay"], true);

    // Non-JSON was force-overwritten
    assert_eq!(ctx.read_file("readme.txt"), "overlay readme");
}
```

**Step 5: Write test for REPOVERLAY_MERGE env var**

```rust
#[test]
fn repoverlay_merge_env_var_enables_merge() {
    let ctx = TestContext::new()
        .with_overlay(&[("settings.json", r#"{"overlay": true}"#)]);

    ctx.create_repo_file("settings.json", r#"{"repo": true}"#);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--copy"])
        .env("REPOVERLAY_MERGE", "1")
        .assert()
        .success();

    let json: serde_json::Value =
        serde_json::from_str(&ctx.read_file("settings.json")).unwrap();
    assert_eq!(json["repo"], true);
    assert_eq!(json["overlay"], true);
}
```

**Step 6: Add serde_json to dev-dependencies if not already in scope**

Check if `serde_json` is available in integration tests. If not, it needs to be added as a `[dev-dependencies]` entry in Cargo.toml:

```toml
[dev-dependencies]
serde_json = "1.0"
```

(It may already be available since it's a regular dependency.)

**Step 7: Run all tests**

Run: `cargo test`
Expected: All tests PASS

**Step 8: Run clippy**

Run: `cargo clippy`
Expected: No warnings

**Step 9: Commit**

```bash
git add tests/cli.rs Cargo.toml
git commit -m "test: add integration tests for JSON deep merge

Tests cover: repo-file merge, merge-without-flag failure, non-JSON
fallthrough, merge+force combo, and REPOVERLAY_MERGE env var activation."
```

---

### Task 7: Run full check suite and final cleanup

**Files:**
- Possibly modify any files with clippy warnings

**Step 1: Run full checks**

Run: `just check`
Expected: All checks pass (format, lint, test)

**Step 2: Fix any issues**

Address any clippy warnings, formatting issues, or test failures.

**Step 3: Commit any fixes**

```bash
git add -A
git commit -m "fix: address clippy warnings and formatting"
```

(Only if needed.)

---

## Summary of Changes

| File | Change |
|------|--------|
| `Cargo.toml` | Add `serde_json = "1.0"` |
| `src/json_merge.rs` | New module: deep merge logic, helpers, unit tests |
| `src/state.rs` | Add `LinkType::Merged` variant |
| `src/lib.rs` | Thread `merge: bool`, add merge handling at conflict points, `log_merge_result` helper |
| `src/cli.rs` | Add `--merge` flag to Apply/Restore/Update/Switch, CLI parsing tests |
| `tests/cli.rs` | Integration tests for merge behavior |
