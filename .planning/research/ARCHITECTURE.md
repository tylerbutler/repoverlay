# Architecture Research

**Domain:** Comprehensive code review and test improvement for a modular Rust CLI (repoverlay 1.0 stabilization)
**Researched:** 2026-02-27
**Confidence:** HIGH

## System Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          CLI Layer (cli.rs)                             │
│  Commands: apply, remove, status, restore, update, create, switch,     │
│            browse, edit, sync, source, cache, completions              │
├─────────────────────────────────────────────────────────────────────────┤
│                     Application Logic (lib.rs)                          │
│  Core Ops: apply_overlay, remove_overlay, show_status, restore,        │
│  update, create_overlay, switch_overlay + git exclude management       │
├──────────────┬──────────────┬──────────────┬───────────────────────────┤
│  Source       │  State       │  GitHub      │  Support Modules          │
│  Resolution   │  Management  │  Integration │                           │
│              │              │              │                           │
│ reference.rs │ state.rs     │ github.rs    │ detection.rs              │
│ sources.rs   │ (persistence,│ cache.rs     │ selection.rs              │
│ overlay_     │  OverlayState│              │ json_merge.rs             │
│  repo.rs     │  FileEntry,  │              │ fuzzy.rs                  │
│              │  SourceRe-   │              │ overlay_name.rs           │
│              │  solver)     │              │ upstream.rs               │
├──────────────┴──────────────┴──────────────┴───────────────────────────┤
│                     Configuration (config.rs)                           │
│  Global + per-repo config, CCL format, source URL validation           │
└─────────────────────────────────────────────────────────────────────────┘
```

### Component Responsibilities

| Component | Responsibility | Est. Lines (code + tests) | Risk Level |
|-----------|----------------|---------------------------|------------|
| **lib.rs** | Core application logic: all overlay operations, git exclude mgmt, conflict resolution | ~7200 | HIGH - largest file, most logic, many inline tests |
| **cli.rs** | CLI parsing (clap), command dispatch, 9 issue-specific bug-fix tests | ~8000 | HIGH - second largest, complex dispatch logic |
| **state.rs** | State models, SourceResolver trait, CCL persistence, external backup | ~1700 | MEDIUM - central data types, serialization |
| **selection.rs** | Interactive TUI for file selection, flat + categorized modes | ~3000 | MEDIUM - complex terminal handling, untestable paths |
| **sources.rs** | Multi-source priority resolution, upstream fallback | ~600 | MEDIUM - critical resolution logic |
| **config.rs** | Global/repo config, CCL parsing, source URL validation | ~900 | LOW-MEDIUM - well-tested parsing |
| **overlay_repo.rs** | Shared overlay repo management, recursive copy, staging | ~800 | MEDIUM - filesystem operations, security validation |
| **github.rs** | GitHub URL parsing, git ref handling, remote URL parsing | ~700 | LOW - well-tested pure parsing |
| **cache.rs** | GitHub cache management, git command execution, metadata | ~700 | MEDIUM - external process interaction |
| **reference.rs** | Input parsing into SourceReference enum | ~250 | LOW - well-tested enum parser |
| **detection.rs** | File discovery for create command, AI config patterns | ~500 | LOW - straightforward pattern matching |
| **upstream.rs** | Git fork detection, upstream remote parsing | ~250 | LOW - simple remote inspection |
| **json_merge.rs** | Deep JSON merge with type mismatch tracking | ~250 | LOW - well-tested utility |
| **fuzzy.rs** | Fuzzy overlay name matching | ~200 | LOW - thin wrapper around fuzzy-matcher |
| **overlay_name.rs** | Overlay name validation newtype | ~100 | LOW - trivial wrapper |
| **testutil.rs** | Test helpers (TestContext, fixture factories) | ~150 | LOW - test infrastructure |

## Recommended Review Order

The review order is driven by three principles:

1. **Dependency direction** -- review leaf modules before modules that depend on them
2. **Risk and complexity** -- spend time proportional to the module's bug surface area
3. **State correctness first** -- data models and persistence must be correct before reviewing operations that use them

### Phase 1: Foundation Modules (Leaf Dependencies)

Review these first because every other module depends on their correctness. They have no `use crate::` imports (or minimal ones to each other).

```
overlay_name.rs  (no crate deps)
    |
