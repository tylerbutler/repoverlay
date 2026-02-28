# Stack Research

**Domain:** Rust CLI stabilization, code review, test coverage, and release preparation
**Researched:** 2026-02-27
**Confidence:** HIGH

## Context

This research is scoped to a **subsequent milestone** for an existing Rust CLI tool (repoverlay v0.8.0). The core stack is already established and working. This document covers *additional* tooling needed for 1.0 stabilization: code review automation, test coverage analysis, test quality verification, and release preparation. It does NOT re-recommend the existing core stack (clap, serde, anyhow, etc.).

## Recommended Stack

### Code Review & Static Analysis

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| clippy (pedantic + nursery) | rustc-bundled | Static lint analysis across all code paths | **Already configured.** Pedantic + nursery catches subtle correctness issues, not just style. The existing allowed-lint list is reasonable. No changes needed. Confidence: HIGH |
| cargo-semver-checks | 0.44+ | API compatibility verification before 1.0 tag | Catches unintentional breaking changes between 0.8.0 and 1.0.0. Now has 245 lints (doubled in 2025). MSRV 1.90 matches project MSRV exactly. Will be built into cargo eventually. Run pre-release to verify public API surface is intentional. Confidence: HIGH |
| cargo-audit | 0.22.1 | Dependency vulnerability scanning | **Already configured** in justfile and CI. Checks against RustSec Advisory Database. Confidence: HIGH |
| cargo-deny | 0.19.0 | Supply chain security (licenses, bans, advisories, sources) | **Already configured** with deny.toml. Goes beyond cargo-audit with license compliance and dependency ban lists. Confidence: HIGH |

### Test Coverage Analysis

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| cargo-llvm-cov | 0.8.4 | LLVM source-based code coverage reporting | **Already configured** in justfile and CI. Generates lcov.info for Codecov. The de facto standard for Rust coverage -- uses LLVM's instrumentation, not sampling. Supports line, region, and branch coverage. Confidence: HIGH |
| Codecov | SaaS | Coverage tracking, PR annotations, trend analysis | **Already configured** with codecov.yml. Shows coverage diffs on PRs. The 2% threshold with auto-target is appropriate for stabilization. Confidence: HIGH |

### Test Quality & Completeness

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| cargo-mutants | 26.2.0 | Mutation testing to find untested code behaviors | Coverage tells you what code is *reached* by tests. Mutation testing tells you whether tests actually *verify* behavior. Injects bugs (replacing return values, removing function bodies) and checks if tests catch them. CalVer versioning. Integrates with nextest via `--test-tool=nextest`. Emits GitHub Actions annotations. Confidence: HIGH |
| cargo-nextest | 0.9.116+ | Parallel test execution with better reporting | **Already configured** in justfile as `test-fast`. Faster than `cargo test` for large test suites. Provides per-test timing, retries, and JUnit XML output. Confidence: HIGH |

### Test Frameworks (Existing + Additions)

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| assert_cmd | 2 | CLI integration testing | **Already in use.** Primary tool for testing CLI binary behavior. Keep using for targeted assertions on specific command behaviors. Confidence: HIGH |
| predicates | 3 | Test output assertions | **Already in use.** Paired with assert_cmd. Confidence: HIGH |
| tempfile | 3 | Temporary test directories | **Already in use.** Critical for test isolation. Confidence: HIGH |
| trycmd | 0.15+ | Snapshot testing for CLI output | **New recommendation.** Complements assert_cmd for bulk output verification. Tests are written as markdown/TOML files, making them living documentation. Cross-platform normalization built in. Use for: help text verification, error message consistency, output format stability before 1.0. Confidence: HIGH |
| insta | 1.46+ | Snapshot testing for structured data | **New recommendation.** Use for: JSON state file format verification, config parsing output, status command output. Supports inline snapshots and interactive review via `cargo-insta review`. Confidence: MEDIUM (useful but not essential if trycmd covers CLI output) |

