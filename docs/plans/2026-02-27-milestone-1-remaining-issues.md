# Milestone 1.0 — Remaining Issues Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the 4 remaining open issues in the 1.0 milestone: #79 (remove legacy config), #144 (create auto-applies), #148 (interactive edit atomicity), #84 (command vocabulary audit).

**Architecture:** Each issue is an independent workstream. #79 is a pure removal task. #144 adds an apply step to create. #148 fixes atomicity in interactive edit. #84 is a documentation/design task. All changes are in `src/cli.rs`, `src/lib.rs`, and `src/config.rs`.

**Tech Stack:** Rust 2024 edition, clap (CLI), anyhow (errors), tempfile (tests), CCL config format.

---

## Workstream A: Remove legacy `overlay_repo` config (#79)

**Priority:** High — labeled "good first issue", pure cleanup, no new behavior.

### Task A1: Remove `OverlayRepoConfig` struct and config field

**Files:**
- Modify: `src/config.rs:14-25` (RepoverlayConfig struct)
- Modify: `src/config.rs:266-276` (OverlayRepoConfig struct)

**Step 1: Remove the `overlay_repo` field from `RepoverlayConfig`**

In `src/config.rs`, remove the `overlay_repo` field from the struct:

```rust
// REMOVE this field from RepoverlayConfig:
pub overlay_repo: Option<OverlayRepoConfig>,
```

**Step 2: Remove the `OverlayRepoConfig` struct**

Remove the entire `OverlayRepoConfig` struct definition (~lines 266-276).

**Step 3: Remove helper methods that fall back to legacy field**

Remove or simplify these methods on `RepoverlayConfig`:
- `get_default_overlay_repo_config()` (~lines 33-52) — remove the fallback to `self.overlay_repo`
- `get_overlay_repo_config_by_name()` (~lines 58-90) — remove the fallback to legacy format

**Step 4: Compile to find remaining references**

Run: `cargo build 2>&1`
Expected: Compilation errors showing all remaining references to `overlay_repo` and `OverlayRepoConfig`.

### Task A2: Remove migration functions

**Files:**
- Modify: `src/config.rs:278-305` (migration functions)
- Modify: `src/config.rs` (load_config auto-migration call)

**Step 1: Remove `needs_migration()` and `migrate_config()`**

Delete both functions entirely (~lines 278-305).

**Step 2: Remove auto-migration call in `load_config()`**

In `load_config()` (~line 355), remove the block that calls `migrate_config()`.

**Step 3: Compile and fix**

Run: `cargo build 2>&1`
Fix any remaining references.

### Task A3: Remove legacy fallback in source resolution

**Files:**
- Modify: `src/lib.rs:780-874` (`resolve_three_part`)
- Modify: `src/config.rs:398-434` (`generate_sources_config_ccl`)

**Step 1: Remove legacy fallback path in `resolve_three_part()`**

In `src/lib.rs`, remove the fallback block at ~lines 813-821 that uses `config.overlay_repo` when sources are empty.

**Step 2: Remove legacy section in `generate_sources_config_ccl()`**

In `src/config.rs`, remove the legacy `overlay_repo` serialization block at ~lines 419-431.

**Step 3: Compile and fix all remaining errors**

Run: `cargo build 2>&1`
Fix any remaining compilation errors from removed types.

### Task A4: Update/remove tests

**Files:**
- Modify: `src/config.rs` (test module, ~lines 740+)
- Modify: any test files referencing migration

**Step 1: Remove migration-related tests**

Search for and remove tests that specifically test `needs_migration()`, `migrate_config()`, or `OverlayRepoConfig`.

Run: `rg "needs_migration|migrate_config|OverlayRepoConfig|overlay_repo" src/ tests/`

Remove or update each test.

**Step 2: Run all tests**

Run: `just test`
Expected: All tests pass with no references to legacy config.

**Step 3: Run lints**

Run: `just check`
Expected: Clean — no warnings, no failures.

**Step 4: Commit**

