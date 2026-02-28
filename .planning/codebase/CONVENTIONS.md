# Coding Conventions

**Analysis Date:** 2026-02-27

## Naming Patterns

**Files:**
- `src/*.rs` - Module files use lowercase with underscores: `overlay_repo.rs`, `state.rs`, `github.rs`
- `src/main.rs` - CLI entry point (minimal, delegates to lib)
- `src/lib.rs` - Core library with public API
- `tests/cli.rs` - Integration tests
- `tests/common/mod.rs` - Shared test utilities

**Functions:**
- Lowercase with underscores: `apply_overlay()`, `remove_single_overlay()`, `validate_path_component()`
- Helper functions follow action-noun pattern: `get_current_commit()`, `ensure_cloned()`, `needs_clone()`
- Test functions start with `test_` or use `#[test]` attribute, describe behavior: `apply_and_remove_workflow()`, `apply_with_copy_flag()`

**Variables:**
- Lowercase with underscores for locals: `overlay_path`, `source_name`, `config_dir`
- Constants use UPPERCASE_WITH_UNDERSCORES: `STATE_DIR`, `OVERLAYS_DIR`, `DEFAULT_OVERLAY_REPO_NAME`
- Single-letter variables limited to loops: `for ms in &self.sources` (ms = ManagedSource)

**Types:**
- PascalCase for structs, enums: `TestContext`, `OverlayState`, `ConflictStrategy`, `OverlaySource`
- Generic type parameters are single uppercase letters: `T`, `E`
- Enum variants: PascalCase: `Interactive`, `SkipConflicts`, `Force`

**Imports:**
- Group imports by category:
  1. Standard library: `use std::fs;`, `use std::path::{Path, PathBuf};`
  2. External crates: `use anyhow::{Context, Result, bail};`, `use clap::{Parser, Subcommand};`
  3. Internal modules: `use crate::OverlayName;`, `use crate::state::{...};`
- Use qualified imports for re-exporting: `pub(crate) use overlay_name::OverlayName;`

## Code Style

**Formatting:**
- Max line width: 100 characters (`rustfmt.toml`)
- Tab width: 4 spaces
- Rust 2024 edition
- Uses default heuristics for breaking

**Linting:**
- Clippy configured with: `all`, `pedantic`, `nursery` lints enabled
- Severity level: warn
- Allowed deviations:
  - `missing_errors_doc`, `missing_panics_doc` - Disabled for noise
  - `module_name_repetitions` - Disabled (natural in modular design)
  - `similar_names` - Disabled
  - `too_many_lines` - Disabled
  - `cognitive_complexity` - Disabled
  - `significant_drop_tightening` - Disabled

**Documentation:**
- Module-level doc comments at top of each file: `//! Module description`
- Function/struct doc comments: `///` style with full documentation
- Doc comments explain "what" and "why", not "how"
- Include examples in doc comments when behavior is non-obvious
- Public types include `#[derive(Debug)]` and full documentation
- Use rustdoc markdown syntax

Example from `src/config.rs`:
```rust
//! Configuration management for repoverlay.
//!
//! Handles global and per-repo configuration using CCL format.
//! Global config: `~/.config/repoverlay/config.ccl`
//! Per-repo config: `.repoverlay/config.ccl`

/// Global repoverlay configuration.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct RepoverlayConfig {
    /// Configured overlay sources (checked in order for resolution).
    #[serde(default)]
    pub sources: Vec<Source>,
}
```

## Error Handling

**Pattern:**
- Use `anyhow::Result<T>` for all fallible operations
- Use `anyhow::bail!()` for immediate error returns with custom messages
- Use `.context()` or `.with_context()` to add contextual information
- Never unwrap in production code without documentation

Example from `src/config.rs`:
```rust
pub fn get_default_overlay_repo_config(&self) -> Result<OverlayRepoConfig> {
    let source = self.sources.first().ok_or_else(|| {
        anyhow::anyhow!(
            "Overlay repository not configured.\n\n\
             Run 'repoverlay source add <url>' to set up an overlay source."
        )
    })?;
    // ...
}
```