### Release Preparation

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| release-plz | 0.3.156 (action v0.5) | Automated version bumping and release PR creation | **Already configured** in CI. Creates release PRs from conventional commits. Integrates with cargo-semver-checks automatically. Manages git tags that trigger cargo-dist. Confidence: HIGH |
| cargo-dist | 0.30.3 | Multi-platform binary distribution | **Already configured** with dist-workspace.toml. Produces binaries for Linux (x86_64, aarch64), macOS (universal), Windows (x86_64). Generates shell/PowerShell/Homebrew installers. Confidence: HIGH |
| changie | 1.22.1 | Changelog management | **Already configured** with .changie.yaml. Component-based changelog entries per CLI command. Note: release-plz is configured with `changelog_update = false` so changie manages the changelog, while release-plz handles version bumps and tagging. Confidence: HIGH |

### Development Workflow

| Tool | Purpose | Notes |
|------|---------|-------|
| just | Task runner | **Already configured.** Comprehensive justfile with aliases. No changes needed. |
| mise | Dev tool version management | **Already configured.** Manages changie, hk, just, python versions. |
| cargo-watch | File watching for dev loop | **Already configured.** `just watch-test` and `just watch-lint`. |

## Installation

```bash
# New tools for stabilization (not already installed)
cargo install cargo-semver-checks --locked
cargo install cargo-mutants

# Optional: snapshot testing additions
cargo add --dev trycmd
cargo add --dev insta --features json

# Already installed (verify versions)
cargo install cargo-llvm-cov
cargo install cargo-nextest --locked
cargo install cargo-audit
cargo install cargo-deny --locked
```

## Alternatives Considered

| Recommended | Alternative | When to Use Alternative |
|-------------|-------------|-------------------------|
| cargo-llvm-cov | grcov | Never for this project. grcov uses gcov-based instrumentation which is less precise. cargo-llvm-cov uses LLVM source-based coverage which is the Rust ecosystem standard. |
| cargo-mutants | mutest-rs | Never. mutest-rs requires nightly Rust and compiler plugins. cargo-mutants works on stable Rust with no source changes. |
| trycmd | snapbox | When you need to customize trycmd's behavior or need one-off snapshot assertions. snapbox is the underlying engine. trycmd is higher-level and better for bulk CLI testing. |
| assert_cmd | rexpect | Only for testing interactive prompts (dialoguer). rexpect controls a PTY, which is needed for interactive input. assert_cmd cannot test interactive flows. |
| proptest | quickcheck | Never for this project. proptest has better strategy composition, better shrinking, and constraint-aware generation. quickcheck is simpler but less flexible. |
| insta | expect-test (rust-analyzer) | If you prefer inline-only snapshots. insta is more mature, more widely used, and has both inline and file-based snapshots plus an interactive review tool. |
| changie | git-cliff | Never. changie is already configured with per-component entries matching repoverlay's CLI commands. Switching would lose this structure. |
| release-plz | cargo-release | Never. release-plz is already configured and integrates with cargo-dist's tag-based release workflow. cargo-release is more manual. |

## What NOT to Use

| Avoid | Why | Use Instead |
|-------|-----|-------------|
| grcov | Less precise than LLVM source-based coverage; requires gcov instrumentation; extra build complexity | cargo-llvm-cov |
| tarpaulin | Linux-only, uses ptrace-based instrumentation which is slower and less accurate than LLVM-based coverage | cargo-llvm-cov |
| mutest-rs | Requires nightly compiler; compiler plugin stability concerns; not production-ready | cargo-mutants |
| cargo-fuzz | Overkill for a CLI config tool. Fuzzing is for finding memory safety bugs in parsers/network code. repoverlay delegates parsing to sickle/serde. | cargo-mutants + proptest (if needed) |
| proptest (for now) | Property testing is valuable but adds complexity to the stabilization timeline. The project's primary risk is *untested paths*, not *incorrect algorithms*. Coverage + mutation testing addresses this more directly. | Defer to post-1.0. Add proptest for config parsing and state file round-trip properties later. |
| custom test harness | The built-in test framework + assert_cmd + trycmd covers all needs. A custom harness (libtest-mimic) adds maintenance burden with no benefit for this project. | Standard `#[test]` + assert_cmd + trycmd |
| Miri | Memory safety tool for unsafe code. repoverlay has minimal unsafe code (just a `warn` lint, not `forbid`). Miri is expensive to run and the project doesn't need it. | clippy's unsafe-related lints |

