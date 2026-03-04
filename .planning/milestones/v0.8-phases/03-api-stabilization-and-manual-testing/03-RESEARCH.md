# Phase 3: API Stabilization and Manual Testing - Research

**Researched:** 2026-03-03
**Domain:** Rust API visibility auditing and CLI manual test documentation
**Confidence:** HIGH

## Summary

Phase 3 covers two distinct workstreams: (1) locking down the public API surface for a binary-only 1.0 release, and (2) creating manual test documents for all CLI workflows.

The API work is straightforward. The codebase already has `pub(crate)` used extensively in `lib.rs` (30+ items), but other modules expose items as `pub` that are only consumed within the crate. Since this is a binary-only release with no external library consumers, every `pub` item outside of `main.rs -> lib::run()` should become `pub(crate)`. The `testutil` module is already `#[cfg(test)]` gated, so its `pub` items are internal-only. All 17 source modules already have `//!` module-level doc comments, so API-03 requires only review and potential enhancement rather than writing from scratch.

The manual test work is creating 8 markdown files covering the CLI workflows. The 8 commands map to the CONTEXT.md decisions: apply, remove, status, restore, update, create, switch/browse, and source management. The CLI actually has additional commands (cache, sync, edit, browse, completions) but the manual test requirements scope is limited to the 8 specified.

**Primary recommendation:** Systematic `pub` -> `pub(crate)` conversion via grep-audit, then sequential manual test document creation for each CLI workflow.

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions
- Binary-only for 1.0 -- no external library consumers
- Convert all `pub` items to `pub(crate)` across the entire codebase
- No external API commitment -- library API deferred to future version
- Manual review of pub items (grep + audit), no semver-checks tooling
- Structure internals so a library API could be added later, but don't design it now
- Skip `#[non_exhaustive]` entirely for 1.0 (no external consumers means it has no effect)
- Module-level `//!` docs only -- no per-item documentation
- Freeform descriptions explaining module purpose and role
- No rustdoc examples or usage notes for 1.0
- Hybrid markdown with embedded copy-pasteable command blocks and expected output sections
- One file per CLI command (8 files: apply.md, remove.md, status.md, restore.md, update.md, create.md, switch-browse.md, source-management.md)
- Location: `docs/manual-tests/` in the repository root
- Local overlays primary (no network required), GitHub-specific tests marked as optional/requires-network sections

### Claude's Discretion
- testutil module gating approach (cfg(test) vs pub(crate))
- Module doc format per module (short for simple modules, more detail for complex ones)
- Manual test document structure within each file (section ordering, setup instructions)

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| API-01 | Public API surface reviewed (pub vs pub(crate) boundaries) | Audit of 102 `pub` items across 16 modules; only `lib::run()` needs to stay `pub` |
| API-02 | `#[non_exhaustive]` added to extensible public enums and structs | Skipped per user decision -- binary-only, no external consumers |
| API-03 | Public API documented with appropriate doc comments | All 17 modules already have `//!` docs; review and enhance per module complexity |
| MANUAL-01 | CLI walkthrough test cases for apply command | Apply command has 11 flags; test local + GitHub sources, conflict modes |
| MANUAL-02 | CLI walkthrough test cases for remove command | Remove has name, --all, --interactive, --dry-run flags |
| MANUAL-03 | CLI walkthrough test cases for status command | Status has --json, --quiet, filter-by-name modes |
| MANUAL-04 | CLI walkthrough test cases for restore command | Restore has conflict resolution flags matching apply |
| MANUAL-05 | CLI walkthrough test cases for update command | Update has per-overlay and all-overlay modes with conflict flags |
| MANUAL-06 | CLI walkthrough test cases for create command | Create has interactive and --include modes, --output for local |
| MANUAL-07 | CLI walkthrough test cases for switch/browse commands | Switch removes all + applies new; browse lists available overlays |
| MANUAL-08 | CLI walkthrough test cases for source management (add, list, remove) | Three subcommands under `repoverlay source` |
</phase_requirements>

## Standard Stack

### Core
| Tool | Purpose | Why Standard |
|------|---------|--------------|
| `rg` (ripgrep) | Grep for pub items across codebase | Fast, already in project toolchain |
| Rust compiler | Validates `pub(crate)` changes compile | Catches any missed cross-module references |

### Supporting
No additional libraries needed. This phase is audit-and-document work, not implementation.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Manual grep audit | cargo-semver-checks | User explicitly chose manual review; no semver-checks tooling |
| Per-item doc comments | Module-only `//!` docs | User chose module-level only for 1.0 |

## Architecture Patterns

### pub -> pub(crate) Conversion Pattern

**What:** Change all `pub` items to `pub(crate)` except the single entry point `lib::run()`.

