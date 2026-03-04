# Project Retrospective

*A living document updated after each milestone. Lessons feed forward into future planning.*

## Milestone: v0.8 — Repoverlay 1.0 Stabilization

**Shipped:** 2026-03-04
**Phases:** 4 | **Plans:** 8

### What Was Built
- Full code review of all 15 source modules with zero correctness bugs found
- Bug fixes: Display-format errors, SIGPIPE handling, issues #142-#148 verified
- Test coverage: path traversal, cache failure, interactive conflict, mutation testing
- API surface locked: pub(crate) across 14 modules
- 41 manual test cases across 8 CLI workflows
- README simplified, crates.io metadata finalized

### What Worked
- Compressing 8-phase research structure to 4 phases kept momentum high
- Review-then-test ordering: Phase 1 found zero bugs, Phase 2 confirmed via mutation testing
- stdin injection pattern for testing interactive prompts without TTY
- Unconditional .failure() assertions for path traversal tests (fail-by-default is clearer)
- Quick depth setting appropriate for stabilization work on mature codebase

### What Was Inefficient
- Phase 1 lacked VERIFICATION.md (workflow wasn't established yet), causing audit to flag 11 "partial" requirements
- Audit ran before Phase 4 existed, creating stale gap report
- TEST-05/TEST-06 checkbox staleness required manual fixup

### Patterns Established
- pub(crate) as default visibility for binary-only Rust crates
- Manual test documents with mktemp -d isolation and embedded expected output
- Equivalent mutant documentation alongside mutation testing results
- clippy::redundant_pub_crate lint allowance for explicit visibility

### Key Lessons
1. Run milestone audit after all phases complete, not before — avoids stale gap reports
2. VERIFICATION.md should be part of phase execution from the start, not retroactively
3. Mutation testing with scoped file targets is practical; full-crate runs are too expensive
4. Zero correctness bugs in review validates that the codebase was already well-implemented

### Cost Observations
- Sessions: ~4
- Notable: Average plan execution ~7min; mutation testing (02-03) was the outlier at 27min

---

## Cross-Milestone Trends

### Process Evolution

| Milestone | Phases | Plans | Key Change |
|-----------|--------|-------|------------|
| v0.8 | 4 | 8 | First milestone with GSD workflow |

### Cumulative Quality

| Milestone | Unit Tests | Integration Tests | Mutation Testing |
|-----------|-----------|-------------------|-----------------|
| v0.8 | 925 | 97 | Baseline established |

### Top Lessons (Verified Across Milestones)

1. (First milestone — lessons to be validated in future milestones)
