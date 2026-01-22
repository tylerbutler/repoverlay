# Simplified Publish Workflow Design

## Overview

Simplify the overlay publishing workflow by removing the `publish` command and integrating publishing directly into `create` and a new `sync` command. The overlay repo becomes the primary working tree, with `push` handling remote synchronization.

## Current State

```
repoverlay create --output ./path      # Create overlay locally
repoverlay publish ./source --target org/repo --name foo  # Publish to overlay repo
```

Problems:
- Two-step process for publishing
- Redundant `--target` and `--name` flags
- Doesn't match the `org/repo/name` syntax used by `apply`

## Proposed Design

### Commands

#### `repoverlay create <name>` or `repoverlay create org/repo/name`

Create an overlay and write it directly to the overlay repo.

**Short form (from within a git repo):**
```bash
repoverlay create my-overlay
# Detects org/repo from git remote
# Writes to overlay repo at org/repo/my-overlay/
# Auto-commits
```

**Explicit form:**
```bash
repoverlay create myorg/myrepo/my-overlay
# Writes to overlay repo at myorg/myrepo/my-overlay/
# Auto-commits
```

**Local form:**
```bash
repoverlay create --local ./output
# Writes to local directory only
# No overlay repo involved
```

**Behavior:**
- If `org/repo/name` already exists in overlay repo: **error** (suggest `sync` or `--force`)
- With `--force`: overwrite existing overlay
- Auto-commits with message: "Add overlay: org/repo/name"

#### `repoverlay sync <name>` or `repoverlay sync org/repo/name`

Sync changes from an applied overlay back to the overlay repo.

```bash
repoverlay sync my-overlay
# Detects org/repo from git remote
# Copies changed files back to overlay repo
# Auto-commits
```

**Behavior:**
- Errors if the overlay isn't currently applied
- Only copies files that have changed
- Auto-commits with message: "Update overlay: org/repo/name"

#### `repoverlay push`

Push all pending commits in the overlay repo to remote.

```bash
repoverlay push
# Equivalent to: cd ~/.local/share/repoverlay/repo && git push
```

### Migration

| Current | New |
|---------|-----|
| `create --output ./path` | `create --local ./path` |
| `create` (to overlay repo) | `create <name>` or `create org/repo/name` |
| `publish ./source --target org/repo --name foo` | **Removed** |

The `publish` command will be removed entirely.

### Detection Logic

When running `create <name>` or `sync <name>` without explicit `org/repo`:
1. Look for git remote in current directory
2. Parse remote URL to extract `org/repo`
3. Error if not in a git repo or no remote configured

This logic already exists in `detect_target_repo()`.

### Argument Parsing

To distinguish between `<name>` and `org/repo/name`:
- Count slashes in the argument
- 0 slashes: `<name>` form, detect org/repo
- 2 slashes: `org/repo/name` form, use explicit values
- 1 slash: error (invalid format)

### Error Messages

**Overlay already exists:**
```
Error: Overlay 'myorg/myrepo/my-overlay' already exists.

To update an applied overlay, use: repoverlay sync my-overlay
To overwrite, use: repoverlay create my-overlay --force
```

**Overlay not applied (for sync):**
```
Error: Overlay 'my-overlay' is not currently applied.

To apply it first: repoverlay apply myorg/myrepo/my-overlay
```

**Not in a git repo:**
```
Error: Could not detect target repository.

Run from within a git repository, or specify explicitly:
  repoverlay create myorg/myrepo/my-overlay
```

## Implementation Steps

1. Update `create` command argument parsing to accept `org/repo/name` or `<name>`
2. Rename `--output` flag to `--local`
3. Add auto-commit after writing to overlay repo
4. Add `--force` flag to overwrite existing overlays
5. Add `sync` command
6. Add `push` command
7. Remove `publish` command
8. Update documentation and help text

## Testing

- `create foo` from a git repo with remote → writes to `detected-org/detected-repo/foo`
- `create org/repo/foo` → writes to `org/repo/foo`
- `create --local ./out` → writes to local directory
- `create foo` when `foo` exists → error
- `create foo --force` when `foo` exists → overwrites
- `sync foo` when applied → updates overlay repo
- `sync foo` when not applied → error
- `push` → pushes overlay repo