**Current state by module (pub item counts):**
| Module | `pub` items | Notes |
|--------|-------------|-------|
| state.rs | 27 | Largest: enums, structs, functions for state management |
| config.rs | 16 | Structs, constants, functions for config |
| detection.rs | 12 | File detection enums, functions, constants |
| selection.rs | 9 | UI types and functions |
| testutil.rs | 8 | Already `#[cfg(test)]`; pub items are test-internal |
| overlay_repo.rs | 6 | Manager struct and helpers |
| json_merge.rs | 5 | Merge types and functions |
| cache.rs | 5 | Cache structs and functions |
| upstream.rs | 4 | Upstream detection types |
| github.rs | 3 | GitHub parsing types |
| sources.rs | 2 | Source resolution types |
| fuzzy.rs | 2 | Fuzzy matching |
| reference.rs | 1 | Source reference enum |
| overlay_name.rs | 1 | Newtype wrapper |
| lib.rs | 1 | `pub fn run()` -- KEEP pub |
| cli.rs | 1 | `pub fn run()` -- used by lib.rs internally |

**Total: ~102 pub items to audit; ~101 should become pub(crate).**

**Pattern:**
```rust
// Before
pub struct OverlayState { ... }
pub fn save_overlay_state(...) -> Result<()> { ... }

// After
pub(crate) struct OverlayState { ... }
pub(crate) fn save_overlay_state(...) -> Result<()> { ... }
```

**Key edge case -- testutil.rs:**
- Already gated with `#[cfg(test)]` at module declaration in lib.rs
- Items inside are `pub` but only visible during test compilation
- Recommendation: Leave as `pub` since `cfg(test)` already scopes them. Changing to `pub(crate)` is also fine but adds no practical difference since the module itself is test-only.

**Key edge case -- lib.rs `pub fn run()`:**
- Called from `main.rs` as `repoverlay::run()`
- This is the ONLY item that must remain `pub` (not `pub(crate)`)
- `cli::run()` is called from within `lib::run()`, so it can be `pub(crate)`

### Module Doc Enhancement Pattern

**What:** Review existing `//!` docs for adequacy per module complexity.

**Current state:** All 17 modules have `//!` docs. Most have 2-4 line descriptions. Examples:
- Simple (adequate): `fuzzy.rs` -- 4 lines explaining fuzzy matching purpose
- Complex (may need enhancement): `state.rs` -- 4 lines but covers persistence, external backup, CCL format, and multiple functions

**Recommendation:** Short modules (fuzzy, overlay_name, reference, json_merge) keep existing docs. Complex modules (state, config, lib, cli, sources, overlay_repo) may benefit from a sentence about key types or patterns, but keep within the "freeform, module purpose" constraint -- no per-item docs.

### Manual Test Document Structure

**What:** Each manual test file should be self-contained with setup, test steps, and expected output.

**Recommended structure per file:**
```markdown
# Command Name - Manual Test

## Prerequisites
- repoverlay installed and on PATH
- git installed

## Setup
[Common setup steps: create temp repo, create test overlay]

## Test Cases

### TC-01: [Scenario Name]
**Steps:**
1. [Step with copy-pasteable command]
2. [Verification step]

**Expected Output:**
[Expected terminal output or state]

### TC-02: [Next scenario]
...

## Optional: GitHub Source Tests (requires network)
[GitHub-specific test cases]
```

### Anti-Patterns to Avoid
- **Mixing pub and pub(crate) inconsistently:** Apply the conversion uniformly; don't leave some modules partially converted
- **Adding doc examples when user said no:** Module-level `//!` only, no `/// # Examples` blocks
- **Writing manual tests that require network by default:** Local overlays primary; GitHub tests in optional sections

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Finding pub items | Manual file-by-file reading | `rg '^pub (fn\|struct\|enum\|...)' src/` | Grep is faster and less error-prone |
| Verifying compilation | Run individual tests | `cargo check` after each module conversion | Catches missing visibility immediately |
| Manual test setup scripts | Complex bash scripts in test docs | Simple inline commands (mkdir, git init, echo) | Keep tests copy-pasteable by humans |

**Key insight:** This phase is audit and documentation work. The code changes are mechanical (pub -> pub(crate)), and the manual tests are creative writing. No complex tooling needed.

## Common Pitfalls

### Pitfall 1: Breaking integration tests with pub(crate)
**What goes wrong:** Integration tests in `tests/` cannot access `pub(crate)` items -- they can only see `pub` items from the crate root.
**Why it happens:** Integration tests are external to the crate; `pub(crate)` hides items from them.
**How to avoid:** The integration tests in `tests/cli.rs` use `assert_cmd` to test the binary via CLI. They don't import library items directly. Verify this by checking `tests/cli.rs` imports -- if it only uses `assert_cmd::Command` and process-level testing, the conversion is safe.
**Warning signs:** `cargo test` fails with "private item" errors after conversion.

