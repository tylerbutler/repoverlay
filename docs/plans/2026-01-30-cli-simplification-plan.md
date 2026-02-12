# CLI Simplification Plan

**Date**: 2026-01-30
**Status**: Draft
**Priority**: Medium

## Overview

This document captures opportunities to simplify and improve the repoverlay CLI based on a comprehensive review of commands, flags, and usage patterns.

## High-Priority Issues

### 1. `--target` Semantic Confusion

**Problem**: `--target` means different things in different commands:
- `apply --target /path` = repository directory to apply overlay to
- `list --target org/repo` = filter available overlays by target repository

**Solution**: Rename `list --target` to `list --filter` or `list --repo`

```bash
# Before (confusing)
repoverlay list --target microsoft/FluidFramework

# After (clear)
repoverlay list --filter microsoft/FluidFramework
```

**Files to modify**: `src/cli.rs` (List command definition and handler)

### 2. `create` Command Overload

**Problem**: The `create` command has 5 different execution paths based on argument combinations:
1. `create --local ./dir` (local-only mode)
2. `create name` (auto-detect org/repo)
3. `create org/repo/name` (explicit path)
4. `create name --include file1 file2` (with specific files)
5. `create --local ./dir --include file1` (local with specific files)

**Solution**: Split into two commands:
- `create <name>` - Overlay repo mode only
- `create-local --output <dir>` - Local directory mode

```bash
# Overlay repo mode
repoverlay create my-overlay
repoverlay create microsoft/FluidFramework/claude-config

# Local mode (separate command)
repoverlay create-local --output ./overlays/my-config
```

**Files to modify**: `src/cli.rs` (split Create into Create and CreateLocal)

### 3. `apply` vs `switch` Overlap

**Problem**: `switch` is essentially `remove --all` + `apply`. Having two commands is redundant.

**Solution Options**:
- Option A: Add `--exclusive` or `--replace-all` flag to `apply`
- Option B: Keep `switch` but document clearly it's a convenience wrapper
- Option C: Deprecate `switch` in favor of `apply --exclusive`

```bash
# Option A
repoverlay apply <source> --exclusive  # Removes all other overlays first

# Option B (current)
repoverlay switch <source>  # Keep as convenience alias
```

**Recommendation**: Option B (keep for convenience, document relationship)

## Medium-Priority Issues

### 4. Inconsistent Positional Arguments

**Problem**: Commands handle positional arguments differently:
```
remove [<name>]           # optional (triggers interactive mode)
sync <name>               # required
add <name> <files>...     # required
```

**Solution**: Make patterns consistent:
- Use `--interactive` flag instead of omitting arguments
- Or document the inconsistency clearly

```bash
# Before
repoverlay remove                    # Interactive
repoverlay remove my-overlay         # Direct

# After (explicit)
repoverlay remove --interactive
repoverlay remove my-overlay
```

### 5. `--dry-run` Not Available Everywhere

**Problem**: `--dry-run` exists on some commands but not others:
- Has: restore, update, create, sync, add
- Missing: apply, remove, list

**Solution**: Add `--dry-run` to `apply` and `remove`

```bash
repoverlay apply <source> --dry-run   # Show what would be applied
repoverlay remove <name> --dry-run    # Show what would be removed
```

### 6. GitHub-Specific Flags Mixed with General

**Problem**: `--ref` and `--update` only matter for GitHub URLs but appear alongside general flags.

**Solution**: Group in help text:
```
GITHUB OPTIONS (only for https://github.com/ URLs):
    --ref <REF>      Git ref to checkout (branch, tag, commit)
    --update         Force cache update even if not stale

GENERAL OPTIONS:
    --target <DIR>   Target repository directory
    --name <NAME>    Override overlay name
    --copy           Force copy mode (no symlinks)
```

## Low-Priority Issues

### 7. Missing Common Flags

**Add these flags for better UX**:

| Flag | Commands | Purpose |
|------|----------|---------|
| `--quiet` / `-q` | All | Suppress output |
| `--verbose` / `-v` | All | More detailed output |
| `--json` | status, list | Machine-readable output |
| `--force` / `-f` | remove, apply | Skip confirmations/override conflicts |

### 8. `--from` Flag Unclear

**Problem**: `--from` in `apply` is only meaningful with multi-source configs.

**Solution Options**:
- Rename to `--use-source` (clearer intent)
- Improve help text to explain when it matters

```bash
# Current
repoverlay apply org/repo/name --from personal

# Clearer
repoverlay apply org/repo/name --use-source personal
```

### 9. Inconsistent Short Flags

**Problem**: Not all common options have short flags.

**Solution**: Add short flags consistently:
```
--target   → -t (all commands) ✓ already have
--name     → -n (all commands) ✓ already have
--dry-run  → -d (add to all)
--force    → -f (add to all)
--yes      → -y (add to all)
--json     → -j (status, list)
--quiet    → -q (all)
--verbose  → -v (all)
```

## Implementation Order

### Phase 1: Quick Wins (Low Risk)
1. [ ] Rename `list --target` to `list --filter`
2. [ ] Add `--dry-run` to `apply` command
3. [ ] Add `--json` output to `status` command
4. [ ] Improve help text grouping for GitHub-specific options

### Phase 2: New Features (Medium Risk)
5. [ ] Add `--force` flag to `remove` command
6. [ ] Add `--quiet` and `--verbose` global flags
7. [ ] Add `--json` output to `list` command

### Phase 3: Structural Changes (Higher Risk)
8. [ ] Split `create` into `create` and `create-local`
9. [ ] Add `--exclusive` flag to `apply` (or document `switch`)
10. [ ] Standardize positional argument handling

## Breaking Changes

The following changes would be breaking:
- Renaming `list --target` to `list --filter`
- Splitting `create` command

**Mitigation**:
- Keep old flags as hidden aliases for one release cycle
- Document migration in CHANGELOG

## Related Files

- `src/cli.rs` - Command definitions and handlers
- `ARCHITECTURE.md` - CLI documentation
- `tests/cli.rs` - Integration tests (update for any changes)

## Notes

- Current CLI has 12 top-level commands + 2 subcommand groups (cache, source)
- Total of ~20 distinct operations
- Most commands follow consistent patterns already
- Focus on the high-priority issues first for maximum impact
