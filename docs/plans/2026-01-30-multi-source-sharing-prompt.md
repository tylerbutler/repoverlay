# Implementation Planning Prompt

Use this prompt to start a session for implementing multi-source overlay sharing.

---

## Prompt

```
I want to implement the multi-source overlay sharing feature for repoverlay.

Read the design doc at docs/plans/2026-01-30-multi-source-sharing-design.md

Then create a detailed implementation plan following TDD. The design doc has a
section on "Implementation Approach (TDD)" with 7 testable units in dependency
order - use that as the starting point.

For each unit:
1. What tests to write first
2. What code changes are needed
3. Which files to create/modify

Start with unit 1 (config parsing) and work through in order. The implementation
should be incremental - each unit should be mergeable on its own.
```

---

## Context

- **Design doc**: `docs/plans/2026-01-30-multi-source-sharing-design.md`
- **Key files to understand first**:
  - `src/config.rs` - current config parsing
  - `src/overlay_repo.rs` - current single-source overlay repo
  - `src/lib.rs` - core apply/resolve logic
- **Test approach**: Unit tests with mocks, integration tests with tempdir repos
- **Rust edition**: 2024, clippy pedantic enabled
