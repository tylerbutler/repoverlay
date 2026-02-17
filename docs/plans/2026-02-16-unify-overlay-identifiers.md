# Unify Overlay Identifier Types — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace raw `String`/`&str` overlay identifiers with structured types (`AvailableOverlay`, `OverlayName`) throughout the codebase to eliminate fragile string parsing and prevent format-mismatch bugs.

**Architecture:** Bottom-up approach — enrich `AvailableOverlay` with `Display` and utility methods, change leaf listing functions to return structured types, propagate upward through consumers, then introduce newtype wrappers. Each task produces a compilable, testable codebase.

**Tech Stack:** Rust 2024 edition, no new dependencies.

---

### Task 1: Add `Display` impl and `full_path()` to `AvailableOverlay`

**Files:**
- Modify: `src/overlay_repo.rs:47-58`
- Test: `src/overlay_repo.rs` (existing test module)

**Step 1: Add `Display` impl and `full_path()` method**

In `src/overlay_repo.rs`, after the `AvailableOverlay` struct definition (line 58), add:

```rust
impl std::fmt::Display for AvailableOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.org, self.repo, self.name)
    }
}

impl AvailableOverlay {
    /// Format the overlay path for display with the overlay name in bold.
    pub fn display_bold(&self) -> String {
        use owo_colors::OwoColorize;
        format!("{}/{}/{}", self.org, self.repo, self.name.bold())
    }
}
```