```bash
git add -A
git commit -m "feat!: remove legacy overlay_repo config support

Remove deprecated overlay_repo field, OverlayRepoConfig struct,
migration functions, and legacy fallback paths. Users must use
the sources config format.

Closes #79"
```

---

## Workstream B: Create command auto-applies overlay (#144)

**Priority:** High — user-facing workflow gap.

### Task B1: Understand the current create flow

**Files:**
- Read: `src/cli.rs:1689-1816` (`create_overlay_command`)
- Read: `src/lib.rs:2728-3003` (`create_overlay`)
- Read: `src/lib.rs:3085-3108` (`create_overlay_with_files`)
- Read: `src/lib.rs:1034` (`apply_overlay` signature)

**Step 1: Map the current flow**

Read the create functions to understand exactly what they return and what state exists after they complete.

### Task B2: Write failing test for auto-apply after create

**Files:**
- Create or modify: `tests/cli.rs` (integration test)

**Step 1: Write a test that creates an overlay and expects it to be applied**

The test should:
1. Create a test repo with some files
2. Run `repoverlay create` targeting an overlay repo
3. Assert that the original files are replaced with symlinks
4. Assert that `.repoverlay/overlays/<name>.ccl` state file exists
5. Assert that `.git/info/exclude` contains the overlay section

**Step 2: Run the test to confirm it fails**

Run: `cargo test <test_name> -- --nocapture`
Expected: FAIL — symlinks not created, state file missing.

### Task B3: Add apply step to create_overlay_command

**Files:**
- Modify: `src/cli.rs:1689-1816` (`create_overlay_command`)
- Possibly modify: `src/lib.rs` (`create_overlay`)

**Step 1: Add apply call after successful overlay creation**

After the overlay is created and committed (in the overlay repo path), call `apply_overlay()` with the newly created overlay as the source and the original repo as the target.

The key consideration: `create_overlay_command` knows the `source` (original repo path) which becomes the target for apply, and the output directory which becomes the source for apply.

**Step 2: Handle the `--local` create path similarly**

For `--local` mode, after `create_overlay()` completes, apply the local overlay to the source repo.

**Step 3: Run the test**

Run: `cargo test <test_name> -- --nocapture`
Expected: PASS

**Step 4: Run full test suite**

Run: `just test`
Expected: All tests pass.

**Step 5: Commit**

```bash
git commit -m "feat: auto-apply overlay after create

After creating an overlay (both local and overlay-repo modes),
automatically apply it to the source repository. This replaces
files with symlinks, saves state, and updates git exclude.

Closes #144"
```

---

## Workstream C: Fix interactive edit atomicity (#148)

**Priority:** High — data integrity bug.

### Task C1: Understand the current interactive edit flow

**Files:**
- Read: `src/cli.rs:2236-2421` (`interactive_edit_overlay`)
- Read: `src/cli.rs:2627-2840` (`add_files_to_overlay`)
- Read: `src/state.rs:227-258` (`SourceResolver` trait)

**Step 1: Map the source-type-aware behavior**

Understand how `add_files_to_overlay` works and why it fails for non-OverlayRepo sources. Trace the `SourceResolver` trait to understand `is_mutable()`.

### Task C2: Write failing test for non-OverlayRepo interactive edit

**Files:**
- Modify: `tests/cli.rs` or add unit test in `src/cli.rs`

**Step 1: Write a test that tries interactive edit on a Local overlay**

The test should:
1. Create a local overlay with some files
2. Apply it to a test repo
3. Attempt to add new files via the add-files path
4. Assert that either: it fails cleanly before any mutation, OR it succeeds for local sources

**Step 2: Run to confirm failure**

Run: `cargo test <test_name> -- --nocapture`
Expected: FAIL — half-modified state or unclear error.

### Task C3: Make add_files_to_overlay source-type-aware

**Files:**
- Modify: `src/cli.rs:2627-2840` (`add_files_to_overlay`)
- Modify: `src/cli.rs:2236-2421` (`interactive_edit_overlay`)

**Step 1: Early source-type check in add_files_to_overlay**

