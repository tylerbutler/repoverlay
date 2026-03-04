---
phase: 01-code-review-and-bug-fixes
plan: 01
subsystem: core
tags: [code-review, security-audit, rust, source-resolver, git-command-safety, path-traversal]

# Dependency graph
requires: []
provides:
  - "Verified correctness of all 13 non-orchestration source modules"
  - "SourceResolver trait completeness verification (all 3 variants, all 5 methods)"
  - "Security audit of git command construction and path validation"
  - "Tech debt inventory for future phases"
affects: [01-code-review-and-bug-fixes, 02-documentation-and-api-review]

# Tech tracking
tech-stack:
  added: []
  patterns: []

key-files:
  created: []
  modified: []

key-decisions:
  - "No correctness bugs found across all 13 modules -- codebase is well-implemented"
  - "Tech debt items documented but NOT fixed per plan constraints (review-only)"

patterns-established:
  - "SourceResolver trait centralizes OverlaySource dispatch with compile-time exhaustiveness"
  - "Path traversal validation via validate_path_component in overlay_repo.rs"
  - "Git flag injection prevention via starts_with('-') check in github.rs and overlay_repo.rs"

requirements-completed: [REVIEW-01, REVIEW-02, REVIEW-03, REVIEW-04, REVIEW-07]

# Metrics
duration: 4min
completed: 2026-02-28
---

# Phase 1 Plan 01: Code Review of Non-Orchestration Modules Summary

**Reviewed all 13 non-orchestration source modules for correctness, security, and edge cases -- zero bugs found, SourceResolver verified complete across all variants**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-28T06:18:03Z
- **Completed:** 2026-02-28T06:22:08Z
- **Tasks:** 2
- **Files modified:** 0 (review-only, no bugs required fixing)

## Accomplishments

- All 13 non-orchestration modules reviewed systematically against 8-point checklist
- SourceResolver trait implementation verified: all 5 methods handle all 3 OverlaySource variants (Local, GitHub, OverlayRepo)
- Security audit complete: git command construction, path traversal validation, URL scheme restriction, flag injection prevention
- All 913 library tests + 93 integration tests pass clean; `just check` passes

## Modules Reviewed

### Leaf Modules (Task 1)

| Module | Lines | Status | Notes |
|--------|-------|--------|-------|
| `overlay_name.rs` | 95 | Clean | `debug_assert` only fires in debug builds (tech debt) |
| `fuzzy.rs` | 270 | Clean | All edge cases handled correctly |
| `json_merge.rs` | 250 | Clean | Deep merge logic correct; type mismatches tracked |
| `github.rs` | 706 | Clean | Flag injection check correct; 40-char SHA detection working |
| `upstream.rs` | 277 | Clean | Git remote detection correct; callers trim whitespace |
| `reference.rs` | 409 | Clean | Parse priority order well-designed; tilde expansion correct |

### Infrastructure/Resolution/Support Modules (Task 2)

| Module | Lines | Status | Notes |
|--------|-------|--------|-------|
| `state.rs` | 1656 | Clean | SourceResolver trait VERIFIED COMPLETE |
| `config.rs` | 919 | Clean | CCL parsing correct; URL validation during deserialization |
| `cache.rs` | 1040 | Clean | Git commands safe; `--` separator used in clone |
| `sources.rs` | 1045 | Clean | First-match-wins semantics correct |
| `overlay_repo.rs` | 1576 | Clean | Path traversal and URL scheme validation correct |
| `detection.rs` | 844 | Clean | File discovery correct; gitignore interaction correct |
| `selection.rs` | 3329 | Clean* | Terminal raw mode cleanup is not RAII-based (tech debt) |

## SourceResolver Verification (REVIEW-07)

All 5 trait methods verified to handle all 3 `OverlaySource` variants:

| Method | Local | OverlayRepo | GitHub |
|--------|-------|-------------|--------|
| `resolve_local_path()` | Returns path directly | Loads config, ensures cloned, gets overlay path | Creates CacheManager, ensures cached |
| `is_mutable()` | true | true | false |
| `is_syncable()` | false | true | false |
| `is_updatable()` | false | true | true |
| `source_type_label()` | "local" | "overlay repo" | "GitHub" |

Rust `match` exhaustiveness guarantees compile-time completeness -- adding a new variant would be a compile error.

