# Feature Research

**Domain:** Rust CLI tool 1.0 stabilization (repoverlay)
**Researched:** 2026-02-27
**Confidence:** HIGH

## Context

repoverlay is a feature-complete Rust CLI tool (currently v0.8.0) being prepared for a 1.0 release. The tool overlays config files into git repositories via symlinks/copies. This research examines what a quality 1.0 release needs *beyond* new features -- the stabilization, quality, and release-readiness concerns that separate a credible 1.0 from a premature version bump.

## Feature Landscape

### Table Stakes (Users Expect These)

Features/qualities users assume exist in a 1.0 release. Missing these = product feels incomplete or unreliable.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| **All documented commands work correctly** | 1.0 signals production-ready; broken commands erode trust immediately | MEDIUM | Code review of all command paths, edge case testing. PROJECT.md confirms this is the core goal. |
| **Helpful error messages** | Users expect actionable errors, not stack traces or cryptic messages | MEDIUM | Current approach uses `anyhow` with `.context()`, which is correct for CLI apps. Review all error paths for user-facing clarity. Errors should suggest recovery actions where possible. |
| **Consistent CLI behavior** | Commands should behave predictably: flags work the same across subcommands, output format is stable | LOW | Already uses clap derive with good structure. Verify flag consistency across apply/restore/update/switch. |
| **Shell completions ship correctly** | Users of clap-based tools expect completions to work | LOW | Already has `clap_complete` via `completions` subcommand. Verify completions are accurate for all current subcommands and flags. |
| **Clean install/uninstall** | Homebrew, cargo install, shell installer all work; no leftover state on uninstall | LOW | cargo-dist already configured with shell/powershell/homebrew installers. Verify installation paths work end-to-end. |
| **No silent data loss** | Overlay apply/remove must never silently delete user files or corrupt state | HIGH | Most critical for 1.0 trust. Path traversal validation exists but has edge cases per CONCERNS.md. Conflict strategies must be airtight. |
| **Cross-platform basics work** | Linux and macOS (primary targets) work correctly; Windows works with documented limitations | MEDIUM | Windows symlink elevation documented as a concern. At minimum, clear error messages when symlinks fail on Windows. |
| **Documented configuration** | Users can find how to configure the tool | LOW | README covers `repoverlay.ccl` config. Consider `--help` output completeness for all config options. |
| **Semantic versioning commitment** | After 1.0, API changes follow semver strictly. Public Rust API (lib.rs) must be stable. | MEDIUM | cargo-semver-checks already runs in PR CI. Review public API surface -- consider what should be `pub` vs `pub(crate)`. Use `#[non_exhaustive]` on public enums. |
| **Changelog for 1.0** | Users need to know what changed and what's stable | LOW | changie already configured with component-based changelog. Compile full 1.0 changelog covering the release. |

### Differentiators (Above-Average Quality Practices)

Practices that go beyond table stakes. Not expected by all users, but signal professional quality.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **Comprehensive test coverage with documented gaps** | Demonstrates engineering rigor; catches regressions before users do | HIGH | Current integration tests are strong (~1844 lines). Gaps identified in CONCERNS.md: Windows symlink failures, path traversal edge cases, Ctrl+C terminal recovery, cache update failure recovery. Closing these gaps is the core stabilization work. |
| **Manual test suite (CLI walkthroughs)** | Catches UX issues automated tests miss: output formatting, interactive prompts, real-world workflows | MEDIUM | PROJECT.md lists this as an active requirement. Create documented test scenarios for: local overlay apply/remove cycle, GitHub overlay with caching, overlay repo resolution, create/add/sync workflow, restore after git clean. |
| **Semver-checked public API** | Prevents accidental breaking changes post-1.0 | LOW | Already running cargo-semver-checks in PR CI. Already differentiated. Ensure `#[non_exhaustive]` on public enums/structs that may grow. |
| **Binary size tracking** | Prevents accidental bloat from dependency additions | LOW | Already implemented in CI with PR comments for significant changes. Already differentiated. |
| **Security audit pipeline** | Catches vulnerable dependencies before they reach users | LOW | Already running daily cargo-audit + cargo-deny. Already differentiated. |
| **Structured error output (JSON mode)** | Enables scripting and tooling integration | MEDIUM | `status --json` already exists. Could extend to other commands, but not required for 1.0. Defer. |
| **Man page generation** | Professional CLI tools ship man pages | MEDIUM | Not currently implemented. Would use `clap_mangen` in build.rs. Nice to have but not blocking for 1.0 -- README and `--help` output are sufficient for a tool at this scale. |
| **Property-based testing for state serialization** | Catches edge cases in CCL state file round-tripping that unit tests miss | MEDIUM | State file format (CCL) is custom and identified as fragile in CONCERNS.md. Property tests would verify round-trip correctness. Good investment but not blocking. |
| **Deterministic/reproducible builds** | Users can verify published binaries match source | LOW | release profile already has `strip = true`, `lto = true`, `codegen-units = 1`. cargo-dist handles reproducible build matrix. |
| **Documentation site** | Central reference beyond README | HIGH | `website/` directory exists but unclear state. A full docs site is above 1.0 requirements for a tool of this scope. |