Also derive `PartialEq, Eq` on the struct for later comparison support:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableOverlay {
```

**Step 2: Write tests**

Add to the test module in `src/overlay_repo.rs`:

```rust
#[test]
fn available_overlay_display() {
    let o = AvailableOverlay {
        org: "microsoft".to_string(),
        repo: "FluidFramework".to_string(),
        name: "vscode-setup".to_string(),
        has_config: true,
    };
    assert_eq!(o.to_string(), "microsoft/FluidFramework/vscode-setup");
}
```

**Step 3: Run tests**

Run: `just test`
Expected: All tests pass.

**Step 4: Commit**

```
feat(overlay): add Display impl and display_bold to AvailableOverlay

Provides string conversion for AvailableOverlay, replacing the need
for format_overlay_path() and parse_overlay_path() round-trips.
```

---

### Task 2: Change `list_overlays_from_path()` to return `Vec<AvailableOverlay>`

**Files:**
- Modify: `src/lib.rs:479-494` (`list_overlays_from_path`)
- Modify: `src/lib.rs:462-474` (`list_overlays_from_cached_repo`)
- Modify: `src/lib.rs:3927-4007` (tests for `list_overlays_from_path`)

**Step 1: Update `list_overlays_from_path` signature and body**

Change `src/lib.rs:479-494` to:

```rust
fn list_overlays_from_path(repo_path: &Path) -> Result<Vec<AvailableOverlay>> {
    let mut overlays = Vec::new();

    // Walk the three-level structure: org/repo/overlay
    for (org_path, org_name) in visible_subdirs(repo_path)? {
        for (repo_dir, repo_name) in visible_subdirs(&org_path)? {
            for (overlay_path, overlay_name) in visible_subdirs(&repo_dir)? {
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

    overlays.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
    debug!("found {} overlays in path", overlays.len());
    Ok(overlays)
}
```

**Step 2: Update `list_overlays_from_cached_repo` return type**

Change `src/lib.rs:462` to:

```rust
fn list_overlays_from_cached_repo(owner: &str, repo: &str) -> Result<Vec<AvailableOverlay>> {
```

Body is unchanged — it delegates to `list_overlays_from_path`.

**Step 3: Add `use crate::overlay_repo::AvailableOverlay;`** at the top of `src/lib.rs` if not already present.

**Step 4: Update tests**

Update the test assertions in `list_overlays_from_path_with_nested_structure` (lib.rs:3927) and all sibling tests to compare against `AvailableOverlay` values or use `.to_string()`:

```rust
#[test]
fn list_overlays_from_path_with_nested_structure() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path();

    fs::create_dir_all(repo_path.join("microsoft/FluidFramework/vscode-setup")).unwrap();
    fs::create_dir_all(repo_path.join("microsoft/FluidFramework/ci-config")).unwrap();
    fs::create_dir_all(repo_path.join("tylerbutler/some-repo/my-overlay")).unwrap();

    let overlays = list_overlays_from_path(repo_path).unwrap();

    assert_eq!(overlays.len(), 3);
    assert_eq!(overlays[0].to_string(), "microsoft/FluidFramework/ci-config");
    assert_eq!(overlays[1].to_string(), "microsoft/FluidFramework/vscode-setup");
    assert_eq!(overlays[2].to_string(), "tylerbutler/some-repo/my-overlay");
}
```

Apply the same `.to_string()` pattern to the other `list_overlays_from_path_*` tests.

**Step 5: Run tests**

Run: `just test`
Expected: Compile errors in callers of `list_overlays_from_path` / `list_overlays_from_cached_repo` — these will be fixed in Task 3.

**Step 6: Do NOT commit yet** — continue to Task 3 to fix callers.

---

### Task 3: Update `resolve_two_part` callers to use `AvailableOverlay`

**Files:**
- Modify: `src/lib.rs:344-433` (the `resolve_two_part` function body)
- Modify: `src/lib.rs:530-536` (`format_overlay_path`)
- Modify: `src/lib.rs:542-570` (`select_overlays_interactive`)

**Step 1: Update `select_overlays_interactive` to accept `&[AvailableOverlay]`**

```rust
fn select_overlays_interactive(
    owner: &str,
    repo: &str,
    overlays: &[AvailableOverlay],
) -> Result<Vec<AvailableOverlay>> {
    use dialoguer::{MultiSelect, theme::ColorfulTheme};

    println!(
        "\n{} Select overlay(s) from {}/{} (Space to toggle, Enter to confirm):\n",
        "?".cyan().bold(),
        owner,
        repo
    );

    let display_items: Vec<String> = overlays.iter().map(|o| o.display_bold()).collect();

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .items(&display_items)
        .interact_opt()
        .context("Failed to show overlay picker")?;

    match selections {
        Some(indices) if !indices.is_empty() => {
            Ok(indices.into_iter().map(|i| overlays[i].clone()).collect())
        }
        _ => bail!("No overlays selected"),
    }
}
```

**Step 2: Update `format_overlay_path` to accept `&AvailableOverlay`**

```rust
fn format_overlay_path(overlay: &AvailableOverlay) -> String {
    overlay.display_bold()
}
```

Keep this as a thin wrapper for now — it will be removed later once all callers use `display_bold()` directly.

**Step 3: Update the `resolve_two_part` function body (lib.rs:344-433)**

Replace the string-based logic with structured `AvailableOverlay` access:

- Line 345: `available_overlays` is now `Vec<AvailableOverlay>` — no change needed.
- Line 357: `select_overlays_interactive` now returns `Vec<AvailableOverlay>` — update downstream.
- Lines 360-364: Non-interactive error formatting — use `o.display_bold()` instead of `format_overlay_path(o)`.
- Lines 371-385: Selected overlay display — use `selected.display_bold()`.
- Lines 394-401: Remove `parse_overlay_path` call — access `selected.org`, `selected.repo`, `selected.name` directly.
- Lines 403-407: Build overlay path from structured fields.
- Line 421: Build subpath from structured fields.

The loop at line 394 becomes:

```rust
for selected in &selected_overlays {
    let overlay_path = cached
        .path
        .join(&selected.org)
        .join(&selected.repo)
        .join(&selected.name);

    if !overlay_path.exists() {
        bail!("Overlay directory not found: {}", overlay_path.display());
    }

    resolved_sources.push(ResolvedSource {
        path: overlay_path,
        source_info: OverlaySource::github(
            github_url.clone(),
            owner.to_string(),
            repo.to_string(),
            git_ref_str.clone(),
            commit.clone(),
            Some(selected.to_string()),
        ),
    });
}
```

**Step 4: Run tests**

Run: `just test`
Expected: All tests pass.

**Step 5: Commit**

```
refactor(overlay): return AvailableOverlay from listing functions

Changes list_overlays_from_path and list_overlays_from_cached_repo
to return Vec<AvailableOverlay> instead of Vec<String>. Updates
select_overlays_interactive and resolve_two_part to thread the
structured type through, eliminating parse_overlay_path round-trips.

Closes phase 1 and phase 2 of #112.
```

---

### Task 4: Remove `parse_overlay_path` and simplify `format_overlay_path`

**Files:**
- Modify: `src/lib.rs` — delete `parse_overlay_path`, replace remaining `format_overlay_path` calls

**Step 1: Check for remaining callers of `parse_overlay_path`**

Search for `parse_overlay_path` in the codebase. After Task 3, it should only be referenced from `format_overlay_path` (which we already changed) and tests.

**Step 2: Remove `parse_overlay_path` function** (lib.rs:515-524)

Delete the function entirely.

**Step 3: Replace `format_overlay_path` with direct `display_bold()` calls**

Search for remaining `format_overlay_path` calls. Replace each with `.display_bold()` on the `AvailableOverlay`. Then delete `format_overlay_path`.

**Step 4: Remove any tests that tested `parse_overlay_path` directly**

These tests validated string decomposition that is no longer needed.

**Step 5: Run tests**

Run: `just test`
Expected: All tests pass.

**Step 6: Commit**

```
refactor(overlay): remove parse_overlay_path and format_overlay_path

These string utility functions are no longer needed now that listing
functions return AvailableOverlay with Display and display_bold().
```

---

### Task 5: Introduce `OverlayName` newtype

**Files:**
- Create: `src/overlay_name.rs`
- Modify: `src/lib.rs` (module declaration, update `list_applied_overlays` callers)
- Modify: `src/state.rs` (`list_applied_overlays` return type)

**Step 1: Create `src/overlay_name.rs`**

```rust
//! Newtype wrapper for normalized overlay names.

use std::fmt;

/// A normalized overlay name, as stored in `.ccl` file stems.
///
/// This newtype prevents accidental comparison between overlay names
/// and other string types (e.g., full three-part paths like `"org/repo/name"`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OverlayName(String);

impl OverlayName {
    /// Create a new `OverlayName` from a string.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Get the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OverlayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for OverlayName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for OverlayName {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for OverlayName {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_name_display() {
        let name = OverlayName::new("my-overlay");
        assert_eq!(name.to_string(), "my-overlay");
    }

    #[test]
    fn overlay_name_equality() {
        let a = OverlayName::new("foo");
        let b = OverlayName::new("foo");
        let c = OverlayName::new("bar");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn overlay_name_as_str() {
        let name = OverlayName::new("test");
        assert_eq!(name.as_str(), "test");
    }
}
```

**Step 2: Add module declaration in `src/lib.rs`**

Add `pub mod overlay_name;` alongside the other module declarations.

**Step 3: Update `list_applied_overlays` in `src/state.rs`**

```rust
pub fn list_applied_overlays(target: &Path) -> Result<Vec<OverlayName>> {
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        return Ok(Vec::new());
    }

    let mut names: Vec<OverlayName> = fs::read_dir(&overlays_dir)?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "ccl"))
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| OverlayName::new(s.to_string_lossy().to_string()))
        })
        .collect();

    names.sort();
    Ok(names)
}
```

Add `use crate::overlay_name::OverlayName;` to `src/state.rs`.

**Step 4: Fix all callers of `list_applied_overlays`**

There are ~18 call sites across `src/cli.rs` and `src/lib.rs`. For each:

- **String comparisons** like `applied.contains(&name)` → `applied.contains(&OverlayName::new(name))` or use `.as_str()` on the `OverlayName` side and compare.
- **Display/printing** like `println!("{}", name)` → works unchanged due to `Display` impl.
- **Iteration** like `for name in &applied` → works unchanged, but downstream `.as_str()` or `.to_string()` may be needed where `&str` is expected.

The key pattern: most callers do `applied_overlays.contains(&some_string)` or iterate for display. With `OverlayName`:
- `applied_overlays.iter().any(|n| n.as_str() == some_string)` for contains checks
- `name.as_str()` where `&str` is needed

**Step 5: Update tests in `src/state.rs`**

Update assertions to use `OverlayName`:

```rust
#[test]
fn test_list_applied_overlays_with_overlays() {
    // ... setup ...
    let overlays = list_applied_overlays(temp.path()).unwrap();
    assert_eq!(overlays.len(), 2);
    assert_eq!(overlays[0], OverlayName::new("bar"));
    assert_eq!(overlays[1], OverlayName::new("foo"));
}
```

**Step 6: Run tests**

Run: `just test`
Expected: All tests pass.

**Step 7: Commit**

```
refactor(overlay): introduce OverlayName newtype for normalized names

