---
phase: 01-code-review-and-bug-fixes
plan: 02
subsystem: core
tags: [code-review, bug-fix, rust, sigpipe, error-handling, source-resolver, libc]

# Dependency graph
requires:
  - phase: 01-01
    provides: "Verified correctness of all 13 non-orchestration source modules"
provides:
  - "Complete code review of all 15 source modules (lib.rs and cli.rs reviewed)"
  - "FIX-02: Human-readable error output with Display format ({e:#})"
  - "FIX-03: Clean pipe handling via SIGPIPE reset with libc"
  - "FIX-04: All 7 source_resolver_bugs regression tests verified passing"
  - "Full Phase 1 correctness baseline established"
affects: [02-documentation-and-api-review, 03-testing-and-verification]

# Tech tracking
tech-stack:
  added: [libc]
  patterns: ["SIGPIPE reset at entry point for CLI tools", "anyhow Display format for user-facing errors"]

key-files:
  created: []
  modified:
    - src/main.rs
    - Cargo.toml
    - Cargo.lock

key-decisions:
  - "Used #[allow(unsafe_code)] for SIGPIPE handling since it is the standard CLI pattern and the only unsafe code in the binary"
  - "No correctness bugs found in lib.rs or cli.rs orchestration modules -- codebase is well-implemented"
  - "All SourceResolver usage in cli.rs verified correct -- direct OverlaySource matching is justified for data extraction and display"
  - "Plan referenced 9 regression tests but actual count is 7 -- all 7 cover issues #142-#148 correctly"

patterns-established:
  - "SIGPIPE reset at main() entry before any I/O for clean pipe behavior"
  - "anyhow error chain display with {e:#} format for user-facing messages"

requirements-completed: [REVIEW-05, REVIEW-06, FIX-01, FIX-02, FIX-03, FIX-04]

# Metrics
duration: 4min
completed: 2026-02-28
---

# Phase 1 Plan 02: Orchestration Module Review and Bug Fixes Summary

**Reviewed lib.rs and cli.rs orchestration modules, fixed error display format and SIGPIPE handling, verified all issue regression tests passing**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-28T06:25:50Z
- **Completed:** 2026-02-28T06:29:52Z
- **Tasks:** 2
- **Files modified:** 3 (src/main.rs, Cargo.toml, Cargo.lock)

## Accomplishments

