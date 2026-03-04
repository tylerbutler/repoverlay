---
phase: 03-api-stabilization-and-manual-testing
verified: 2026-03-03T00:00:00Z
status: passed
score: 6/6 must-haves verified
re_verification: false
human_verification:
  - test: "Follow apply.md TC-01 through TC-07 against installed repoverlay binary"
    expected: "Symlinks created, copy mode works, --name override registered, --dry-run leaves no files, --force re-applies, --skip-conflicts skips, --merge deep-merges JSON"
    why_human: "Tests require an installed binary, live git repos, and visual confirmation of file types (symlink vs copy)"
  - test: "Follow source-management.md TC-01 through TC-04"
    expected: "Source add/list/remove subcommands operate on repoverlay config correctly"
    why_human: "Requires installed binary; source config reading is a live behavior"
---

# Phase 3: API Stabilization and Manual Testing Verification Report

**Phase Goal:** Public API is locked for semver and all CLI workflows are verified through manual test documentation
**Verified:** 2026-03-03
**Status:** PASSED
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths (Derived from ROADMAP Success Criteria)

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | Every pub item except lib::run() is pub(crate) | VERIFIED | `rg '^pub ' src/` returns only `lib.rs:pub fn run()` and testutil.rs items (testutil is #[cfg(test)] gated — excluded per plan) |
| 2 | #[non_exhaustive] skipped per explicit user decision (API-02) | VERIFIED | No `#[non_exhaustive]` in any source file; REQUIREMENTS.md marks API-02 as "Complete (skipped per user decision)"; both PLAN and SUMMARY document this decision |
| 3 | All 17 modules have //! doc comments | VERIFIED | Checked all 17 src files — every one has //! on line 1 or 2; enhanced for cli.rs, sources.rs, json_merge.rs, overlay_name.rs |
| 4 | 8 manual test documents exist covering all core CLI workflows | VERIFIED | All 8 files exist in docs/manual-tests/ with substantive content (136-279 lines each) |
| 5 | Test steps are copy-pasteable and use local overlays as primary | VERIFIED | All 8 documents use mktemp -d setup; 37 offline test cases + 4 optional GitHub test cases |
| 6 | Key API wiring: main.rs -> lib::run() -> cli::run() is intact | VERIFIED | main.rs calls repoverlay::run() at line 18; lib.rs calls cli::run() at line 26; cli.rs exports pub(crate) fn run() |

**Score:** 6/6 truths verified

### Required Artifacts

#### Plan 03-01 Artifacts (API Surface Lockdown)

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `src/state.rs` | 27 pub items converted to pub(crate) | VERIFIED | File contains pub(crate); commit 3d4483a confirms 27 conversions |
| `src/config.rs` | 16 pub items converted to pub(crate) | VERIFIED | File contains pub(crate); commit 3d4483a confirms 16 conversions |
| `src/cli.rs` | pub fn run() -> pub(crate) fn run() | VERIFIED | `pub(crate) fn run() -> Result<()>` found at expected location |
| `src/lib.rs` | Only remaining pub fn run() entry point | VERIFIED | Only `pub fn run()` remains; all other items are pub(crate) or private |
| `Cargo.toml` | redundant_pub_crate = "allow" clippy lint | VERIFIED | Line 91: `redundant_pub_crate = "allow"` confirmed |

All 14 non-testutil source modules contain pub(crate) (verified via `rg 'pub\(crate\)' src/ -l`):
cache.rs, cli.rs, config.rs, detection.rs, fuzzy.rs, github.rs, json_merge.rs, lib.rs,
overlay_name.rs, overlay_repo.rs, reference.rs, selection.rs, sources.rs, state.rs, upstream.rs

#### Plan 03-02 Artifacts (Manual Test Documents)

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `docs/manual-tests/apply.md` | apply command manual tests | VERIFIED | 279 lines; 9 TC headings (8 + 1 optional); contains "repoverlay apply" 13 times |
| `docs/manual-tests/remove.md` | remove command manual tests | VERIFIED | 154 lines; 4 TC headings; contains "repoverlay remove" 12 times |
| `docs/manual-tests/status.md` | status command manual tests | VERIFIED | 173 lines; 6 TC headings; contains "repoverlay status" 15 times |
| `docs/manual-tests/restore.md` | restore command manual tests | VERIFIED | 136 lines; 4 TC headings; contains "repoverlay restore" 6 times |
| `docs/manual-tests/update.md` | update command manual tests | VERIFIED | 177 lines; 5 TC headings; contains "repoverlay update" 13 times |
| `docs/manual-tests/create.md` | create command manual tests | VERIFIED | 167 lines; 5 TC headings; contains "repoverlay create" 6 times |
| `docs/manual-tests/switch-browse.md` | switch/browse manual tests | VERIFIED | 195 lines; 6 TC headings; contains "repoverlay switch" 3 times + browse cmds |
| `docs/manual-tests/source-management.md` | source subcommand manual tests | VERIFIED | 154 lines; 6 TC headings; contains "repoverlay source" 10 times |

Total: 45 TC headings across 8 documents (41 unique offline + 4 optional network).

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| `src/main.rs` | `src/lib.rs pub fn run()` | `repoverlay::run()` call | VERIFIED | Line 18: `if let Err(e) = repoverlay::run()` |
| `src/lib.rs` | `src/cli.rs pub(crate) fn run()` | crate-internal `cli::run()` | VERIFIED | Line 26: `cli::run()` inside pub fn run() body |
| `docs/manual-tests/apply.md` | repoverlay apply CLI | copy-pasteable commands | VERIFIED | 8 TC headings with bash blocks; mktemp setup; Expected Output sections throughout |
| `docs/manual-tests/source-management.md` | repoverlay source add/list/remove CLI | copy-pasteable commands | VERIFIED | TC-01 through TC-04 cover all three subcommands |

### Requirements Coverage

| Requirement | Source Plan | Description | Status | Evidence |
|-------------|-------------|-------------|--------|----------|
| API-01 | 03-01-PLAN.md | Public API surface reviewed (pub vs pub(crate) boundaries) | SATISFIED | 14 modules converted; only lib::run() remains pub |
| API-02 | 03-01-PLAN.md | #[non_exhaustive] skipped per user decision | SATISFIED | No #[non_exhaustive] in codebase; documented in REQUIREMENTS.md as intentional skip |
| API-03 | 03-01-PLAN.md | Public API documented with appropriate doc comments | SATISFIED | All 17 modules have //! doc comments; 4 enhanced with additional context |
| MANUAL-01 | 03-02-PLAN.md | CLI walkthrough for apply (local + GitHub) | SATISFIED | apply.md: 8 TC cases including optional GitHub |
| MANUAL-02 | 03-02-PLAN.md | CLI walkthrough for remove | SATISFIED | remove.md: 4 TC cases covering named, --all, --dry-run, non-existent |
| MANUAL-03 | 03-02-PLAN.md | CLI walkthrough for status | SATISFIED | status.md: 6 TC cases covering empty, applied, --json, --quiet, --name |
| MANUAL-04 | 03-02-PLAN.md | CLI walkthrough for restore | SATISFIED | restore.md: 4 TC cases covering basic, --dry-run, --force |
| MANUAL-05 | 03-02-PLAN.md | CLI walkthrough for update | SATISFIED | update.md: 5 TC cases including optional GitHub |
| MANUAL-06 | 03-02-PLAN.md | CLI walkthrough for create | SATISFIED | create.md: 5 TC cases covering --output, --include, --dry-run, --yes, --force |
| MANUAL-07 | 03-02-PLAN.md | CLI walkthrough for switch/browse | SATISFIED | switch-browse.md: 6 TC cases for both commands |
| MANUAL-08 | 03-02-PLAN.md | CLI walkthrough for source management (add, list, remove) | SATISFIED | source-management.md: 6 TC cases covering add, list, remove, duplicate, GitHub |

All 11 requirement IDs declared across both plans are satisfied. No orphaned requirements found for Phase 3.

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| `src/lib.rs` | 413 | `// TODO: In a future version, require ./` | Info | Design note for future improvement; no incomplete implementation |
| `src/reference.rs` | 93 | `// TODO: In a future version, require ./` | Info | Same design note in parsing logic; not a stub |
| `src/cli.rs` | 3679 | `"# TODO"` | Info | Inside test helper writing a test file named "todo.md"; not a code TODO |
| `src/detection.rs` | 415 | `"# TODO"` | Info | Inside test helper writing a test file named "todo.md"; not a code TODO |

No blocker anti-patterns. The TODO comments in lib.rs and reference.rs are documented future-version design notes (identical wording, intentional). The remaining two are test fixture file content, not incomplete logic.

### ROADMAP Success Criteria Assessment

The ROADMAP lists 5 success criteria for Phase 3. One requires attention:

**Criterion 2:** "Extensible public enums and structs have `#[non_exhaustive]` applied"

This criterion was explicitly waived via user decision before plan execution. The decision is documented in:
- REQUIREMENTS.md: `API-02: #[non_exhaustive] skipped per user decision (binary-only, no external consumers)`
- 03-01-PLAN.md frontmatter: `#[non_exhaustive] is NOT added anywhere (skipped per user decision for binary-only 1.0)`
- 03-01-SUMMARY.md: `No #[non_exhaustive] attributes added (API-02 skipped per user decision)`

The remaining 4 success criteria are fully satisfied.

### Human Verification Required

#### 1. Full Manual Test Walkthrough (Apply Command)

**Test:** Follow docs/manual-tests/apply.md TC-01 through TC-07 with the installed repoverlay binary
**Expected:** Symlinks are created in target repo, git exclude is updated, --copy produces actual file copies, --dry-run makes no changes, --force re-applies cleanly, --skip-conflicts skips conflicting files while applying others, --merge deep-merges JSON files
**Why human:** Requires installed binary, live filesystem operations, and visual inspection of symlink vs copy file types

#### 2. Source Management Workflow

**Test:** Follow docs/manual-tests/source-management.md TC-01 through TC-04
**Expected:** `repoverlay source add` registers a source in config, `source list` displays it, `source remove` deletes it, adding a duplicate is handled gracefully
**Why human:** Requires installed binary and confirms live config file manipulation

### Gaps Summary

No gaps. All automated checks passed.

All 11 requirements (API-01, API-02, API-03, MANUAL-01 through MANUAL-08) are satisfied. The public API surface is locked down with pub(crate) across all internal modules, module documentation is complete, and 8 substantive manual test documents exist covering all CLI workflows with 41+ test cases using copy-pasteable commands and expected output sections.

---

_Verified: 2026-03-03_
_Verifier: Claude (gsd-verifier)_