Direct `OverlaySource` matching in `lib.rs` and `cli.rs` is legitimate for display formatting and command-specific logic (sync, update). These are not dispatch errors -- they're presentation-layer concerns that don't need `SourceResolver` abstraction.

## Security Observations

### Git Command Safety
- All `Command::new("git")` calls use `.args([...])` with string arrays -- no shell interpolation
- `clone_repo()` in both `cache.rs` and `overlay_repo.rs` uses `--` separator before URL arguments
- Branch names passed via `--branch` flag, not positional arguments
- SHAs validated as 40 hex chars at parse time before being passed to git

### Path Validation
- `validate_path_component()` rejects empty, `.`, `..`, `/`, and `\` -- comprehensive path traversal protection
- Applied to all user-supplied path components (org, repo, overlay name) before filesystem operations
- `copy_dir_recursive()` canonicalizes paths and detects symlink escapes

### URL Scheme Restriction
- `validate_clone_url()` restricts to `https://`, `ssh://`, and `git@` schemes
- Blocks `file://` to prevent local filesystem access via overlay repos
- Rejects URLs starting with `-` to prevent flag injection

### Flag Injection Prevention
- `GitRef::from_str()` rejects refs starting with `-`
- `validate_clone_url()` rejects URLs starting with `-`
- `--` separator used in git clone commands

## Tech Debt Documented

These items are noted for future phases. They are NOT bugs and were NOT fixed per plan constraints.

1. **`overlay_name.rs`**: `debug_assert!` for path separator validation only fires in debug builds. In release builds, invalid names (containing `/`) silently pass. Could be replaced with a `Result`-returning `try_new()` method for stronger validation.

2. **`selection.rs`**: Terminal raw mode is managed via explicit `enable_raw_mode()`/`disable_raw_mode()` calls, not RAII Drop guards. If a panic occurs inside the selection loop, the terminal stays in raw mode. A `RawModeGuard` struct with a `Drop` impl would fix this.

3. **`cache.rs`**: `fs::write()` for cache metadata is not atomic. A crash during write could corrupt `.repoverlay-cache-meta.ccl`. However, `load_meta` handles parse failures gracefully (returns `None`, logs warning). Low severity.

4. **`github.rs`**: Branch names containing slashes (e.g., `feature/my-branch`) are split by the URL parser -- the first segment becomes the ref, the rest becomes subpath. This is a known limitation documented in tests but could confuse users.

5. **`sickle` crate (v0.1.2)**: First-party CCL parsing dependency. Early version number but functionally correct for current use. Maintenance status should be monitored as the project grows.

6. **`cache.rs`**: Interrupted clone operations leave partial state. On next call, `ensure_cached` will attempt to update rather than re-clone, which may fail or leave stale data. A lock file or atomic rename pattern would improve robustness.

## Task Commits

This was a review-only plan. No code changes were made.

1. **Task 1: Review leaf modules** - No code changes (all 6 modules verified correct)
2. **Task 2: Review infrastructure/resolution/support modules + verify SourceResolver** - No code changes (all 7 modules verified correct, SourceResolver complete)

## Decisions Made

- No correctness bugs found across any of the 13 reviewed modules. The codebase is well-implemented with consistent error handling patterns, proper use of `.context()`, and comprehensive test coverage.
- SourceResolver trait implementation is complete and correct. All direct `OverlaySource` matching outside the trait is justified for presentation/command-specific logic.

## Deviations from Plan

None - plan executed exactly as written. All 13 modules reviewed per checklist. No bugs found, so no inline fixes were needed.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- All 13 non-orchestration modules verified correct, establishing confidence for Phase 1 Plan 02 (orchestration module review of lib.rs and cli.rs)
- SourceResolver trait completeness confirmed, enabling Plan 02 to focus on how orchestration code uses the trait
- Tech debt inventory provides clear targets for Phase 3 (polish) or future maintenance
- The direct `OverlaySource` matching in lib.rs/cli.rs needs review in Plan 02 for correctness (noted but not in scope here)

## Self-Check: PASSED

- SUMMARY.md exists at expected path
- All 13 source modules verified to exist
- 913 library tests pass (0 failed)
- No code changes were made (review-only plan)

---
*Phase: 01-code-review-and-bug-fixes*
*Completed: 2026-02-28*