fuzzy.rs         (no crate deps)
    |
json_merge.rs    (no crate deps)
    |
github.rs        (no crate deps)  <- reference.rs depends on this
    |
upstream.rs      (depends on: github::parse_remote_url)
    |
reference.rs     (depends on: github::GitHubSource)
```

**Review focus:** Correctness of parsing, edge cases, security validation (flag injection in github.rs).

**Estimated effort:** Light -- these modules are well-tested with focused unit tests.

### Phase 2: Infrastructure Modules (State + Config + Cache)

These provide the data backbone. State correctness is critical -- bugs here corrupt user data.

```
state.rs         (depends on: overlay_name)
    |
config.rs        (depends on: nothing internal)
    |
cache.rs         (depends on: github)
```

**Review focus:**
- `state.rs`: SourceResolver trait correctness, serialization roundtrips, external state backup/restore, the new SourceResolver abstraction (PR #150) for exhaustive source-type dispatch
- `config.rs`: CCL parsing robustness, source URL validation edge cases, env var handling
- `cache.rs`: Git command error handling, metadata consistency under partial failures, cache cleanup logic

**Estimated effort:** Medium -- state.rs is critical, cache.rs has external process interaction.

### Phase 3: Resolution Layer (Sources + Overlay Repo)

These connect user input to actual overlay files. Resolution bugs produce the wrong overlay silently.

```
sources.rs       (depends on: config, overlay_repo, state, upstream)
    |
overlay_repo.rs  (depends on: config, state, upstream)
```

**Review focus:**
- `sources.rs`: Priority ordering correctness, first-match-wins semantics, upstream fallback logic, edge cases with missing/unreachable sources
- `overlay_repo.rs`: Path traversal validation, copy_dir_recursive symlink handling, MAX_COPY_DEPTH enforcement, git operations (stage/commit/push)

**Estimated effort:** Medium -- subtle resolution priority bugs, security-sensitive path validation.

### Phase 4: Support Modules (Detection + Selection)

These are input-gathering modules used by the `create` and `edit` commands.

```
detection.rs     (no crate deps)
    |
