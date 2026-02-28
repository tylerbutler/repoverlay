# Repoverlay 1.0 Stabilization

## What This Is

Repoverlay is a CLI tool for overlaying configuration files onto Git repositories using symlinks (or copies on Windows). It supports multiple overlay sources (local filesystem, GitHub, shared overlay repositories), conflict resolution strategies, and persistent state management. This milestone focuses on comprehensive code review, correctness verification, test coverage improvements, and 1.0 release preparation.

## Core Value

Every feature that ships in 1.0 must work correctly and be verified — no silent failures, no untested code paths, no surprises.

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

### Active

- [ ] Comprehensive code review across all modules (correctness, edge cases, coverage gaps)
- [ ] Fix all bugs discovered during review
- [ ] Add missing test coverage for untested code paths
- [ ] Create manual test suite (CLI walkthroughs + real-world scenarios)
- [ ] Prepare changelog and release documentation for 1.0

### Out of Scope

- New features — 1.0 is feature-complete, this is stabilization only
- Performance optimization — unless correctness issues are found
- Refactoring for style — only fix actual bugs and coverage gaps

## Context

- Rust 2024 edition, minimum version 1.90
- Clippy pedantic + nursery lints enabled
- Tests use tempfile::TempDir for temporary git repos
- Integration tests in tests/cli.rs (~1844 lines), unit tests inline
- changie already configured for changelog management
- CI: GitHub Actions with test, lint, format, coverage, security audits
- Coverage reporting via cargo-llvm-cov + codecov
- Distribution via cargo-dist (Linux, macOS, Windows binaries + Homebrew)

## Constraints

- **Feature freeze**: No new features — only correctness fixes and test additions
- **Backward compatibility**: No breaking changes to CLI interface or state format
- **Test isolation**: Tests must not interfere with each other (use TempDir, isolated configs)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Feature-complete for 1.0 | Focus on quality over quantity | — Pending |
| Fix all discovered bugs | Ship with confidence, not known issues | — Pending |
| Manual + automated test suite | Automated catches regressions, manual verifies real workflows | — Pending |

---
*Last updated: 2026-02-27 after initialization*