Add a check at the beginning of `add_files_to_overlay` that verifies the overlay's source type supports adding files. For local overlays, the add operation should work differently — copying files to the local overlay directory rather than assuming an overlay repo.

**Step 2: Handle each source type appropriately**

- **OverlayRepo**: Existing behavior (copy to overlay repo, commit)
- **Local**: Copy files to the local overlay path, update state
- **GitHub**: Reject with clear error message (read-only source)

**Step 3: Ensure no partial mutations on failure**

Wrap the file operations so that if any step fails, no state is left inconsistent. Consider:
- Validating all files can be added before starting
- Rolling back on failure

**Step 4: Run tests**

Run: `just test`
Expected: All pass, including new test.

**Step 5: Commit**

```bash
git commit -m "fix: make interactive edit source-type-aware

add_files_to_overlay now handles Local and GitHub sources correctly
instead of assuming OverlayRepo. Local overlays copy files to the
overlay directory; GitHub overlays reject with a clear error.
No partial mutations occur on failure.

Closes #148"
```

---

## Workstream D: Rationalize command vocabulary (#84)

**Priority:** Medium — documentation/design, informs 1.0 API surface.

### Task D1: Audit current command vocabulary

**Files:**
- Read: `src/cli.rs:84-522` (Commands enum)
- Read: `src/cli.rs:525-576` (subcommand enums)

**Step 1: Create a vocabulary inventory**

Document every command, its current status (active/hidden/deprecated), what noun it operates on (overlay, source, cache, file), and what verb it represents.

### Task D2: Propose vocabulary rationalization

**Files:**
- Create: `docs/plans/2026-02-27-command-vocabulary-rationalization.md`

**Step 1: Write the proposal document**

The document should:
1. List all current commands with their nouns and verbs
2. Identify ambiguities and redundancies
3. Propose a clean vocabulary for 1.0
4. Note which deprecated commands can be removed
5. Document migration path for removed commands

Key questions to address:
- `remove` should mean "remove an overlay from the target repo" (not files or sources)
- `source add/remove/list` is already clean — keep as-is
- `cache clear/remove/list/path` is already clean — keep as-is
- Remove deprecated hidden commands: `Add`, `Publish`, `List`, `CreateLocal`
- Clarify `sync` vs `update` distinction or merge them

**Step 2: Open this as a discussion/comment on #84**

The vocabulary rationalization is primarily a design decision. The implementation work (removing deprecated commands, renaming) follows from the design.

### Task D3: Remove deprecated hidden commands

**Files:**
- Modify: `src/cli.rs:84-522`

**Step 1: Remove the hidden deprecated commands**

Remove these enum variants and their handlers:
- `Add` (line 429) — deprecated, replaced by `edit --add`
- `Publish` (line 485) — deprecated, replaced by `create`
- `List` (line 389) — hidden, overlapped by `browse`
- `CreateLocal` (line 283) — hidden, subsumed by `create --local`

**Step 2: Run tests**

Run: `just test`
Expected: All pass. If any tests reference removed commands, update them.

**Step 3: Commit**

```bash
git commit -m "feat!: remove deprecated commands (Add, Publish, List, CreateLocal)

Remove hidden/deprecated command variants that have been replaced:
- Add → use 'edit --add'
- Publish → use 'create'
- List → use 'browse'
- CreateLocal → use 'create --local'

Part of #84"
```

---

## Execution Order and Dependencies

```
A1 → A2 → A3 → A4  (Workstream A: legacy config removal)
B1 → B2 → B3        (Workstream B: create auto-apply)
C1 → C2 → C3        (Workstream C: interactive edit fix)
D1 → D2 → D3        (Workstream D: vocabulary rationalization)
```

All four workstreams are independent and can be executed in parallel.

**Recommended assignment:**
- **Workstream A (#79)** — Refactoring specialist: pure code removal, good for systematic cleanup
- **Workstream B (#144)** — Feature developer: requires understanding apply flow and wiring it into create
- **Workstream C (#148)** — Feature developer: requires understanding source types and ensuring atomicity
- **Workstream D (#84)** — Design/documentation: audit + proposal, then mechanical removal of deprecated commands
