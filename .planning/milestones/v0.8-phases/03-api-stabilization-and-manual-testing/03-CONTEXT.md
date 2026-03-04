# Phase 3: API Stabilization and Manual Testing - Context

**Gathered:** 2026-03-03
**Status:** Ready for planning

<domain>
## Phase Boundary

Lock public API surface for semver guarantees and create manual test documentation for all 8 CLI workflows. This phase ensures the crate has clean internal boundaries (pub(crate) everywhere) and that every command has a human-followable test document.

</domain>

<decisions>
## Implementation Decisions

### API boundary scope
- Binary-only for 1.0 — no external library consumers
- Convert all `pub` items to `pub(crate)` across the entire codebase
- No external API commitment — library API deferred to future version
- Manual review of pub items (grep + audit), no semver-checks tooling
- Structure internals so a library API could be added later, but don't design it now

### Non-exhaustive strategy
- Skip `#[non_exhaustive]` entirely for 1.0
- No external consumers means it has no effect
- Add when library API is designed in a future version

### Doc comment depth
- Module-level `//!` docs only — no per-item documentation
- Freeform descriptions explaining module purpose and role
- No rustdoc examples or usage notes for 1.0

### Manual test format
- Hybrid markdown with embedded copy-pasteable command blocks and expected output sections
- One file per CLI command (8 files: apply.md, remove.md, status.md, restore.md, update.md, create.md, switch-browse.md, source-management.md)
- Location: `docs/manual-tests/` in the repository root
- Local overlays primary (no network required), GitHub-specific tests marked as optional/requires-network sections

### Claude's Discretion
- testutil module gating approach (cfg(test) vs pub(crate))
- Module doc format per module (short for simple modules, more detail for complex ones)
- Manual test document structure within each file (section ordering, setup instructions)

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches

</specifics>

<code_context>
## Existing Code Insights

### Reusable Assets
- Only `pub fn run()` currently exported from lib.rs — most items are already module-internal
- `src/testutil.rs` has pub items used by integration tests — needs special handling

### Established Patterns
- 17 source files in src/ with clear module separation
- CLI layer in cli.rs with clap derive structs
- Application logic centralized in lib.rs
- State management uses CCL format via sickle crate

### Integration Points
- `src/main.rs` → `lib::run()` is the only cross-module public boundary
- Integration tests in `tests/` directory consume testutil items
- 8 CLI commands: apply, remove, status, restore, update, create, switch/browse, source management (add/list/remove/sync)

</code_context>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 03-api-stabilization-and-manual-testing*
*Context gathered: 2026-03-03*