Wraps overlay names from .ccl file stems in a newtype to prevent
accidental comparison with full three-part path strings. Updates
list_applied_overlays and all 18+ call sites.

Closes #112.
```

---

### Task 6: Update `SourceManager::list_overlays_for_repo` to return `Vec<OverlayName>`

**Files:**
- Modify: `src/sources.rs:199-218`
- Modify: callers in `src/lib.rs:744-759` and `src/cli.rs:1036`

**Step 1: Update return type**

```rust
pub fn list_overlays_for_repo(&self, org: &str, repo: &str) -> Vec<OverlayName> {
    let mut names = std::collections::HashSet::new();

    for ms in &self.sources {
        if ms.manager.needs_clone() {
            continue;
        }

        if let Ok(overlays) = ms.manager.list_overlays_for_repo(org, repo) {
            for overlay in overlays {
                names.insert(OverlayName::new(overlay.name));
            }
        }
    }

    let mut result: Vec<_> = names.into_iter().collect();
    result.sort();
    result
}
```

**Step 2: Update callers**

- `get_fuzzy_suggestions_multi_source` (lib.rs:752-759): `fuzzy_suggest` takes `&[String]`. Convert with `.iter().map(|n| n.to_string()).collect()`, or update `fuzzy_suggest` to accept `&[impl AsRef<str>]`.
- `get_fuzzy_suggestions_legacy` (lib.rs:738-748): Already converts to `Vec<String>` via `.map(|o| o.name)`. Change to `.map(|o| OverlayName::new(o.name))` if `fuzzy_suggest` is updated, or keep as-is if `fuzzy_suggest` still takes `&[String]`.
- `cli.rs:1036`: This caller uses `OverlayRepoManager::list_overlays_for_repo` (not `SourceManager`), so it already returns `Vec<AvailableOverlay>`. No change needed.

The simplest approach: update `fuzzy_suggest` to accept `&[impl AsRef<str>]`:

```rust
fn fuzzy_suggest(query: &str, candidates: &[impl AsRef<str>]) -> Vec<String> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let matcher = OverlayMatcher::new();
    let str_candidates: Vec<&str> = candidates.iter().map(|c| c.as_ref()).collect();
    matcher.suggest(query, &str_candidates, 3)
}
```

Check what `OverlayMatcher::suggest` accepts and adapt accordingly.

**Step 3: Run tests**

Run: `just test`
Expected: All tests pass.

**Step 4: Commit**

```
refactor(overlay): return OverlayName from SourceManager::list_overlays_for_repo

Completes the type unification across all overlay listing paths.
fuzzy_suggest now accepts AsRef<str> for flexibility.
```

---

### Task 7: Final cleanup and verification

**Files:**
- Review: all modified files

**Step 1: Run full check suite**

Run: `just check`
Expected: Format, lint, and test all pass.

**Step 2: Search for remaining raw string overlay patterns**

Search for:
- `parse_overlay_path` — should be deleted
- `format_overlay_path` — should be deleted
- `Vec<String>` near "overlay" in function signatures — should be `Vec<AvailableOverlay>` or `Vec<OverlayName>`

**Step 3: Run clippy**

Run: `just lint`
Expected: No new warnings.

**Step 4: Commit any final cleanup**

```
refactor(overlay): final cleanup of string-based overlay identifiers
```
