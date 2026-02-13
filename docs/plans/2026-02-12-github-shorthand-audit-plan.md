# GitHub Shorthand Audit Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `validate_source_url()` accept bare owner names (e.g., `tylerbutler`) and expand them to `https://github.com/tylerbutler/repo-overlays`, so all URL entry points support this consistently.

**Architecture:** Extend the existing three-function chain in `config.rs` (`is_git_url` / `is_github_shorthand` / `validate_source_url`) with a new `is_bare_owner()` predicate. Move the `DEFAULT_OVERLAY_REPO_NAME` constant from `lib.rs` to `config.rs`. All callers (`source add`, serde deserialization) get the fix automatically.

**Tech Stack:** Rust, sickle (CCL parser), cargo test

---

### Task 1: Move DEFAULT_OVERLAY_REPO_NAME to config.rs

**Files:**
- Modify: `src/config.rs:68` (add constant before `is_git_url`)
- Modify: `src/lib.rs:79` (replace local constant with import)

**Step 1: Add the constant to config.rs**

In `src/config.rs`, add this line immediately before the `is_git_url` function (line 69):

```rust
/// Default overlay repository name for the one-part shorthand syntax.
/// When user types `username`, it expands to `username/repo-overlays`.
pub const DEFAULT_OVERLAY_REPO_NAME: &str = "repo-overlays";
```

**Step 2: Update lib.rs to use the config constant**

In `src/lib.rs`, replace line 77-79:

```rust
/// Default overlay repository name for the one-part shorthand syntax.
/// When user types `username`, it expands to `username/repo-overlays`.
const DEFAULT_OVERLAY_REPO_NAME: &str = "repo-overlays";
```

with:

```rust
use crate::config::DEFAULT_OVERLAY_REPO_NAME;
```

Note: `lib.rs` already has `use crate::config;` or similar imports. Check existing imports and add this use statement near them. If `config` is already imported as a module, use `config::DEFAULT_OVERLAY_REPO_NAME` inline at the two usage sites (lines ~221 and ~225) instead of a separate `use`.

**Step 3: Verify it compiles**

Run: `cargo check` in `/Volumes/Code/claude-workspace-ccl/repoverlay`
Expected: Compiles with no errors.

**Step 4: Run tests to verify nothing broke**

Run: `cargo test` in `/Volumes/Code/claude-workspace-ccl/repoverlay`
Expected: All existing tests pass.

**Step 5: Commit**

```
feat(config): move DEFAULT_OVERLAY_REPO_NAME to config module

Shared constant needed by both config validation and lib resolution.
```

---

### Task 2: Add is_bare_owner() and extend validate_source_url()

**Files:**
- Modify: `src/config.rs:75-104` (add function, modify existing function)

**Step 1: Write failing tests for bare owner expansion**

In `src/config.rs`, in the `#[cfg(test)] mod tests` block, find the test `test_validate_source_url_bare_word_rejected` (around line 977). Replace it and add new tests after `test_validate_source_url_whitespace_rejected`:

```rust
#[test]
fn test_validate_source_url_bare_owner_expanded() {
    let result = validate_source_url("tylerbutler");
    assert_eq!(
        result.unwrap(),
        "https://github.com/tylerbutler/repo-overlays"
    );
}

#[test]
fn test_validate_source_url_bare_owner_with_hyphens() {
    let result = validate_source_url("my-org");
    assert_eq!(
        result.unwrap(),
        "https://github.com/my-org/repo-overlays"
    );
}

#[test]
fn test_validate_source_url_empty_rejected() {
    let result = validate_source_url("");
    assert!(result.is_err());
}

#[test]
fn test_validate_source_url_whitespace_only_rejected() {
    let result = validate_source_url("  ");
    assert!(result.is_err());
}

#[test]
fn test_validate_source_url_bare_owner_with_whitespace_rejected() {
    let result = validate_source_url("tyler butler");
    assert!(result.is_err());
}
```

Also update `test_deserialize_source_with_bare_word_fails` (around line 1012) - it should now **succeed** instead of fail. Rename and update:

```rust
#[test]
fn test_deserialize_source_with_bare_owner_expands() {
    let ccl = r"
sources =
  =
    name = personal
    url = tylerbutler
";
    let config: RepoverlayConfig = sickle::from_str(ccl).unwrap();
    assert_eq!(config.sources.len(), 1);
    assert_eq!(
        config.sources[0].url,
        "https://github.com/tylerbutler/repo-overlays"
    );
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib config::tests::test_validate_source_url_bare_owner` in `/Volumes/Code/claude-workspace-ccl/repoverlay`
Expected: FAIL - bare owner returns Err instead of expanded URL.

**Step 3: Add is_bare_owner() and update validate_source_url()**

In `src/config.rs`, after the `is_github_shorthand` function (after line 82), add:

```rust
/// Check if a string is a bare owner name (single word, no slashes).
///
/// Valid: `tylerbutler`, `my-org`, `user123`
/// Invalid: empty, whitespace, contains `/`
fn is_bare_owner(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains(char::is_whitespace)
}
```

Then update `validate_source_url` (lines 94-104) to:

```rust
/// Validate and normalize a source URL string.
///
/// Accepts:
/// - Full git URLs (`https://...`, `git@...`)
/// - GitHub shorthand (`owner/repo`) - expanded to `https://github.com/owner/repo`
/// - Bare owner name (`owner`) - expanded to `https://github.com/owner/repo-overlays`
///
/// Returns an error for invalid formats (empty, whitespace).
pub fn validate_source_url(url: &str) -> std::result::Result<String, String> {
    if is_git_url(url) {
        Ok(url.to_string())
    } else if is_github_shorthand(url) {
        Ok(expand_github_shorthand(url))
    } else if is_bare_owner(url) {
        Ok(expand_github_shorthand(&format!(
            "{url}/{DEFAULT_OVERLAY_REPO_NAME}"
        )))
    } else {
        Err(format!(
            "Invalid source URL: '{url}'. Expected a git URL (https://...), \
             GitHub shorthand (owner/repo), or a GitHub username (owner)."
        ))
    }
}
```

**Step 4: Run all tests**

Run: `cargo test` in `/Volumes/Code/claude-workspace-ccl/repoverlay`
Expected: All tests pass, including the new bare owner tests.

**Step 5: Run clippy**

Run: `cargo clippy` in `/Volumes/Code/claude-workspace-ccl/repoverlay`
Expected: No warnings.

**Step 6: Commit**

```
feat(config): support bare owner names in source URL validation (#82)

validate_source_url() now accepts bare owner names (e.g., "tylerbutler")
and expands them to "https://github.com/tylerbutler/repo-overlays".
This makes `repoverlay source add tylerbutler` work as expected.
```

---

### Task 3: Update source add help text and error messages

**Files:**
- Modify: `src/cli.rs:404-406` (update arg help text)

**Step 1: Update the help text for the url argument**

In `src/cli.rs`, the `SourceCommand::Add` variant (around line 404-406):

```rust
    Add {
        /// Git URL of the overlay repository
        url: String,
```

Change the doc comment to:

```rust
    Add {
        /// Git URL, GitHub shorthand (owner/repo), or GitHub username
        url: String,
```

**Step 2: Verify it compiles**

Run: `cargo check` in `/Volumes/Code/claude-workspace-ccl/repoverlay`
Expected: Compiles.

**Step 3: Commit**

```
docs(cli): update source add help text for shorthand formats (#82)
```
