# Roadmap: Repoverlay 1.0 Stabilization

## Overview

Repoverlay is feature-complete at v0.8.0. This roadmap delivers a credible 1.0 release through four phases: a full code review with bug fixes, test coverage hardening, verification of API boundaries and real-world workflows, and final release preparation. Every phase depends on the previous one -- review informs fixes, fixes must land before tests lock in behavior, and everything must be stable before the release tag.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Code Review and Bug Fixes** - Review all modules for correctness and fix every bug found
- [ ] **Phase 2: Test Coverage** - Close coverage gaps and verify untested behaviors via mutation testing
- [ ] **Phase 3: API Stabilization and Manual Testing** - Lock public API surface and create manual test suite for all CLI workflows
- [ ] **Phase 4: Release Preparation** - Final release gate: README, crates.io metadata, and release artifacts

## Phase Details

### Phase 1: Code Review and Bug Fixes
**Goal**: Every module has been reviewed for correctness and all discovered bugs are fixed
**Depends on**: Nothing (first phase)
**Requirements**: REVIEW-01, REVIEW-02, REVIEW-03, REVIEW-04, REVIEW-05, REVIEW-06, REVIEW-07, FIX-01, FIX-02, FIX-03, FIX-04
**Success Criteria** (what must be TRUE):
  1. All 15 source modules have been reviewed and any correctness issues are documented or fixed
  2. Error output uses Display format (`{e:#}`) -- no Debug-format error messages reach users
  3. Piping repoverlay output to other commands does not produce broken pipe errors
  4. Issues #142-#148 are verified resolved with passing tests
  5. SourceResolver trait implementation is verified complete across all code paths
**Plans**: 2 plans

Plans:
- [x] 01-01-PLAN.md -- Review all 13 non-orchestration modules (leaf, infrastructure, resolution/support) and verify SourceResolver completeness
- [x] 01-02-PLAN.md -- Review lib.rs and cli.rs orchestration modules, apply bug fixes (error display, SIGPIPE, issue verification)

### Phase 2: Test Coverage
**Goal**: Test suite catches regressions for all fixed bugs and covers identified gap areas
**Depends on**: Phase 1
**Requirements**: TEST-01, TEST-02, TEST-03, TEST-04, TEST-05, TEST-06
**Success Criteria** (what must be TRUE):
  1. Path traversal edge cases have dedicated tests that fail on unsafe behavior
  2. Cache update failure scenarios have tests verifying graceful recovery
  3. Every bug fixed in Phase 1 has a regression test
  4. Mutation testing has been run and surviving mutants have been addressed or documented
**Plans**: 3 plans

Plans:
- [ ] 02-01-PLAN.md -- Path traversal edge case tests (unconditional failure assertions) and FIX-02 regression test (Display format)
- [ ] 02-02-PLAN.md -- Cache failure recovery unit tests and interactive conflict stdin injection test (TEST-03 scope)
- [ ] 02-03-PLAN.md -- Install cargo-mutants, run scoped mutation baseline, address surviving mutants

### Phase 3: API Stabilization and Manual Testing
**Goal**: Public API is locked for semver and all CLI workflows are verified through manual test documentation
**Depends on**: Phase 2
**Requirements**: API-01, API-02, API-03, MANUAL-01, MANUAL-02, MANUAL-03, MANUAL-04, MANUAL-05, MANUAL-06, MANUAL-07, MANUAL-08
**Success Criteria** (what must be TRUE):
  1. Public API boundaries are explicit -- every pub item is intentionally public or has been changed to pub(crate)
  2. Extensible public enums and structs have `#[non_exhaustive]` applied
  3. Public API items have doc comments
  4. Manual test documents exist for all 8 core CLI workflows (apply, remove, status, restore, update, create, switch/browse, source management)
  5. Manual test steps can be followed by a person and produce the documented results
**Plans**: TBD

Plans:
- [ ] 03-01: TBD
- [ ] 03-02: TBD

### Phase 4: Release Preparation
**Goal**: 1.0 release artifacts are verified and ready to publish
**Depends on**: Phase 3
**Requirements**: REL-01, REL-02
**Success Criteria** (what must be TRUE):
  1. README accurately describes all commands and features shipping in 1.0
  2. crates.io metadata (description, categories, keywords, license) is complete and correct
**Plans**: TBD

Plans:
- [ ] 04-01: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Code Review and Bug Fixes | 2/2 | Complete | 2026-02-28 |
| 2. Test Coverage | 0/1 | Not started | - |
| 3. API Stabilization and Manual Testing | 0/2 | Not started | - |
| 4. Release Preparation | 0/1 | Not started | - |
