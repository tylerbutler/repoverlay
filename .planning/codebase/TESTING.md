# Testing Patterns

**Analysis Date:** 2026-02-27

## Test Framework

**Runner:**
- cargo test (default)
- cargo nextest (faster parallel execution via `just test-fast`)

**Test Organization:**
- Integration tests: `tests/cli.rs` - CLI behavior using `assert_cmd`
- Shared utilities: `tests/common/mod.rs` - Test fixtures and contexts
- Library tests: `src/testutil.rs` - Helper module for creating test repos/overlays
- Unit tests: Embedded in modules within `#[cfg(test)]` blocks

**Run Commands:**
```bash
just test                 # Run all tests (builds binary first)
just test-fast            # Run with nextest (parallel)
just test-verbose         # Run with output shown (--nocapture)
just test-coverage        # Run with coverage (lcov.info)
cargo test <test_name>    # Run specific test
cargo test -- --test-threads=1  # Serial execution (for config tests)
```

## Test File Organization

**Location:**
- **`tests/cli.rs`** (1844 lines) - CLI integration tests, organized by command
- **`tests/common/mod.rs`** - Shared test utilities and fixtures
- **`src/testutil.rs`** - Test helper module (only compiled during tests via `#[cfg(test)]`)
- Unit tests embedded in source files with `#[cfg(test)] mod tests { }`

**Naming:**
- Describe behavior, not implementation: `apply_and_remove_workflow()` not `test_apply()`
- Use underscores for readability: `apply_with_explicit_name()`, `status_when_no_overlay()`
- Group related tests with comment headers: `// Apply Command Tests`, `// Remove Command Tests`

**Test Structure:**
```
tests/
├── cli.rs                # Main integration tests
│   ├── Help displays
│   ├── Version displays
│   ├── Apply Command Tests
│   ├── Remove Command Tests
│   └── etc.
└── common/
    └── mod.rs           # TestContext, SourceTestContext, fixtures
```

## Test Structure

**Test Suite Organization:**
```rust
#[test]
fn apply_and_remove_workflow() {
    // 1. Arrange: Set up context
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    // 2. Act: Run command
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--name", "test-overlay"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Applying"));

    // 3. Assert: Verify state
    assert!(ctx.file_exists(".envrc"));

    // 4. Cleanup: Context dropped automatically (TempDir cleanup)
}
```

**Patterns:**

1. **Arrange-Act-Assert (AAA):**
   - Arrange: Create test context with `TestContext::new().with_overlay(...)`
   - Act: Run CLI command via `cargo_bin_cmd!()` with `.assert()`
   - Assert: Verify outcomes with context helper methods

2. **Context Setup:**
   ```rust
   let ctx = TestContext::new()
       .with_overlay(&envrc_overlay());
   ```
   - Creates temp git repo and overlay directory
   - Builder pattern for chaining
   - Automatically cleaned up when `ctx` dropped

3. **CLI Invocation:**
   ```rust
   cargo_bin_cmd!("repoverlay")
       .args(["apply", ctx.overlay_source()])
       .args(["--target", ctx.repo_path().to_str().unwrap()])
       .assert()
       .success()
   ```
   - Uses `assert_cmd` crate for running binary
   - Builds command with `.args()` (takes iterator of &str)
   - Uses `predicates` crate for assertions

4. **Assertions:**
   - File operations: `ctx.file_exists()`, `ctx.is_symlink()`, `ctx.read_file()`
   - State checks: `ctx.state_dir_exists()`, `ctx.overlay_state_exists()`
   - CLI output: `.stdout(predicate::str::contains("..."))`, `.stderr(...)`
   - Exit codes: `.success()`, `.failure()`

## Test Fixtures and Factories

**Test Data:**
```rust
pub fn envrc_overlay() -> Vec<(&'static str, &'static str)> {
    vec![(".envrc", "export FOO=bar")]
}

pub fn nested_overlay() -> Vec<(&'static str, &'static str)> {
    vec![
        (".envrc", "export FOO=bar"),
        (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
    ]
}
```

**Location:**
- `tests/common/mod.rs` - CLI integration test fixtures
- `src/testutil.rs` - Library test helpers

**Usage:**
```rust
let ctx = TestContext::new()
    .with_overlay(&envrc_overlay());

let ctx = TestContext::new()
    .with_overlay(&[
        (".envrc", "export FOO=bar"),
        (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
    ]);
```

## Test Context Classes