### Anti-Features (Things to Deliberately NOT Do for 1.0 Stabilization)

Things that seem good but would derail a stabilization milestone.

| Anti-Feature | Why Requested | Why Problematic | Alternative |
|--------------|---------------|-----------------|-------------|
| **New commands or flags** | Feature ideas always come up during review | Feature freeze is essential for stabilization. Every new feature needs testing, documentation, and introduces risk. Scope creep is the #1 killer of stabilization milestones. | Write down ideas in a backlog. Ship 1.0, then add features in 1.1+. |
| **Large refactors (e.g., splitting apply_overlay_internal)** | CONCERNS.md notes the ~350-line function is complex | Refactoring introduces regressions. The function works; tests cover it. Stabilization means verifying correctness, not rewriting. | Document the tech debt. Add tests that lock current behavior. Refactor in 1.1+ with those tests as a safety net. |
| **Performance optimization** | Tempting to optimize during review | Performance work changes behavior and requires benchmarking. Current performance is fine for the tool's scale (config file overlays, not GB-scale data). | Profile post-1.0. Optimize based on real user reports, not speculation. |
| **Switching state format from CCL to JSON/TOML** | CCL is non-standard and identified as fragile | Migration logic is complex, introduces new failure modes, and must handle all existing state files. Huge risk for stabilization. | Document CCL format. Ensure CCL parsing is well-tested. Plan migration for 2.0 if needed. |
| **Adding tracing/structured logging** | Current `log` + `env_logger` is basic | Swapping logging frameworks touches every module. Structured logging is nice but `RUST_LOG=debug` already works for diagnostics. | Keep current logging. Consider tracing in 1.1+ if diagnostics prove insufficient. |
| **Windows symlink fallback to junction points** | Would improve Windows experience | Complex platform-specific code with its own edge cases. Current copy mode (`--copy`) already works on Windows. | Document that `--copy` is the recommended mode on Windows. Improve error message for symlink failures. |
| **Dual-format state files (CCL + JSON)** | Would future-proof against CCL issues | Doubles the serialization surface area and introduces consistency concerns between formats. | Test CCL round-tripping thoroughly. Plan migration path but don't implement it now. |
| **Cache eviction policies** | Cache can grow unbounded | Adds complexity to cache management. `cache remove --all` already exists for manual cleanup. | Document `cache remove` in help text. Add size reporting to `cache list`. Plan automatic eviction for 1.1+. |

## Feature Dependencies

```
[Comprehensive code review]
    └──produces──> [Bug fixes discovered during review]
                       └──requires──> [Test additions for fixed bugs]

[Test coverage improvements]
    └──requires──> [Understanding of current coverage gaps]
    └──enables──> [Confidence in 1.0 release]

[Public API stabilization]
    └──requires──> [Review of pub vs pub(crate) boundaries]
    └──enables──> [Semver commitment post-1.0]
    └──requires──> [#[non_exhaustive] on extensible types]

[Manual test suite]
    └──requires──> [All bug fixes landed]
    └──produces──> [Release confidence]

[Changelog compilation]
    └──requires──> [All bug fixes and test additions complete]
    └──produces──> [1.0 release documentation]

[Error message review]
    └──enhances──> [Cross-platform support]
    └──enhances──> [No silent data loss]
```

### Dependency Notes

- **Code review produces bug fixes:** The review phase will surface correctness issues that must be fixed before testing.
- **Test additions require bug fixes first:** Tests should verify the *fixed* behavior, not lock in bugs.
- **Manual test suite requires all fixes landed:** Manual walkthroughs validate the final state, not an intermediate state.
- **Public API stabilization requires visibility review:** Before committing to semver, ensure the public API surface is intentional.
- **Changelog requires all work complete:** The 1.0 changelog summarizes everything that shipped.

## MVP Definition (1.0 Release)

### Must Ship With (1.0)

- [x] All documented commands work correctly (verified by code review + tests)
- [ ] All bugs discovered during review are fixed
- [ ] Test coverage for identified gaps (Windows error paths, path traversal, terminal recovery, cache failure recovery)
- [ ] Error messages are user-friendly across all failure modes
- [ ] Public API surface reviewed and locked (`#[non_exhaustive]` where appropriate)
- [ ] Manual test suite documenting real-world scenarios
- [ ] Complete 1.0 changelog
- [ ] README reviewed and up to date for 1.0

