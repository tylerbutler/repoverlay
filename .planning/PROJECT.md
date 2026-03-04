# Repoverlay

## What This Is

Repoverlay is a CLI tool for overlaying configuration files onto Git repositories using symlinks (or copies on Windows). It supports multiple overlay sources (local filesystem, GitHub, shared overlay repositories), conflict resolution strategies, and persistent state management. v0.8 completed a full stabilization cycle: code review, test coverage hardening, API locking, and release preparation.

## Core Value

Every feature that ships must work correctly and be verified — no silent failures, no untested code paths, no surprises.

## Requirements

### Validated

- ✓ Apply overlays from local directories — existing
- ✓ Apply overlays from GitHub repositories with caching — existing
- ✓ Apply overlays from shared overlay repositories — existing
- ✓ Remove applied overlays and clean up state — existing
- ✓ Show status of applied overlays — existing
- ✓ Restore overlays from external state backups — existing
- ✓ Update GitHub overlays to latest commits — existing
- ✓ Create new overlays from detected files with interactive selection — existing
- ✓ Switch between overlays — existing
- ✓ Browse available overlays — existing
- ✓ Multi-source resolution with priority ordering — existing
- ✓ Upstream fork fallback resolution — existing
- ✓ Conflict strategies (Fail, Force, SkipConflicts, Interactive) — existing
- ✓ JSON deep merge via --merge flag — existing
- ✓ Git exclude file management — existing
- ✓ Dual-location state persistence (in-repo + external) — existing
- ✓ Cross-platform support (Linux, macOS, Windows) — existing
- ✓ Shell completion generation — existing
- ✓ Global and per-repo configuration via CCL format — existing
- ✓ Comprehensive code review across all modules — v0.8
- ✓ All discovered bugs fixed (error display, SIGPIPE) — v0.8
- ✓ Test coverage for untested code paths (path traversal, cache failure, mutation testing) — v0.8
- ✓ Manual test suite with 41 test cases across 8 CLI workflows — v0.8
- ✓ README and crates.io metadata ready for release — v0.8

### Active

(None — define with next milestone)

### Out of Scope

- New features — 1.0 is feature-complete, this is stabilization only
- Performance optimization — unless correctness issues are found
- Refactoring for style — only fix actual bugs and coverage gaps
- Offline mode — real-time GitHub caching is sufficient

## Context

Shipped v0.8 stabilization with ~31,000 LOC Rust.
Tech stack: Rust 2024 edition (min 1.90), clap, anyhow, serde, sickle (CCL parser).
Clippy pedantic + nursery lints enabled.
Tests: 925 unit + 97 integration tests, mutation testing baseline established.
CI: GitHub Actions with test, lint, format, coverage (cargo-llvm-cov + codecov), security audits.
Distribution: cargo-dist (Linux, macOS, Windows binaries + Homebrew).
Manual test suite: 41 test cases across 8 CLI workflows in docs/manual-tests/.

## Constraints

- **Backward compatibility**: No breaking changes to CLI interface or state format
- **Test isolation**: Tests must not interfere with each other (use TempDir, isolated configs)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Feature-complete for 1.0 | Focus on quality over quantity | ✓ Good — zero correctness bugs found in review |
| Fix all discovered bugs | Ship with confidence, not known issues | ✓ Good — FIX-02 (Display format), FIX-03 (SIGPIPE), FIX-04 (issues #142-#148) |
| Manual + automated test suite | Automated catches regressions, manual verifies real workflows | ✓ Good — 41 manual test cases + mutation testing baseline |
| pub(crate) API locking | Binary-only tool, no external consumers | ✓ Good — only lib::run() is public |
| Skip #[non_exhaustive] | Binary-only, no external consumers | ✓ Good — avoids unnecessary boilerplate |
| Allow clippy::redundant_pub_crate | Explicit visibility in private modules | ✓ Good — clarity over lint silence |
| Mutation testing with scoped targets | Full mutation run too expensive | ✓ Good — caught real gaps in error propagation |

---
*Last updated: 2026-03-04 after v0.8 milestone*