selection.rs     (depends on: detection)
```

**Review focus:**
- `detection.rs`: AI config pattern completeness, deduplication logic, handling of non-git directories
- `selection.rs`: State machine correctness (mode transitions, scroll clamping, directory expansion), terminal state recovery, untestable paths (actual crossterm rendering skipped in unit tests)

**Estimated effort:** Medium for selection.rs (complex state machine, ~3000 lines), light for detection.rs.

### Phase 5: Core Operations (lib.rs)

Review last because it orchestrates all the modules above. Bugs here are most likely to be logic errors in how modules are composed.

```
lib.rs           (depends on: ALL other modules)
```

**Review focus:**
- `apply_overlay` / `apply_resolved_overlay`: Conflict detection correctness, symlink vs copy logic, JSON merge integration, git exclude management
- `remove_overlay` / `remove_single_overlay`: Complete cleanup (files, state, git exclude, external state), empty directory cleanup
- `show_status` / `show_status_json`: Accurate state reporting, handling of stale/missing state
- `restore_overlays`: Recovery logic, handling of moved/deleted sources
- `update_overlays`: Update detection, remove-then-reapply atomicity
- `create_overlay`: File copying, config generation, output directory handling
- `switch_overlay`: Atomicity of remove-all-then-apply
- Git exclude management: Section markers, multi-overlay isolation, line ending handling

**Estimated effort:** Heavy -- ~7200 lines, most complex logic, highest bug density.

### Phase 6: CLI Layer (cli.rs)

Review after lib.rs since cli.rs is primarily dispatch logic, but it also contains significant command-specific logic (edit, sync, browse, source management).

```
cli.rs           (depends on: lib.rs, all other modules)
```

**Review focus:**
- Command dispatch correctness (all subcommands route correctly)
- Argument validation and error messages
- The 9 issue-specific bug tests (#142-#148) -- verify they pass and the fixes are complete
- `edit` command logic: add/remove files, interactive selection, dry-run handling
- `sync` command: source type checking, overlay repo push logic
- `browse` command: ephemeral source handling, auto-filtering
- `source` command: add/remove/list workflow, config persistence

**Estimated effort:** Heavy -- ~8000 lines, mix of dispatch + complex command logic.

### Phase 7: Integration Tests (tests/cli.rs + tests/common/)

Final review to ensure test coverage matches the reviewed code paths.

```
tests/cli.rs     (integration tests)
tests/common/    (test infrastructure)
```

**Review focus:** Identify gaps between what was reviewed in phases 1-6 and what's tested. Cross-reference with known test coverage gaps from CONCERNS.md.

**Estimated effort:** Medium -- ~1850 lines of integration tests to audit.

## Dependency Graph

```
                          +-----------+
                          |  main.rs  |
                          +-----+-----+
                                |
                          +-----v-----+
                     +----+  cli.rs   +----------------------------+
                     |    +-----+-----+                            |
                     |          |                                  |
                     |    +-----v-----+                            |
                     |    |  lib.rs   | <-- Most deps flow here    |
                     |    +--+--+--+--+                            |
                     |       |  |  |                               |
          +----------+-------+  |  +-------+                       |
          |          |          |           |                       |
    +-----v----+ +---v----+ +--v---+ +----v-----+ +------------+  |
    |sources.rs| |state.rs| |cache | |selection | |detection.rs|  |
    +--+---+---+ +---+----+ |.rs   | +----+-----+ +------------+  |
       |   |         |      +--+---+      |                       |
       |   |         |         |          |                       |
    +--v---v---+     |      +--v------+   |                       |
    |overlay_  |     |      |github   |   |                       |
    |repo.rs   |     |      |.rs      |   |                       |
    +--+-------+     |      +---------+   |                       |
       |             |                    |                       |
    +--v-------+  +--v----------+         |                       |
    |upstream  |  |overlay_name |         |                       |
    |.rs       |  |.rs          |         |                       |
    +----------+  +-------------+         |                       |
                                          |                       |
    +----------+  +---------+  +----------v+  +----------+        |
    |json_     |  |fuzzy.rs |  |config.rs   |  |reference |       |
    |merge.rs  |  |         |  |            |  |.rs       |<------+
    +----------+  +---------+  +------------+  +----------+