### Add After 1.0 (1.x)

- [ ] Man page generation via `clap_mangen` -- trigger: user requests or packaging needs
- [ ] Property-based testing for state serialization -- trigger: state corruption reports
- [ ] Structured JSON output for more commands -- trigger: scripting/automation use cases
- [ ] Cache size reporting and automatic eviction -- trigger: users with many sources
- [ ] Refactor `apply_overlay_internal` into smaller functions -- trigger: when adding new conflict strategies or merge modes

### Future Consideration (2.0+)

- [ ] State format migration from CCL -- trigger: sickle crate becoming unmaintained or format limitations blocking features
- [ ] Plugin system for custom source resolvers -- trigger: demand for non-GitHub/non-local sources
- [ ] Daemon mode for watching overlay changes -- trigger: if users want live-reload workflows

## Feature Prioritization Matrix

| Feature | User Value | Implementation Cost | Priority |
|---------|------------|---------------------|----------|
| Fix discovered bugs | HIGH | MEDIUM | P1 |
| Test coverage for identified gaps | HIGH | HIGH | P1 |
| Error message review/improvement | HIGH | LOW | P1 |
| Public API stabilization | HIGH | LOW | P1 |
| Manual test suite | MEDIUM | MEDIUM | P1 |
| 1.0 changelog | MEDIUM | LOW | P1 |
| README review | MEDIUM | LOW | P1 |
| Man page generation | LOW | MEDIUM | P2 |
| Property-based testing | MEDIUM | MEDIUM | P2 |
| JSON output for more commands | LOW | MEDIUM | P3 |
| Refactor large functions | MEDIUM | HIGH | P3 |

**Priority key:**
- P1: Must have for 1.0 launch
- P2: Should have, add in 1.x when possible
- P3: Nice to have, future consideration

## Comparable Rust CLI Tools Analysis

Examining successful Rust CLI 1.0+ releases for quality patterns:

| Quality Aspect | fd (sharkdp) | ripgrep (BurntSushi) | repoverlay (current state) |
|----------------|-------------|---------------------|---------------------------|
| Clippy lints | Standard | pedantic | pedantic + nursery (strong) |
| Test coverage | Integration + unit | Extensive integration | Integration + unit (good, gaps identified) |
| Shell completions | Build-time gen | Build-time gen | Runtime via subcommand (adequate) |
| Man pages | Generated | Generated | Not yet (P2) |
| Changelog | GitHub releases | CHANGELOG.md | changie-managed CHANGELOG.md (strong) |
| Binary size tracking | No | No | CI with PR comments (strong) |
| Semver checks | No | No | cargo-semver-checks in CI (strong) |
| Security audits | No | cargo-audit | cargo-audit + cargo-deny daily (strong) |
| Cross-platform CI | Linux + macOS + Windows | All major | Linux only in CI (gap) |

**Key observation:** repoverlay already has several differentiating quality practices (semver checks, binary size tracking, security audits) that many established Rust CLI tools lack. The primary gap is test coverage breadth and cross-platform CI verification.

## Sources

- [PROJECT.md](../.planning/PROJECT.md) - Project context and requirements (HIGH confidence)
- [TESTING.md](../.planning/codebase/TESTING.md) - Current test patterns (HIGH confidence)
- [CONCERNS.md](../.planning/codebase/CONCERNS.md) - Known issues and coverage gaps (HIGH confidence)
- [Rain's Rust CLI Recommendations](https://rust-cli-recommendations.sunshowers.io/) - Authoritative CLI best practices (HIGH confidence)
- [sharkdp's Release Checklist](https://dev.to/sharkdp/my-release-checklist-for-rust-programs-1m33) - Real-world Rust release process from fd/bat/hyperfine author (HIGH confidence)
- [The Rust CLI Book](https://rust-cli.github.io/book/index.html) - Official Rust CLI guidance (HIGH confidence)
- [Cargo SemVer Compatibility Reference](https://doc.rust-lang.org/cargo/reference/semver.html) - Official semver rules for Rust (HIGH confidence)
- [cargo-semver-checks](https://github.com/obi1kenobi/cargo-semver-checks) - Automated semver linting (HIGH confidence)
- [Effective Rust - SemVer Promises](https://effective-rust.com/semver.html) - Guidance on reaching 1.0 (HIGH confidence)
- [Rust Error Handling Best Practices 2025](https://markaicode.com/rust-error-handling-2025-guide/) - Current error handling patterns (MEDIUM confidence)

---
*Feature research for: Rust CLI 1.0 stabilization (repoverlay)*
*Researched: 2026-02-27*