### Pitfall 2: testutil visibility from unit tests
**What goes wrong:** Unit tests within `src/*.rs` that use `crate::testutil::*` need testutil items to be visible.
**Why it happens:** `pub` items in a `#[cfg(test)]` module are visible to other test modules within the same crate.
**How to avoid:** Keep testutil items as `pub` (they're already scoped by `#[cfg(test)]`). Only `lib.rs` unit tests currently use testutil (`use crate::testutil::TestContext` and `use crate::testutil::{create_test_overlay, create_test_repo}`).

### Pitfall 3: Manual tests that are not reproducible
**What goes wrong:** Test steps assume state from previous runs or specific system configuration.
**Why it happens:** Not starting from clean state; hardcoded paths.
**How to avoid:** Each test document starts with a fresh `mktemp -d` setup. All paths are relative to the temp directory. Cleanup instructions at the end.

### Pitfall 4: Forgetting `pub use` re-exports
**What goes wrong:** `lib.rs` has `pub(crate) use overlay_name::OverlayName;` -- if any modules use OverlayName, this re-export needs to remain accessible.
**Why it happens:** Re-exports can change visibility scope without touching the original definition.
**How to avoid:** After conversion, run `cargo check` and `cargo test` to catch any visibility errors.

## Code Examples

### pub to pub(crate) conversion for a module

```rust
// state.rs -- Before
pub enum ResolvedVia {
    Direct,
    Upstream,
}

pub struct OverlayState {
    pub name: String,
    // ...
}

pub fn save_overlay_state(target: &Path, state: &OverlayState) -> Result<()> {
    // ...
}

// state.rs -- After
pub(crate) enum ResolvedVia {
    Direct,
    Upstream,
}

pub(crate) struct OverlayState {
    pub(crate) name: String,
    // ...
}

pub(crate) fn save_overlay_state(target: &Path, state: &OverlayState) -> Result<()> {
    // ...
}
```

Note: Struct fields that are `pub` also need conversion to `pub(crate)` when the struct itself becomes `pub(crate)`.

### Manual test document example (apply command snippet)

```markdown
## Setup

    mkdir -p /tmp/repoverlay-test && cd /tmp/repoverlay-test
    git init test-repo && cd test-repo
    git commit --allow-empty -m "init"

    mkdir -p ../test-overlay
    echo 'export FOO=bar' > ../test-overlay/.envrc

### TC-01: Apply local overlay (symlink mode)

**Steps:**

    repoverlay apply ../test-overlay

**Expected Output:**

    Applied overlay "test-overlay" (1 file)
      .envrc -> ../test-overlay/.envrc (symlink)

**Verify:**

    ls -la .envrc
    # Should show symlink pointing to ../test-overlay/.envrc
    cat .git/info/exclude
    # Should contain repoverlay section markers
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `pub` everywhere | `pub(crate)` for binary crates | Rust convention, stable | Prevents accidental API surface |
| `#[non_exhaustive]` always | Skip for binary-only crates | N/A | No effect without library consumers |

## Open Questions

1. **Struct field visibility**
   - What we know: When a struct becomes `pub(crate)`, its `pub` fields should also become `pub(crate)` for consistency
   - What's unclear: Whether any struct fields are accessed directly from other modules (vs through methods)
   - Recommendation: Convert struct fields too; `cargo check` will catch any issues

2. **cli.rs `pub fn run()` visibility**
   - What we know: `cli::run()` is called from `lib::run()`, which is the only externally-visible entry point
   - What's unclear: Whether `cli::run()` is ever called directly from `main.rs`
   - Recommendation: Convert to `pub(crate)` -- it's called within the crate only (from lib.rs)

3. **Manual test scope for browse vs switch**
   - What we know: MANUAL-07 combines switch and browse into one document (switch-browse.md)
   - What's unclear: Whether browse deserves its own document given it's a separate command
   - Recommendation: Follow user decision -- single combined file `switch-browse.md`

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis of all 17 source modules in `src/`
- grep results for `pub` and `pub(crate)` patterns across entire codebase
- CONTEXT.md user decisions from discuss-phase
- ARCHITECTURE.md for module structure and data flows
- CLI help output for command listing and flag inventory

### Secondary (MEDIUM confidence)
- Rust visibility rules for `pub(crate)` in binary crates (established Rust idiom, training knowledge verified against codebase behavior)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - No external tooling needed; grep + Rust compiler
- Architecture: HIGH - Codebase fully analyzed; all pub items counted and categorized
- Pitfalls: HIGH - Integration test pattern verified; testutil gating confirmed
- Manual tests: HIGH - All 8 CLI commands enumerated with flags from source code

**Research date:** 2026-03-03
**Valid until:** 2026-04-03 (stable domain, no external dependencies)