```

## Manual Test Suite Organization

### Structure Recommendation: Feature-Workflow Hybrid

Organize manual tests by **user workflow** (primary axis) with **risk annotations** (secondary). This mirrors how users interact with the tool and ensures the highest-risk paths get the most scrutiny.

### Test Categories

#### Category 1: Core Workflows (Must-Pass for 1.0)

These are the essential user journeys. Every 1.0 release candidate must pass all of these manually.

| ID | Workflow | Steps | Risk Areas |
|----|----------|-------|------------|
| CW-01 | **Local overlay lifecycle** | apply local dir -> status -> remove | Symlink creation, state management, git exclude |
| CW-02 | **GitHub overlay lifecycle** | apply GitHub URL -> status -> update -> remove | Cache management, network handling, commit tracking |
| CW-03 | **Overlay repo lifecycle** | configure source -> apply org/repo/overlay -> sync -> update -> remove | Multi-source resolution, upstream fallback |
| CW-04 | **Multi-overlay stacking** | apply A -> apply B -> status -> remove B -> remove A | Cross-overlay conflict detection, per-overlay git exclude |
| CW-05 | **Switch overlay** | apply A -> switch B -> verify A removed + B applied | Atomicity of remove-all-then-apply |
| CW-06 | **Restore workflow** | apply -> delete .repoverlay -> restore from external state | External state backup correctness |
| CW-07 | **Create overlay** | create from existing repo files -> verify output | Detection patterns, file selection, config generation |
| CW-08 | **Edit overlay** | apply -> edit add/remove files -> verify state updated | Source mutability checks, state file updates |

#### Category 2: Cross-Platform Verification

| ID | Platform | Key Verification |
|----|----------|-----------------|
| CP-01 | Linux | Symlinks, permissions, XDG directories |
| CP-02 | macOS | Symlinks, case-insensitive filesystem, HFS+ |
| CP-03 | Windows | Copy fallback (or symlink with admin), AppData paths |
| CP-04 | WSL | Symlinks across mount boundaries |

#### Category 3: Error Handling and Edge Cases

| ID | Scenario | Expected Behavior | Risk |
|----|----------|-------------------|------|
| EH-01 | Apply to non-git directory | Clear error message | LOW |
| EH-02 | Apply with conflicting file (default strategy) | Fail with conflict details | MEDIUM |
| EH-03 | Apply with --force over existing file | Overwrite, preserve state | MEDIUM |
| EH-04 | Apply with --skip-conflicts | Skip conflicts, apply rest | MEDIUM |
| EH-05 | Apply path traversal attempt (../../../etc) | Reject with security error | HIGH |
| EH-06 | Cache corruption recovery | Graceful degradation, suggest cache clear | MEDIUM |
| EH-07 | Network failure during GitHub clone | Clear error, no partial state | MEDIUM |
| EH-08 | Ctrl+C during interactive selection | Terminal restored, no partial state | HIGH |
| EH-09 | Invalid CCL config file | Parse error with file path and line | MEDIUM |
| EH-10 | Stale state (files removed outside repoverlay) | Status shows warnings | MEDIUM |

#### Category 4: JSON Merge Verification

| ID | Scenario | Expected Behavior |
|----|----------|-------------------|
| JM-01 | Merge disjoint JSON objects | Both keys present |
| JM-02 | Merge overlapping objects (nested) | Deep recursive merge |
| JM-03 | Merge with type mismatch | Overlay wins, warning shown |
| JM-04 | Cross-overlay JSON auto-merge | Automatic merge without --merge flag |
| JM-05 | Non-JSON conflict with --merge flag | Merge for JSON, conflict for others |

#### Category 5: Source Management

| ID | Scenario | Expected Behavior |
|----|----------|-------------------|
| SM-01 | Add source by full URL | Source saved to config |
| SM-02 | Add source by GitHub shorthand | Expanded and saved |
| SM-03 | Add duplicate source name | Clear error |
| SM-04 | Remove source | Config updated, no orphaned state |
| SM-05 | List sources with overlays | Shows overlay counts per source |
| SM-06 | Multi-source priority resolution | First-match-wins ordering |
| SM-07 | Upstream fallback resolution | Falls back to parent repo overlays |

### Test Organization on Disk

```
docs/manual-tests/
├── README.md                    # How to run manual tests
├── core-workflows/
│   ├── CW-01-local-lifecycle.md
│   ├── CW-02-github-lifecycle.md
│   ├── CW-03-overlay-repo-lifecycle.md
│   ├── CW-04-multi-overlay-stacking.md
│   ├── CW-05-switch-overlay.md
│   ├── CW-06-restore-workflow.md
│   ├── CW-07-create-overlay.md
│   └── CW-08-edit-overlay.md
├── cross-platform/
│   ├── CP-01-linux.md
│   ├── CP-02-macos.md
│   ├── CP-03-windows.md
│   └── CP-04-wsl.md
├── error-handling/
│   ├── EH-01-non-git-directory.md
│   ├── ...
│   └── EH-10-stale-state.md
├── json-merge/
│   └── JM-01-through-05.md
└── source-management/
    └── SM-01-through-07.md
```

Each test file follows a consistent format:

```markdown
# CW-01: Local Overlay Lifecycle

**Priority:** P0 (must-pass for 1.0)
**Platforms:** All
**Time estimate:** 5 minutes

## Prerequisites
- Git repository initialized
- Local overlay directory with known files

## Steps
1. `repoverlay apply ./my-overlay --target ./repo`
   - Expected: "Applying overlay..." message, files appear as symlinks
2. `repoverlay status --target ./repo`
   - Expected: Shows overlay name, source, file count
3. Verify `.git/info/exclude` contains overlay section markers
4. `repoverlay remove my-overlay --target ./repo`
   - Expected: Files removed, state cleaned, exclude updated