**`TestContext` (tests/common/mod.rs):**
- Manages temporary git repo and overlay directory
- Builder pattern: `.new().with_overlay(...)`
- Helper methods for assertions:
  - File operations: `file_exists()`, `is_symlink()`, `read_file()`, `create_repo_file()`
  - State checks: `state_dir_exists()`, `overlay_state_exists(name)`
  - Git operations: `git_exclude_content()`

```rust
pub struct TestContext {
    pub repo: TempDir,
    overlay: Option<TempDir>,
}

impl TestContext {
    pub fn new() -> Self { /* ... */ }
    pub fn with_overlay(mut self, files: &[(&str, &str)]) -> Self { /* ... */ }
    pub fn file_exists(&self, path: &str) -> bool { /* ... */ }
    pub fn is_symlink(&self, path: &str) -> bool { /* ... */ }
    // ... more helpers
}
```

**`SourceTestContext` (tests/common/mod.rs):**
- Isolated config directory for source commands
- Sets `XDG_CONFIG_HOME` to prevent test interference

```rust
pub struct SourceTestContext {
    config_dir: TempDir,
}

impl SourceTestContext {
    pub fn new() -> Self { /* ... */ }
    pub fn cmd(&self) -> AssertCommand { /* command with isolated config */ }
}
```

**`TestContext` (src/testutil.rs):**
- Library version with `Default` impl
- Same methods as CLI test version
- Additional helpers: `create_test_repo()`, `create_test_overlay()`

## Mocking

**Framework:** None (uses real temporary filesystems)

**Approach:**
- Real filesystem via `tempfile::TempDir`
- Real git commands via `std::process::Command`
- Real binary execution via `assert_cmd` for integration tests
- No mocking of external services in test suite

**Why:**
- Correctness of git operations is critical
- Filesystem state management is central to repoverlay's purpose
- Integration tests verify actual behavior, not mock behavior

## Coverage

**Requirements:** No enforced minimum (developers can choose target)

**View Coverage:**
```bash
just test-coverage        # Run with coverage, generates lcov.info
just coverage-html        # Generate HTML report
just coverage-report      # Generate and open HTML report
```

**CI Coverage:**
- Coverage runs serially (`--test-threads=1`) to avoid race conditions in config tests
- LLVM-based coverage via `cargo-llvm-cov`

## Test Types

**Unit Tests:**
- Scope: Individual function/module behavior
- Approach: Call functions directly with specific inputs
- Example: Testing config parsing, overlay name normalization
- Location: Embedded in source files

Example (inferred from lib.rs structure):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_name_normalization() {
        // Test a single function in isolation
    }
}
```

**Integration Tests:**
- Scope: CLI commands end-to-end
- Approach: Run compiled binary via `cargo_bin_cmd!()`, verify filesystem state
- Example: Apply overlay → check files exist → remove overlay → check files gone
- Location: `tests/cli.rs` (1844 lines)

Example:
```rust
#[test]
fn apply_and_remove_workflow() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());
    // ... apply via CLI, assert files exist, remove, assert gone
}
```

**E2E Tests:**
- Not formally organized as separate category
- Integration tests effectively serve as E2E (run actual binary, real git repos)
- No separate "ui" or "acceptance" test tier

## Common Testing Patterns

**File Operations Testing:**
```rust
#[test]
fn apply_creates_symlink_by_default() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.is_symlink(".envrc"), ".envrc should be a symlink");
}
```

**State Management Testing:**
```rust
#[test]
fn apply_creates_state_directory() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.state_dir_exists(), ".repoverlay directory should exist");
    assert!(ctx.overlay_state_exists("custom-name"));
}
```

**Nested File Testing:**
```rust
#[test]
fn apply_nested_files() {
    let ctx = TestContext::new().with_overlay(&[
        (".envrc", "export FOO=bar"),
        (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
    ]);

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .assert()
        .success();

    assert!(ctx.file_exists(".envrc"));
    assert!(ctx.file_exists(".vscode/settings.json"));
}
```

**CLI Output Testing:**
```rust
#[test]
fn help_displays() {
    cargo_bin_cmd!("repoverlay")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Overlay config files"));
}
```

**Error Case Testing:**
```rust
#[test]
fn apply_requires_source_argument() {
    cargo_bin_cmd!("repoverlay")
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn apply_requires_git_repo() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());
    let temp_dir = tempfile::TempDir::new().unwrap();

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", temp_dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git"));
}
```

## Serial Execution

**When Needed:**
- Config tests that modify environment variables (`XDG_CONFIG_HOME`)
- Any test with global state dependencies

**How to Run:**
```bash
cargo test -- --test-threads=1
```

**In CI:**
- Coverage runs use `--test-threads=1` to avoid race conditions
- Regular test runs use default parallelism

---

*Testing analysis: 2026-02-27*
