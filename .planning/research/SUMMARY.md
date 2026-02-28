# Project Research Summary

**Project:** repoverlay 1.0 Stabilization
**Domain:** Rust CLI tool stabilization, code review, and release preparation
**Researched:** 2026-02-27
**Confidence:** HIGH

## Executive Summary

Repoverlay is a feature-complete Rust CLI tool (v0.8.0) that overlays configuration files into Git repositories via symlinks or copies. The 1.0 milestone is a stabilization effort — not feature development — with a clear mandate: verify every command works correctly, close identified test coverage gaps, create a manual test suite, and execute a release. The core stack (clap, anyhow, serde, sickle/CCL, cargo-dist, release-plz) is already in place and working. The two meaningful tool additions for this milestone are `cargo-semver-checks` (verify the public API surface before the 1.0 tag) and `cargo-mutants` (find behaviors that existing tests execute but do not actually verify).

The recommended approach is a structured code review ordered by dependency depth: leaf modules first (parsing, name validation), then infrastructure (state, config, cache), then resolution logic (sources, overlay_repo), then UI (selection), and finally the core orchestrators (lib.rs, cli.rs). This ordering ensures that the data model and persistence are verified correct before reviewing the operations that depend on them. The two largest files — lib.rs (~7200 lines) and cli.rs (~8000 lines) — carry the highest bug density and should receive proportionally heavier review effort.

The most critical risks for 1.0 are well-identified: debug-format error messages leaking to users (`{e:?}` instead of `{e:#}` in main.rs), missing SIGPIPE handling causing broken pipe panics in scripts, Linux-only CI while shipping binaries for five targets, state file format lock-in without a migration path, and Windows symlink failures with no actionable guidance. None of these are difficult to fix, but all of them must be caught before the tag is created since crates.io publishes are permanent and the 1.0 API surface becomes a semver contract.

## Key Findings

### Recommended Stack

The existing stack requires no core changes. The toolchain is already strong: clippy with pedantic+nursery lints, cargo-llvm-cov for LLVM-based coverage, cargo-nextest for fast parallel test execution, cargo-audit and cargo-deny for supply chain security, release-plz for automated release PRs, cargo-dist for multi-platform binaries, and changie for structured changelog management.

Two new tools are recommended for the stabilization milestone. `cargo-semver-checks` (0.44+, 245 lints) should run pre-release to verify no unintentional breaking changes exist between v0.8.0 and v1.0.0 — its MSRV matches the project's 1.90 exactly. `cargo-mutants` (26.2.0, CalVer) provides mutation testing to find code that tests execute but whose behavior is not actually verified. For snapshot testing CLI output stability before 1.0, `trycmd` (0.15+) complements the existing `assert_cmd` tests by enabling bulk output verification written as markdown files.

**Core technologies:**
- `cargo-semver-checks 0.44+`: Public API verification before 1.0 tag — catches unintentional breaking changes
- `cargo-mutants 26.2.0`: Mutation testing to find untested behaviors — coverage tells you what's reached, mutations tell you what's verified
- `trycmd 0.15+`: Snapshot testing for CLI output stability — complements assert_cmd for help text, error message, and output format verification
- `cargo-llvm-cov`: Already configured — continue using for coverage baseline and PR annotations

### Expected Features

The feature research reframes "features" as the quality attributes required for a credible 1.0 release of a production CLI tool. The focus is not adding functionality — the tool is feature-frozen — but verifying and hardening what exists.

**Must have (table stakes for 1.0):**
- All documented commands work correctly — verified by code review and test suite
- User-friendly error messages across all failure modes — anyhow Display, not Debug
- Fix all bugs discovered during review — 1.0 cannot ship with known issues
- Test coverage for identified gaps — Windows error paths, path traversal, terminal recovery, cache failure recovery
- Public API surface reviewed and locked with `#[non_exhaustive]` where needed
- Manual test suite documenting real-world scenarios (8 core workflows, cross-platform, error handling)
- Complete 1.0 changelog compiled from changie entries
- README reviewed and accurate for 1.0

**Should have (differentiate quality):**
- `trycmd` snapshot tests locking CLI output format before 1.0
- Cross-platform CI matrix (Windows + macOS runners added)
- SIGPIPE handling for clean pipe behavior
- Distinct exit codes for different error categories (not just "exit 1 always")

**Defer to 1.x:**
- Man page generation via `clap_mangen`
- Property-based testing for state serialization
- JSON output extension to more commands
- Cache size reporting and automatic eviction
- Refactoring `apply_overlay_internal` (~350 lines) into smaller units