## Pass Criteria
- [ ] Files created as symlinks (Linux/macOS) or copies (Windows)
- [ ] State directory `.repoverlay/` created and removed correctly
- [ ] Git exclude markers added and removed cleanly
- [ ] External state backup created and removed

## Known Issues
- None
```

## How to Structure Test Additions Without Disrupting Existing Tests

### Principle 1: Additive Only

All new tests are additions -- never modify existing tests unless they contain actual bugs. Existing tests that pass represent verified behavior. Changing them risks breaking that verification.

### Principle 2: Module-Local Unit Tests

Add new unit tests inside the existing `#[cfg(test)] mod tests { }` blocks in each source file. Group related tests using inner modules:

```rust
// In src/state.rs, inside the existing #[cfg(test)] mod tests { }
mod source_resolver_edge_cases {
    use super::*;

    #[test]
    fn resolve_local_path_with_missing_directory() { /* ... */ }

    #[test]
    fn resolve_local_path_with_symlink_chain() { /* ... */ }
}
```

### Principle 3: Integration Test Expansion

Add new integration tests at the end of `tests/cli.rs`. Group by feature area with clear comment separators:

```rust
// ==================== 1.0 Stabilization: Edge Case Tests ====================

#[test]
fn apply_with_unicode_filename() { /* ... */ }

#[test]
fn remove_with_stale_state_file() { /* ... */ }
```

### Principle 4: Shared Test Infrastructure

If new tests need shared fixtures, add to the existing helpers:
- **New CLI test fixtures** -> `tests/common/mod.rs` (add new functions like `mapped_overlay()`, `json_overlay()`)
- **New library test fixtures** -> `src/testutil.rs` (add new `TestContext` methods or factory functions)

Never duplicate test infrastructure -- extend the existing `TestContext` types.

### Principle 5: Test Isolation

Every test must:
- Create its own `TempDir` (via `TestContext::new()` or directly)
- Not depend on environment variables (or isolate them via `SourceTestContext`)
- Not depend on network access (unit tests) -- integration tests that need GitHub access should be behind `#[cfg(feature = "network-tests")]` or `#[ignore]`
- Not depend on ordering with other tests

### Principle 6: Coverage-Driven Additions

Use `just test-coverage` + `just coverage-html` to identify untested lines. Prioritize:

1. **Error paths** in lib.rs (bail! and error returns that are never triggered in tests)
2. **Branch conditions** in apply_overlay (conflict strategies, JSON merge paths)
3. **State transitions** in selection.rs (mode switches, directory expansion edge cases)
4. **Serialization roundtrips** in state.rs (every OverlaySource variant)

## Architectural Patterns

### Pattern 1: Module-Per-Responsibility

**What:** Each `.rs` file owns one cohesive responsibility. The codebase uses flat module organization (all modules at `src/` level) rather than nested module directories.
**When to use:** Repoverlay's current structure works well for ~15 modules. If module count grows past 20, consider grouping into subdirectories (e.g., `src/resolution/` for reference.rs, sources.rs, overlay_repo.rs).
**Trade-offs:** Simple to navigate; potential for large files (lib.rs at ~7200 lines is pushing the limit).

### Pattern 2: Enum-Based Dispatch

**What:** Core types use enums with pattern matching for variant-specific behavior: `SourceReference`, `OverlaySource`, `ConflictStrategy`, `GitRef`.
**When to use:** When a type has a fixed set of variants that each require different handling.
**Trade-offs:** Compiler-enforced exhaustiveness (adding a variant forces updating all match arms); pattern matching can become deeply nested for complex variants.

### Pattern 3: SourceResolver Trait