- Reviewed lib.rs (~3200 lines) and cli.rs (~7965 lines) orchestration modules for correctness
- FIX-02: Changed error format from Debug ({e:?}) to Display ({e:#}) for human-readable error output
- FIX-03: Added SIGPIPE reset at start of main() with libc dependency for clean pipe behavior
- FIX-04: Verified all 7 source_resolver_bugs regression tests pass (issues #142-#148)
- FIX-01: Consolidated all review findings -- no additional bugs discovered across either plan
- Complete Phase 1 code review of all 15 source modules finished with correctness baseline established

## Review Findings

### lib.rs Review

| Area | Lines | Status | Notes |
|------|-------|--------|-------|
| Path traversal validation | ~1280-1318 | Correct | Normalizes components, checks `starts_with(target)` after each step and at end |
| Symlink creation | ~1231, ~1463 | Correct | Uses absolute source paths; `#[cfg(unix/windows)]` platform gating correct |
| ConflictStrategy dispatch | ~1066-1444 | Correct | All 4 variants handled in both directory and file conflict paths |
| `apply_overlay_internal` | ~966-1534 | Correct | Error handling uses `?` and `.with_context()` consistently; state saved atomically after all files |
| JSON merge path | ~1322-1396 | Correct | Falls through to conflict handling on merge failure |
| `update_overlays` | ~2457-2613 | Correct | Uses SourceResolver for non-GitHub sources (is_updatable, source_type_label) |
| `restore_overlays` | ~2340-2444 | Correct | Handles all 3 source types for re-apply |
| Git exclude management | ~3014-3124 | Correct | Section markers written/removed correctly; handles empty exclude, managed section cleanup |

**Path traversal validation detail:** The code at line 1282-1317 iterates through each component of the target relative path. For `ParentDir` (`..`), it checks that the current normalized path hasn't reached or escaped the target root before popping. After all components, it re-checks `starts_with(target)`. This prevents symlink-chain bypass because the path is validated at the logical level, not following symlinks. The earlier `canonicalize_path` call at line 1020 resolves the target itself, so the target anchor is a real path.

**Symlink creation note:** Symlinks are created using absolute source paths (the `source` variable is already canonicalized by `resolve_source`). This means symlinks break if the overlay source moves. This is known tech debt documented in CONCERNS.md but is not a bug -- it's the expected behavior for local overlays.

### cli.rs Review

| Area | Status | Notes |
|------|--------|-------|
| SourceResolver at all call sites | Correct | `is_syncable()`, `is_mutable()`, `is_updatable()`, `source_type_label()` used for all behavioral dispatch |
| Issue #142-#148 regression tests | All 7 pass | Tests correctly verify SourceResolver usage at all dispatch points |
| Deprecated commands (`create-local`, `list`) | Correct | Deprecation warnings printed; dispatch to current implementations |
| Command routing | Correct | All Commands variants map to correct handler functions; no dead code |

**SourceResolver usage detail:** Every behavioral decision in cli.rs (can sync? can edit? can update? can add files?) uses SourceResolver trait methods. Direct `OverlaySource` pattern matching occurs only for:
1. Data extraction (getting org/repo/name fields from OverlayRepo variant)
2. Display formatting (showing source-specific status information)
3. Auto-commit logic (OverlayRepo-only feature that requires repo path)

These are legitimate uses, not dispatch bypasses. Adding a new OverlaySource variant would still be caught at compile time by the SourceResolver trait's exhaustive match.

## Bug Fixes Applied

### FIX-02: Error Display Format (main.rs)

**Before:** `eprintln!("Error: {e:?}");` -- Debug format shows internal representation
**After:** `eprintln!("Error: {e:#}");` -- Display alternate format shows human-readable error chain

The `#` flag on `anyhow::Error` prints the full error chain with `: ` separators. Example:
- Debug: `Error: Failed to apply overlay\n\nCaused by:\n    0: Failed to create symlink`
- Display (#): `Error: Failed to apply overlay: Failed to create symlink`

### FIX-03: SIGPIPE Handling (main.rs + Cargo.toml)

Added at start of main(), before any I/O:
```rust
#[cfg(unix)]
#[allow(unsafe_code)]
unsafe {
    libc::signal(libc::SIGPIPE, libc::SIG_DFL);
}
```

- Rust's runtime masks SIGPIPE, causing "Broken pipe" errors when piping to `head`, `less`, etc.
- Resetting to SIG_DFL (default: terminate) is the standard fix for CLI tools
- `#[cfg(unix)]` ensures this only compiles on Unix platforms
- `libc = "0.2"` added to Cargo.toml (already a transitive dependency via crossterm/nix, no new download)
- `#[allow(unsafe_code)]` permits the unsafe block locally despite the project's `unsafe_code = "warn"` lint

### FIX-04: Issue Regression Test Verification

All 7 source_resolver_bugs tests pass:

| Test | Issue | Status |
|------|-------|--------|
| `issue_142_resolve_source_path_github_should_not_bail` | #142 | PASS |
| `issue_143_add_files_should_check_source_type_for_local` | #143 | PASS |
| `issue_143_add_files_should_reject_github_clearly` | #143 | PASS |
| `issue_145_update_code_should_handle_overlay_repo_separately` | #145 | PASS |
| `issue_146_sync_single_name_should_check_source_type` | #146 | PASS |
| `issue_147_resolve_should_use_source_name` | #147 | PASS |
| `issue_148_add_should_check_mutability_before_filesystem_changes` | #148 | PASS |

Note: The plan referenced "9 expected tests" but the actual module contains 7 tests. The plan listed 7 unique test names, so the count discrepancy is in the plan description only.

### FIX-01: Bug Consolidation

No additional bugs were discovered during the review of lib.rs or cli.rs. Combined with Plan 01's finding of zero bugs across 13 non-orchestration modules, the complete Phase 1 review found:
- **0 correctness bugs** across all 15 source modules
- **2 user-facing quality issues** fixed (FIX-02 error format, FIX-03 SIGPIPE)
- **7 regression tests** verified passing (FIX-04)

## Task Commits

1. **Task 1: Review lib.rs and cli.rs for correctness** - No code changes (review-only, all findings documented above)
2. **Task 2: Apply bug fixes (FIX-02, FIX-03) and verify issues (FIX-04)** - `f53a918` (fix)

## Files Created/Modified

- `src/main.rs` - Added SIGPIPE handling at main() entry; changed error format from Debug to Display
- `Cargo.toml` - Added `libc = "0.2"` dependency
- `Cargo.lock` - Updated lockfile with libc dependency

## Decisions Made

- Used `#[allow(unsafe_code)]` attribute on the SIGPIPE reset block rather than changing the project-wide lint. The unsafe block is the standard pattern for Rust CLI tools and is the only unsafe code in the binary.
- No correctness bugs found in either lib.rs or cli.rs, confirming the codebase is well-implemented. All direct OverlaySource matching in these modules is justified for data extraction and display purposes.
- The plan's reference to "9 tests" was noted as a minor inaccuracy (actual: 7 tests). All 7 tests are correct and pass.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added #[allow(unsafe_code)] for SIGPIPE handling**
- **Found during:** Task 2 (SIGPIPE implementation)
- **Issue:** Project-wide `unsafe_code = "warn"` lint (promoted to error by `-D warnings` in clippy) blocked the planned unsafe SIGPIPE reset block
- **Fix:** Added `#[allow(unsafe_code)]` attribute on the unsafe block with a SAFETY comment explaining why this is correct
- **Files modified:** src/main.rs
- **Verification:** `just check` passes clean (clippy, format, all tests)
- **Committed in:** f53a918

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Necessary to satisfy both the SIGPIPE requirement and the project's lint configuration. No scope creep.

## Tech Debt Documented

Items carried forward from Plan 01 (not fixed per plan constraints):

1. **`overlay_name.rs`**: `debug_assert!` only fires in debug builds for path separator validation
2. **`selection.rs`**: Terminal raw mode not RAII-based; panic could leave terminal in raw mode
3. **`cache.rs`**: Non-atomic `fs::write()` for cache metadata
4. **`github.rs`**: Branch names with slashes split by URL parser
5. **`sickle` crate (v0.1.2)**: Early version first-party dependency
6. **`cache.rs`**: Interrupted clone leaves partial state
7. **Symlink paths**: Absolute symlinks break if overlay source moves (documented in CONCERNS.md)

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- All 15 source modules reviewed and verified correct, establishing the Phase 1 correctness baseline
- Error output now uses human-readable Display format (important for Phase 2 documentation)
- SIGPIPE handling enables clean piping (important for scripting/CI usage patterns)
- All 915 library tests + 93 integration tests pass clean
- `just check` passes (format + lint + all tests) with zero warnings
- Tech debt inventory provides clear targets for Phase 3 (polish) or future maintenance

## Self-Check: PASSED

- 01-02-SUMMARY.md exists at expected path
- src/main.rs exists with SIGPIPE handling and Display error format
- Cargo.toml exists with libc dependency
- Commit f53a918 exists in git log
- SIGPIPE handling verified: `libc::signal(libc::SIGPIPE, libc::SIG_DFL)` present
- Display format verified: `{e:#}` present (not `{e:?}`)
- libc dependency verified: `libc = "0.2"` in Cargo.toml

---
*Phase: 01-code-review-and-bug-fixes*
*Completed: 2026-02-28*