## Stack Patterns by Milestone Phase

**If doing code review/audit:**
- Run `cargo clippy --all-targets --all-features` (existing)
- Run `cargo-semver-checks check-release` (NEW -- verify public API before 1.0)
- Run `cargo audit && cargo deny check` (existing)
- Run `cargo-mutants` on critical modules to find behavioral gaps (NEW)

**If improving test coverage:**
- Run `just test-coverage` to generate baseline lcov.info (existing)
- Run `just coverage-html` to identify uncovered lines (existing)
- Run `cargo mutants --in-place` to find untested behaviors (NEW)
- Add trycmd snapshot tests for CLI output stability (NEW)
- Add targeted assert_cmd tests for uncovered error paths (existing patterns)

**If preparing the release:**
- Run `cargo-semver-checks check-release` to verify no unintended breaking changes (NEW)
- Run full `just ci` suite (existing)
- Run `cargo mutants` on full codebase for final quality gate (NEW)
- Verify changie unreleased entries are complete (existing)
- Merge release-plz PR which creates tag -> cargo-dist builds binaries (existing)

## Version Compatibility

| Package A | Compatible With | Notes |
|-----------|-----------------|-------|
| cargo-semver-checks 0.44+ | Rust 1.90+ | MSRV matches project exactly. Uses rustdoc JSON which requires matching Rust toolchain. |
| cargo-mutants 26.2+ | Rust stable (any) | No compiler plugin requirements. Works with cargo test or nextest. |
| trycmd 0.15+ | assert_cmd 2.x | Same ecosystem (assert-rs). Can coexist in test suite. |
| insta 1.46+ | Rust 1.60+ | Well below project's 1.90 MSRV. Compatible with cargo-insta CLI tool. |
| cargo-llvm-cov 0.8.4 | llvm-tools-preview component | Requires `rustup component add llvm-tools-preview`. Already in CI setup. |

## Sources

- [cargo-llvm-cov GitHub](https://github.com/taiki-e/cargo-llvm-cov) -- v0.8.4, released 2026-02-06. HIGH confidence.
- [cargo-semver-checks GitHub](https://github.com/obi1kenobi/cargo-semver-checks) -- v0.44.0, 245 lints. HIGH confidence.
- [cargo-semver-checks 2025 Year in Review](https://predr.ag/blog/cargo-semver-checks-2025-year-in-review/) -- Lint count doubled in 2025. HIGH confidence.
- [cargo-mutants docs](https://mutants.rs/) -- v26.2.0, released 2026-02-01. HIGH confidence.
- [cargo-nextest docs](https://nexte.st/) -- v0.9.116+. HIGH confidence.
- [cargo-audit crates.io](https://crates.io/crates/cargo-audit) -- v0.22.1, released 2026-02-04. HIGH confidence.
- [cargo-deny GitHub](https://github.com/EmbarkStudios/cargo-deny) -- v0.19.0, released 2026-01-08. HIGH confidence.
- [release-plz docs](https://release-plz.dev/) -- v0.3.156. MEDIUM confidence (version from docs.rs listing).
- [cargo-dist GitHub](https://github.com/axodotdev/cargo-dist) -- v0.30.3 (stable), v0.30.4-pre. HIGH confidence.
- [changie docs](https://changie.dev/) -- v1.22.1. HIGH confidence.
- [trycmd docs](https://docs.rs/trycmd/latest/trycmd/) -- assert-rs ecosystem. HIGH confidence.
- [insta docs](https://insta.rs/) -- v1.46.3. HIGH confidence.
- [Rust CLI Book -- Testing](https://rust-cli.github.io/book/tutorial/testing.html) -- Official testing guide. HIGH confidence.
- [alexwlchan on assert_cmd (2025)](https://alexwlchan.net/2025/testing-rust-cli-apps-with-assert-cmd/) -- Practitioner perspective. MEDIUM confidence.
- Context7 library resolution for cargo-llvm-cov, cargo-dist, release-plz -- Used for version verification. HIGH confidence.

---
*Stack research for: Rust CLI 1.0 stabilization and release preparation*
*Researched: 2026-02-27*