**What:** The recently introduced `SourceResolver` trait (PR #150) centralizes source-type dispatch so each command doesn't independently match on `OverlaySource` variants.
**When to use:** When multiple call sites need to branch on the same enum. Extract the branching into a trait.
**Trade-offs:** Single point of update for new source types; adds indirection over direct pattern matching.

### Pattern 4: Dual-Location State Persistence

**What:** Overlay state is stored both in-repo (`.repoverlay/`) and externally (`~/.local/share/repoverlay/`). The external copy enables recovery after `git clean -fdx`.
**When to use:** When user data must survive destructive operations on the repository.
**Trade-offs:** Must keep both locations in sync; introduces consistency concerns if one write fails.

## Anti-Patterns to Avoid

### Anti-Pattern 1: Modifying Existing Test Assertions

**What people do:** Change an existing test's expected output to make it pass with new code.
**Why it's wrong:** The test was validating correct behavior. Changing the assertion means the previous correct behavior is no longer verified.
**Do this instead:** If behavior intentionally changed, add a NEW test for the new behavior and explicitly delete the old test with a comment explaining why.

### Anti-Pattern 2: Adding Tests That Depend on Test Execution Order

**What people do:** Write a test that assumes state left by a previous test (e.g., a file in a shared temp directory).
**Why it's wrong:** `cargo test` runs tests in parallel by default. Order-dependent tests fail intermittently.
**Do this instead:** Each test creates its own isolated `TempDir` via `TestContext::new()`.

### Anti-Pattern 3: Testing Implementation Details

**What people do:** Assert on internal state structure (e.g., checking raw CCL file contents character by character).
**Why it's wrong:** Coupling tests to serialization format means any format change breaks tests, even if behavior is correct.
**Do this instead:** Test through the public interface. Apply an overlay, then use `status` to verify state, not by reading CCL files directly.

### Anti-Pattern 4: Ignoring Failing Tests

**What people do:** Add `#[ignore]` to tests that fail during review rather than fixing the underlying bug.
**Why it's wrong:** For a 1.0 stabilization, every discovered bug must be tracked and resolved. Ignored tests are invisible bugs.
**Do this instead:** If a test reveals a bug that cannot be fixed immediately, create a GitHub issue and add a comment referencing it: `// TODO(#NNN): Fix after merge cleanup`.

## Integration Points

### Internal Boundaries

| Boundary | Communication | Review Concern |
|----------|---------------|----------------|
| lib.rs <-> state.rs | Direct function calls, shared types | Serialization correctness, state consistency |
| lib.rs <-> cli.rs | Function calls from dispatch | Argument passing correctness, error propagation |
| lib.rs <-> sources.rs | Resolution calls during apply | Priority ordering, fallback behavior |
| cli.rs <-> selection.rs | Interactive UI invocation | Terminal state management, Ctrl+C handling |
| state.rs <-> config.rs | CCL format shared (sickle crate) | Format compatibility, parsing edge cases |
| cache.rs <-> github.rs | URL parsing for cache keys | Cache key uniqueness, path sanitization |

### External Dependencies

| Dependency | Integration Pattern | Review Concern |
|------------|---------------------|----------------|
| git CLI | `Command::new("git")` process execution | Output parsing fragility, flag injection |
| sickle (CCL) | `serde` derive for serialization | Format stability, error messages on parse failure |
| crossterm | Raw terminal mode for selection UI | Terminal state recovery on panic/interrupt |
| clap | Derive macros for CLI parsing | Argument validation, help text accuracy |
| fuzzy-matcher | Score-based string matching | Score threshold sensitivity |

## Scaling Considerations

| Scale | Architecture Adjustments |
|-------|--------------------------|
| Current (15 modules, ~25K lines) | Flat module structure works fine. lib.rs is the largest concern at ~7200 lines |
| +5 commands | Consider splitting cli.rs command logic into `src/commands/` directory with one file per command |
| +5 source types | SourceResolver trait handles this cleanly -- just add new match arms |
| Large overlays (>1000 files) | apply_overlay needs streaming/progress; selection.rs needs virtual scrolling |

## Sources

- Direct codebase analysis of all 16 source files
- `.planning/codebase/ARCHITECTURE.md` (current architecture documentation)
- `.planning/codebase/CONCERNS.md` (known tech debt and coverage gaps)
- `.planning/codebase/TESTING.md` (current test patterns)
- `docs/adr/0001-git-cli-over-git-library.md` (git integration decision)
- `docs/adr/0002-keep-apply-and-switch-commands.md` (CLI design decision)
- `Cargo.toml` (dependency inventory)
- Commit history: PR #150 (SourceResolver trait introduction)

---
*Architecture research for: repoverlay 1.0 stabilization*
*Researched: 2026-02-27*
