# Requirements: Repoverlay 1.0 Stabilization

**Defined:** 2026-02-27
**Core Value:** Every feature that ships in 1.0 must work correctly and be verified

## v1 Requirements

Requirements for 1.0 release. Each maps to roadmap phases.

### Code Review

- [ ] **REVIEW-01**: All leaf modules reviewed for correctness (overlay_name, fuzzy, json_merge, github, upstream, reference)
- [ ] **REVIEW-02**: All infrastructure modules reviewed for correctness (state, config, cache)
- [ ] **REVIEW-03**: Source resolution modules reviewed for correctness (sources, overlay_repo)
- [ ] **REVIEW-04**: Support modules reviewed for correctness (detection, selection)
- [ ] **REVIEW-05**: Core operations reviewed for correctness (lib.rs — apply, remove, status, restore, update, create, switch)
- [ ] **REVIEW-06**: CLI dispatch reviewed for correctness (cli.rs — all subcommands)
- [ ] **REVIEW-07**: SourceResolver trait implementation verified as complete across all code paths

### Bug Fixes

- [ ] **FIX-01**: All bugs discovered during code review are fixed
- [ ] **FIX-02**: Error display switched from Debug to Display format (`{e:?}` → `{e:#}` in main.rs)
- [ ] **FIX-03**: SIGPIPE handling added for clean pipe behavior in scripts
- [ ] **FIX-04**: Issues #142-#148 verified as fully resolved

### Test Coverage

- [ ] **TEST-01**: Coverage gaps closed for path traversal edge cases
- [ ] **TEST-02**: Coverage gaps closed for cache update failure recovery
- [ ] **TEST-03**: Coverage gaps closed for terminal recovery (Ctrl+C in interactive selection)
- [ ] **TEST-04**: Regression tests added for every bug fixed during review
- [ ] **TEST-05**: Mutation testing run with cargo-mutants to identify untested behaviors
- [ ] **TEST-06**: Surviving mutants from TEST-05 addressed with additional tests

### API Stabilization

- [ ] **API-01**: Public API surface reviewed (pub vs pub(crate) boundaries)
- [ ] **API-02**: `#[non_exhaustive]` added to extensible public enums and structs
- [ ] **API-03**: Public API documented with appropriate doc comments

### Manual Test Suite

- [ ] **MANUAL-01**: CLI walkthrough test cases for apply command (local + GitHub sources)
- [ ] **MANUAL-02**: CLI walkthrough test cases for remove command
- [ ] **MANUAL-03**: CLI walkthrough test cases for status command
- [ ] **MANUAL-04**: CLI walkthrough test cases for restore command
- [ ] **MANUAL-05**: CLI walkthrough test cases for update command
- [ ] **MANUAL-06**: CLI walkthrough test cases for create command
- [ ] **MANUAL-07**: CLI walkthrough test cases for switch/browse commands
- [ ] **MANUAL-08**: CLI walkthrough test cases for source management commands (add, list, remove, sync)

### Release Preparation

- [ ] **REL-01**: README reviewed and accurate for 1.0
- [ ] **REL-02**: crates.io metadata verified (description, categories, keywords, license)

## v2 Requirements

Deferred to future release. Tracked but not in current roadmap.

### Test Infrastructure

- **TEST-V2-01**: Cross-platform CI runners (macOS + Windows) added to test matrix
- **TEST-V2-02**: trycmd snapshot tests for CLI output stability
- **TEST-V2-03**: Property-based testing for state serialization round-trips

### Release

- **REL-V2-01**: Complete 1.0 changelog via changie
- **REL-V2-02**: cargo-semver-checks run before release tag
- **REL-V2-03**: Man page generation via clap_mangen

### Quality

- **QUAL-V2-01**: Distinct exit codes for different error categories
- **QUAL-V2-02**: Real-world workflow scenario manual tests
- **QUAL-V2-03**: Error handling manual test scenarios
- **QUAL-V2-04**: Cross-platform manual test scenarios

## Out of Scope

| Feature | Reason |
|---------|--------|
| New commands or flags | Feature freeze for stabilization |
| Refactoring large functions (apply_overlay_internal) | Risk of regressions during stabilization; document tech debt instead |
| Switching state format from CCL | Complex migration logic, huge risk for stabilization |
| Performance optimization | Not needed at this scale; optimize based on real reports post-1.0 |
| Adding tracing/structured logging | Touches every module; current logging sufficient |
| Windows symlink junction fallback | --copy mode works; improve error message instead |
| Cache eviction policies | Manual cleanup exists; defer automatic eviction |
| Documentation site | Above 1.0 requirements for tool of this scope |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| REVIEW-01 | Phase 1 | Pending |
| REVIEW-02 | Phase 1 | Pending |
| REVIEW-03 | Phase 1 | Pending |
| REVIEW-04 | Phase 1 | Pending |
| REVIEW-05 | Phase 1 | Pending |
| REVIEW-06 | Phase 1 | Pending |
| REVIEW-07 | Phase 1 | Pending |
| FIX-01 | Phase 2 | Pending |
| FIX-02 | Phase 2 | Pending |
| FIX-03 | Phase 2 | Pending |
| FIX-04 | Phase 2 | Pending |
| TEST-01 | Phase 3 | Pending |
| TEST-02 | Phase 3 | Pending |
| TEST-03 | Phase 3 | Pending |
| TEST-04 | Phase 3 | Pending |
| TEST-05 | Phase 3 | Pending |
| TEST-06 | Phase 3 | Pending |
| API-01 | Phase 4 | Pending |
| API-02 | Phase 4 | Pending |
| API-03 | Phase 4 | Pending |
| MANUAL-01 | Phase 4 | Pending |
| MANUAL-02 | Phase 4 | Pending |
| MANUAL-03 | Phase 4 | Pending |
| MANUAL-04 | Phase 4 | Pending |
| MANUAL-05 | Phase 4 | Pending |
| MANUAL-06 | Phase 4 | Pending |
| MANUAL-07 | Phase 4 | Pending |
| MANUAL-08 | Phase 4 | Pending |
| REL-01 | Phase 5 | Pending |
| REL-02 | Phase 5 | Pending |

**Coverage:**
- v1 requirements: 28 total
- Mapped to phases: 28
- Unmapped: 0 ✓

---
*Requirements defined: 2026-02-27*
*Last updated: 2026-02-27 after initial definition*