**Defer to 2.0+:**
- State format migration from CCL to a standard format
- Plugin system for custom source resolvers

### Architecture Approach

The codebase is organized as a flat 15-module Rust library with a thin `main.rs` entry point and a dedicated `cli.rs` command dispatch layer. All overlay operations are centralized in `lib.rs`, which orchestrates the specialized modules. The recently introduced `SourceResolver` trait (PR #150) centralizes source-type dispatch so individual commands don't independently pattern-match on `OverlaySource` variants — this is the right abstraction and review should verify its implementation is complete across all code paths.

The dual-location state persistence pattern (in-repo at `.repoverlay/overlays/` and external at `~/.local/share/repoverlay/applied/`) is architecturally sound for surviving `git clean` operations, but requires that both locations stay synchronized. Any bug in the external backup path creates a gap in the restore guarantee. State file format integrity is the single highest-risk architectural concern for post-1.0 compatibility.

**Major components:**
1. `lib.rs` (~7200 lines) — Core operations: apply, remove, status, restore, update, create, switch; highest bug density; review last but most thoroughly
2. `cli.rs` (~8000 lines) — Command dispatch and per-command logic; contains 9 issue-specific bug-fix tests from issues #142-#148
3. `state.rs` (~1700 lines) — Data models, SourceResolver trait, CCL serialization, dual-location persistence; review second (infrastructure layer)
4. `selection.rs` (~3000 lines) — Interactive TUI for file selection; complex terminal state machine; untestable render paths require manual verification
5. `sources.rs` + `overlay_repo.rs` (~1400 lines combined) — Multi-source resolution with priority ordering and upstream fork fallback; subtle correctness requirements
6. `cache.rs` + `github.rs` (~1400 lines combined) — GitHub repository caching and URL parsing; security-sensitive (flag injection validation)

### Critical Pitfalls

1. **Debug error display leaking to users** — `main.rs` uses `{e:?}` (Debug) instead of `{e:#}` (anyhow alternate Display). Switch immediately; test by asserting no `Os {` patterns appear in integration test stderr output.

2. **No SIGPIPE handling** — Rust masks SIGPIPE by default; piped commands (`repoverlay status | head -1`) print "Error: Broken pipe" instead of exiting cleanly. Fix: detect `ErrorKind::BrokenPipe` in the error handler and exit 0 silently. One-line fix that must not ship at 1.0.

3. **Linux-only CI shipping five-platform binaries** — `cargo-dist` produces binaries for Linux x86/aarch64, macOS universal, and Windows x86_64. Current CI only runs on `ubuntu-latest`. Add `windows-latest` and `macos-latest` to the test matrix before release candidate.

4. **State file format lock-in without migration** — The `sickle` (CCL) state files include a `version: 1` marker but no migration logic. Before 1.0, freeze the schema, write round-trip serialization tests for every `OverlayState` variant, and add `#[serde(deny_unknown_fields)]` to make format mismatches explicit rather than silently corrupting state.

5. **Windows symlink failures with no actionable guidance** — OS error 1314 from Windows symlink creation is surfaced as a raw error. Before 1.0, detect the error and display: "Use --copy mode or enable Developer Mode." Consider making `--copy` the default on Windows.

## Implications for Roadmap

Based on the research, the natural phase structure follows the dependency ordering identified in the architecture research: establish correct foundations first, then verify the logic that builds on those foundations, then validate the full system.

### Phase 1: Code Review — Foundation Modules
**Rationale:** Leaf modules (overlay_name, fuzzy, json_merge, github, upstream, reference) have no crate dependencies and are prerequisites for everything else. Correctness here is foundational — bugs in parsing propagate to all consumers. These are well-tested with focused unit tests; estimated effort is light.
**Delivers:** Verified correctness of all leaf-level parsing and utility code; identifies any security issues (flag injection, path traversal) at the lowest layer.
**Addresses:** "All documented commands work correctly" (table stakes), git flag injection security concern.
**Avoids:** Missing security validation in parsing layers before they are covered by the broader review.

### Phase 2: Code Review — Infrastructure Modules
**Rationale:** State, config, and cache modules form the data backbone. State bugs corrupt user data; they must be verified before reviewing the operations that manipulate state. The `SourceResolver` trait (PR #150) lives here and needs exhaustiveness verification.
**Delivers:** Verified state serialization correctness, CCL round-trip tests, cache metadata consistency under partial failures.
**Addresses:** State file format lock-in pitfall, `sickle` dependency risk, atomic write correctness.
**Avoids:** Discovering state corruption bugs late in the review cycle after they have compounded with higher-level logic errors.

### Phase 3: Code Review — Resolution and Support Modules
**Rationale:** Source resolution (sources.rs, overlay_repo.rs) and UI support (detection.rs, selection.rs) are mid-layer. Resolution bugs produce incorrect overlays silently — the worst kind of failure. Selection.rs terminal state management requires manual verification for Ctrl+C recovery.
**Delivers:** Verified multi-source priority ordering, upstream fallback correctness, path traversal security in overlay_repo, terminal state recovery.
**Addresses:** Path traversal security, interactive selection edge cases, source resolution priority bugs.
**Avoids:** Discovering resolution priority bugs only during manual testing after all automated review is complete.

### Phase 4: Code Review — Core Operations (lib.rs + cli.rs)
**Rationale:** These are reviewed last because they depend on all other modules being verified first. lib.rs and cli.rs are the two largest files (~15K lines combined) with the highest expected bug density. This phase includes the 9 issue-specific bug-fix tests from cli.rs.
**Delivers:** Verified apply/remove/status/restore/update/switch operations; conflict strategy correctness; JSON merge integration; git exclude management.
**Addresses:** "No silent data loss," conflict strategy correctness, absolute symlinks tech debt, SIGPIPE fix, debug error display fix.
**Avoids:** Reviewing orchestration logic without first verifying the components it orchestrates.

### Phase 5: Test Coverage and Infrastructure
**Rationale:** After code review is complete and bugs are identified, fill in test coverage gaps. Coverage analysis (cargo-llvm-cov) identifies untested lines; mutation testing (cargo-mutants) identifies untested behaviors. New tests verify the *fixed* behavior, not the pre-fix behavior.
**Delivers:** Increased test coverage for error paths, Windows-specific code paths, path traversal edge cases, terminal recovery scenarios, state serialization round-trips. Integration of `trycmd` for CLI output snapshot tests.
**Uses:** `cargo-llvm-cov` (existing), `cargo-mutants` (new), `trycmd` (new), `insta` (optional).
**Addresses:** All identified coverage gaps from CONCERNS.md, Windows symlink error paths.
**Avoids:** Writing tests that lock in buggy behavior (test after fix, not before).

### Phase 6: Cross-Platform and Error Handling Polish
**Rationale:** Cross-platform CI and error message quality are pre-release gates. CI matrix expansion and error display fixes are independent of code review, so they can proceed in parallel or sequentially after Phase 4 as CI time allows.
**Delivers:** Windows CI job, macOS CI job, user-friendly error messages (switching to `{e:#}`), SIGPIPE handling, actionable Windows symlink guidance, distinct exit codes.
**Addresses:** Linux-only CI pitfall, debug error display pitfall, Windows symlink pitfall.
**Avoids:** Releasing 1.0 with the first Windows user encounter being an inscrutable OS error 1314.

### Phase 7: Manual Test Suite
**Rationale:** Manual tests validate real-world workflows that automated tests cannot fully exercise: interactive prompts, actual terminal behavior, multi-step overlay workflows. This phase requires all code fixes to be landed.
**Delivers:** Documented manual test scenarios for 8 core workflows (CW-01 through CW-08), 4 cross-platform scenarios (CP-01 through CP-04), 10 error handling scenarios (EH-01 through EH-10), and 7 source management scenarios (SM-01 through SM-07). Located at `docs/manual-tests/`.
**Addresses:** Manual test suite requirement in PROJECT.md active requirements list.
**Avoids:** Releasing 1.0 without validating the full user experience end-to-end.

### Phase 8: Release Preparation
**Rationale:** Final gate before the 1.0 tag. Public API review, semver verification, changelog compilation, and crates.io metadata verification. This phase is last because it requires all other work to be complete.
**Delivers:** `#[non_exhaustive]` on extensible public types, `cargo-semver-checks` clean output, complete changie-based 1.0 changelog, verified crates.io metadata, release-plz PR merged, cargo-dist binaries built and uploaded.
**Uses:** `cargo-semver-checks 0.44+` (new), release-plz (existing), cargo-dist (existing), changie (existing).
**Addresses:** Semver commitment pitfall, public API stabilization, changelog compilation.
**Avoids:** Discovering a breaking API change after the crates.io publish is permanent.

### Phase Ordering Rationale

- Phases 1-4 follow the dependency-depth ordering from architecture research: leaf modules before infrastructure before resolution before orchestration. This prevents the scenario where high-level logic bugs mask lower-level bugs.
- Phase 5 (test coverage) deliberately follows Phase 4 because tests should verify fixed behavior, not lock in bugs discovered during review.
- Phase 6 (cross-platform + error polish) is largely independent but benefits from having the code review complete so fixes and UX improvements are not duplicated.
- Phase 7 (manual tests) requires all automated fixes to be stable — manually testing an intermediate state produces false results.
- Phase 8 (release prep) is a strict dependency on all prior phases being complete.

### Research Flags

Phases with well-documented patterns (standard — skip additional research-phase):
- **Phase 1:** Leaf module review — standard Rust code review practices
- **Phase 2:** Infrastructure review — well-understood state management patterns
- **Phase 5:** Coverage tooling — all tools already identified with exact versions
- **Phase 8:** Release process — already automated via release-plz + cargo-dist

Phases that may benefit from targeted research during planning:
- **Phase 6:** Cross-platform CI matrix setup — macOS runner costs and caching strategy for Rust toolchain may need research to avoid slow CI
- **Phase 7:** Manual test format — the markdown format for manual test docs is suggested but the team may want to evaluate alternatives (e.g., UAT-style checklists vs step-by-step procedures)

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Core stack is existing and working; new tool recommendations (cargo-semver-checks, cargo-mutants) are verified against current versions with matching MSRV |
| Features | HIGH | Feature scope is well-defined by feature-freeze constraint; all P1 requirements are specific and actionable |
| Architecture | HIGH | Research based on direct codebase analysis of all 16 source files; module line estimates and dependency graph are derived from actual code |
| Pitfalls | HIGH | 6 critical pitfalls are all verified against the actual codebase (main.rs error format, CI yml, state.rs structs, Windows symlink code paths); not speculative |

**Overall confidence:** HIGH

### Gaps to Address

- **`sickle` crate health:** The research flags `sickle` (v0.1.2) as a risk dependency for CCL parsing but does not have a definitive assessment of its maintenance status (commit frequency, responsiveness to issues). During Phase 2, the team should evaluate whether to pin to `=0.1.2`, vendor it, or fork it before the 1.0 commitment.
- **Actual coverage percentage:** The research identifies *categories* of coverage gaps (Windows paths, path traversal, terminal recovery) but does not have the current line/branch coverage percentage as a baseline. Running `just test-coverage` at the start of Phase 5 will establish this baseline.
- **Issue #142-#148 bug status:** `cli.rs` is noted to contain 9 issue-specific bug-fix tests from issues #142-#148 but the research does not confirm whether those issues are fully resolved or partially fixed. Phase 4 must explicitly verify completion of each.
- **Windows CI cost:** Adding `windows-latest` and `macos-latest` to the CI matrix will increase build time. The team should decide whether to run the full test suite on all platforms or a targeted subset.

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis: all 16 source files in `src/`, `tests/cli.rs`, `tests/common/mod.rs`
- `.planning/PROJECT.md` — milestone requirements and constraints
- `.planning/codebase/ARCHITECTURE.md`, `TESTING.md`, `CONCERNS.md` — existing documentation
- `ARCHITECTURE.md` (repo root) — module structure and data flows
- `DEV.md` (repo root) — build commands and CI process
- [cargo-semver-checks](https://github.com/obi1kenobi/cargo-semver-checks) — v0.44.0, 245 lints
- [cargo-mutants docs](https://mutants.rs/) — v26.2.0
- [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) — v0.8.4
- [cargo-dist](https://github.com/axodotdev/cargo-dist) — v0.30.3
- [release-plz docs](https://release-plz.dev/) — v0.3.156

### Secondary (MEDIUM confidence)
- [Rain's Rust CLI Recommendations](https://rust-cli-recommendations.sunshowers.io/) — CLI best practices
- [sharkdp's Release Checklist](https://dev.to/sharkdp/my-release-checklist-for-rust-programs-1m33) — real-world release process from fd/bat/hyperfine author
- [The Rust CLI Book](https://rust-cli.github.io/book/index.html) — official Rust CLI guidance
- [Effective Rust - SemVer Promises](https://effective-rust.com/semver.html) — guidance on 1.0 commitment
- [trycmd docs](https://docs.rs/trycmd/latest/trycmd/) — snapshot testing for CLIs

### Tertiary (supporting)
- Commit history: PR #150 (SourceResolver abstraction), PR #152 (milestone cleanup)
- Cargo.toml dependency audit for version confirmation
- `.github/workflows/ci.yml` analysis for platform coverage gap verification

---
*Research completed: 2026-02-27*
*Ready for roadmap: yes*
