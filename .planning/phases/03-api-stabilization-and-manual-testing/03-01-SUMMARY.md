---
phase: 03-api-stabilization-and-manual-testing
plan: 01
subsystem: api
tags: [rust, visibility, pub-crate, api-surface, clippy, module-docs]

requires:
  - phase: 02-test-coverage
    provides: verified test suite covering all modules
provides:
  - All pub items locked to pub(crate) except lib::run()
  - Module-level doc comments reviewed and enhanced
  - redundant_pub_crate clippy lint allowed for explicit visibility
affects: [03-02, release]

tech-stack:
  added: []
  patterns: [pub(crate) for explicit API surface control in private modules]

key-files:
  created: []
  modified:
    - src/state.rs
    - src/config.rs
    - src/detection.rs
    - src/selection.rs
    - src/overlay_repo.rs
    - src/json_merge.rs
    - src/cache.rs
    - src/upstream.rs
    - src/github.rs
    - src/sources.rs
    - src/fuzzy.rs
    - src/reference.rs
    - src/overlay_name.rs
    - src/cli.rs
    - Cargo.toml

key-decisions:
  - "Allowed clippy::redundant_pub_crate lint to enable explicit pub(crate) in private modules"
  - "No #[non_exhaustive] added per user decision (binary-only, no external consumers)"
  - "Enhanced doc comments for cli, sources, json_merge, overlay_name modules only"

patterns-established:
  - "pub(crate) visibility: all items except lib::run() use pub(crate) for explicit API surface"

requirements-completed: [API-01, API-02, API-03]

duration: 6min
completed: 2026-03-04
---

# Phase 3 Plan 1: API Surface Lockdown Summary

**All pub items converted to pub(crate) across 14 modules with redundant_pub_crate lint allowed, module docs enhanced for 4 complex modules**

## Performance

- **Duration:** 6 min
- **Started:** 2026-03-04T02:36:57Z
- **Completed:** 2026-03-04T02:43:32Z
- **Tasks:** 2
- **Files modified:** 15

## Accomplishments
- Converted all pub items (fn, struct, enum, const, trait, type, struct fields) to pub(crate) across 14 source modules
- Only lib::run() remains pub as the single external entry point
- testutil.rs pub items left unchanged (already #[cfg(test)] gated)
- Enhanced module-level doc comments for cli.rs, sources.rs, json_merge.rs, overlay_name.rs
- All 17 modules verified to have //! doc comments
- No #[non_exhaustive] attributes added (API-02 skipped per user decision)

## Task Commits

Each task was committed atomically:

1. **Task 1: Convert all pub items to pub(crate) across 14 source modules** - `3d4483a` (refactor)
2. **Task 2: Review and enhance module-level doc comments** - `170ce64` (docs)

## Pub Items Converted Per Module

| Module | Items Converted |
|--------|----------------|
| state.rs | 27 items (5 consts, 3 enums, 5 structs, 1 trait, 13 fns + struct fields) |
| config.rs | 16 items (3 structs, 1 enum, 1 const, 11 fns + struct fields) |
| detection.rs | 12 items (1 enum, 1 struct, 2 consts, 8 fns + struct fields) |
| selection.rs | 9 items (4 structs, 1 fn + struct fields) |
| overlay_repo.rs | 6 items (3 structs, 3 fns + struct fields + impl methods) |
| json_merge.rs | 5 items (2 structs, 3 fns + struct fields) |
| cache.rs | 5 items (4 structs, 1 fn + struct fields + impl methods) |
| upstream.rs | 4 items (2 structs, 2 fns + struct fields) |
| github.rs | 3 items (1 struct, 1 enum, 1 fn + struct fields + impl methods) |
| sources.rs | 2 items (2 structs + struct fields + impl methods) |
| fuzzy.rs | 2 items (2 structs + impl methods) |
| reference.rs | 1 item (1 enum + impl methods) |
| overlay_name.rs | 1 item (1 struct + impl methods) |
| cli.rs | 1 item (1 fn) |

## Files Created/Modified
- `Cargo.toml` - Added redundant_pub_crate = "allow" to clippy lints
- `src/state.rs` - 27 pub items converted to pub(crate)
- `src/config.rs` - 16 pub items converted to pub(crate)
- `src/detection.rs` - 12 pub items converted to pub(crate)
- `src/selection.rs` - 9 pub items converted to pub(crate)
- `src/overlay_repo.rs` - 6 pub items converted to pub(crate)
- `src/json_merge.rs` - 5 pub items converted to pub(crate), doc comment enhanced
- `src/cache.rs` - 5 pub items converted to pub(crate)
- `src/upstream.rs` - 4 pub items converted to pub(crate)
- `src/github.rs` - 3 pub items converted to pub(crate)
- `src/sources.rs` - 2 pub items converted to pub(crate), doc comment enhanced
- `src/fuzzy.rs` - 2 pub items converted to pub(crate)
- `src/reference.rs` - 1 pub item converted to pub(crate)
- `src/overlay_name.rs` - 1 pub item converted to pub(crate), doc comment enhanced
- `src/cli.rs` - 1 pub item converted to pub(crate), doc comment enhanced

## Module Docs Enhanced
- **cli.rs** - Added description of command structure and run() entry point role
- **sources.rs** - Added key type documentation (SourceManager, ResolvedOverlay)
- **json_merge.rs** - Added explanation of merge-instead-of-overwrite behavior
- **overlay_name.rs** - Added clarification of normalization purpose

## Decisions Made
- Allowed clippy::redundant_pub_crate lint because all modules are private (mod, not pub mod) making pub(crate) technically redundant but explicitly documenting API intent
- Skipped #[non_exhaustive] per user decision (binary-only 1.0, no external consumers)
- Enhanced docs only for modules where clarification adds value; left adequate existing docs unchanged

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added redundant_pub_crate clippy lint allowance**
- **Found during:** Task 1 (pub to pub(crate) conversion)
- **Issue:** Clippy pedantic + nursery lints include `redundant_pub_crate` which flags pub(crate) inside private modules as redundant
- **Fix:** Added `redundant_pub_crate = "allow"` to `[lints.clippy]` in Cargo.toml
- **Files modified:** Cargo.toml
- **Verification:** just check passes clean
- **Committed in:** 3d4483a (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Lint allowance necessary to enable explicit pub(crate) visibility. No scope creep.

## Issues Encountered
None beyond the clippy lint addressed above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- API surface locked down, ready for manual testing (03-02)
- All existing tests pass (923 unit + 97 integration)
- just check passes clean (format + lint + test)

---
*Phase: 03-api-stabilization-and-manual-testing*
*Completed: 2026-03-04*
