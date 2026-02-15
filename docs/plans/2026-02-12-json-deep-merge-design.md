# JSON Deep Merge for Overlay Application

**Issue:** #88
**Date:** 2026-02-12
**Status:** Approved

## Overview

Add a `--merge` flag (and `REPOVERLAY_MERGE` env var) that deep merges `.json` files instead of treating them as conflicts during overlay application. Works for both cross-overlay conflicts (two overlays contributing the same `.json` file) and repo-file conflicts (overlay `.json` conflicts with an existing repo file). Combinable with `--force`/`--skip-conflicts` for non-JSON conflicts.

## Merge Semantics

- **Objects:** Recursively deep merge. Overlay keys override base keys at each level.
- **Arrays:** Overlay replaces base entirely (arrays are atomic).
- **Type mismatches:** Overlay wins. Log a warning with full detail (key path, base type, overlay type).
- **Scalars:** Overlay wins.
- **Result:** Written as a plain regular file (not a symlink), even in symlink mode.

## Activation

1. **`--merge` CLI flag** on `apply`, `update`, `restore`, `switch` commands
2. **`REPOVERLAY_MERGE=1` or `REPOVERLAY_MERGE=true` env var** — implies `--merge` when set
3. CLI flag is boolean (no value). Env var checked at flag resolution time: `cli_flag || env_var_is_truthy`.

## Integration into Apply Flow (Pre-Processing Step)

At each conflict detection point in `apply_resolved_overlay()`:

### Cross-overlay conflict (file managed by another overlay)

- **Current:** Always fails, even with `--force`.
- **With `--merge`:** If both files are `.json`, read the existing target file and overlay source, deep merge (base = existing target, overlay = new source), write merged result. Log all merge details.
- **Not `.json`:** Existing behavior (always fail).

### Repo-file conflict (file exists in repo, not from overlay)

- **Current:** Fail/force/skip based on `ConflictStrategy`.
- **With `--merge`:** If `.json`, read existing repo file and overlay source, deep merge, write result. Log details.
- **Not `.json`:** Fall through to existing `ConflictStrategy`.

### No conflict

Normal behavior (symlink or copy), unaffected by `--merge`.

## State Tracking

New `LinkType` variant:

```rust
pub(crate) enum LinkType {
    Symlink,
    Copy,
    Merged, // deep merged from multiple sources
}
```

This lets `remove`, `update`, and `restore` handle merged files correctly (delete the materialized file rather than unlinking a symlink).

## Logging

When `--merge` is active, log verbosely:

- Each JSON file being merged (source paths)
- Keys added, overridden, and type mismatches (with full dotted key paths)
- Summary per file (e.g., "Merged settings.json: 3 keys added, 2 overridden, 1 type mismatch")

## Dependency

Add `serde_json` crate for JSON parsing and serialization.

## Out of Scope (Phase 1)

- YAML/TOML merge support (future Phase 2)
- Configurable array merge strategies
- Per-file merge config in `repoverlay.ccl`
- Auto-merge without flag (future, once battle-tested)