**Error Messages:**
- User-facing errors include helpful context
- Multi-line error messages for complex issues use `\n` explicitly
- Include recovery instructions when possible

**Result Type:**
- Always use `Result<T>` = `anyhow::Result<T>` (imported in prelude via lib.rs)
- Functions returning `Result` should document potential error cases in doc comments

## Logging

**Framework:** `log` crate with `env_logger` initialization

**Patterns:**
- `log::debug!()` - Detailed diagnostic information
- `log::trace!()` - Very detailed tracing
- Use in critical operations: caching, source resolution, file operations
- No `log::info!()` or `log::warn!()` in library code (only stderr/stdout for CLI)

Example from `src/state.rs`:
```rust
use log::debug;
debug!("Loading overlay state from {:?}", state_file_path);
```

## Comments

**When to Comment:**
- Complex algorithms or non-obvious logic (what makes this necessary?)
- Important invariants or preconditions
- Workarounds for quirks in dependencies
- Sections that separate logical blocks in large functions

**What NOT to Comment:**
- Self-explanatory code: `let config_file = path.join("config.ccl");` needs no comment
- Obvious function behavior already described in doc comments

**Example:**
```rust
// Must use separator to avoid false matches like ".claude-backup" matching ".claude"
let pattern_with_sep = format!("{pattern}/");
if path_str.starts_with(&pattern_with_sep) {
    return true;
}
```

## Function Design

**Size:**
- Target < 100 lines per function
- Break complex operations into helper functions
- Example: `apply_overlay()` is ~200 lines but handles multiple phases (conflict checking, file copying, state saving)

**Parameters:**
- Prefer named parameters over booleans when 3+ parameters
- Use enums for mutually exclusive options: `ConflictStrategy` enum instead of 4 boolean flags

**Return Values:**
- Return `Result<T>` for fallible operations
- Return `Option<T>` for optional values (preferred over `Result` when no error context needed)
- Use tuple returns only for related values that move together

**Builder/Constructor Pattern:**
- Simple constructors: Use `impl Type { pub fn new(...) -> Self { } }`
- Complex builders: Chain methods returning `self`

Example from `tests/common/mod.rs`:
```rust
pub fn with_overlay(mut self, files: &[(&str, &str)]) -> Self {
    self.overlay = Some(create_overlay_dir(files));
    self
}
```

## Module Design

**Module Structure:**
- One primary type per module when possible
- Helper types grouped with their consumers
- Test utilities in separate submodule: `#[cfg(test)] mod testutil;`

**Exports:**
- `pub` for public API
- `pub(crate)` for internal APIs (used across modules)
- Private otherwise (no `pub(super)` used)
- Re-export key types in lib.rs for simpler imports

Example from `src/lib.rs`:
```rust
pub(crate) use overlay_name::OverlayName;
use cache::CacheManager;
use config::config;
```

**Visibility Strategy:**
- Keep modules private; expose via public functions in lib.rs
- Minimal public surface: only operations and key domain types
- lib.rs imports privately, re-exports publicly what's needed

## Struct and Enum Design

**Derive Traits:**
- Standard: `#[derive(Debug, Clone)]` for most structs
- Serialization: `#[derive(Deserialize, Serialize)]` for config/state types
- Comparison: `#[derive(PartialEq, Eq)]` for types used as keys or tested for equality
- Copy: Rare; only for very small types (< 24 bytes)

Example from `src/state.rs`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedVia {
    Direct,
    Upstream,
}
```

**Tagged Enums:**
- Use `#[serde(tag = "type")]` for discriminated unions in serialization
- Provides clear serialization boundaries

Example from `src/state.rs`:
```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum OverlaySource {
    Local { path: PathBuf },
    GitHub { url: String, owner: String, ... },
    OverlayRepo { org: String, repo: String, ... },
}
```

## Testing Organization

- Unit tests: Embedded in modules (`#[cfg(test)] mod tests { }`)
- Integration tests: `tests/` directory
- Shared test utilities: `tests/common/mod.rs` and `src/testutil.rs`
- Test fixtures: Defined as functions returning test data (`envrc_overlay()`, `nested_overlay()`)

---

*Convention analysis: 2026-02-27*
