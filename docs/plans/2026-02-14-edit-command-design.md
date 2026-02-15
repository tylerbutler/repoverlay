# Edit Command Design

**Date:** 2026-02-14
**Issue:** #81

## Summary

Add an `edit` command that allows modifying an existing overlay — adding files, removing files, or re-running interactive file selection. The command subsumes the existing `add` command, which will be deprecated.

## CLI Interface

```
repoverlay edit <name> --add <files...>
repoverlay edit <name> --remove <files...>
repoverlay edit <name> --add f1 --remove f2
repoverlay edit <name> --interactive
repoverlay edit <name> [options] --dry-run
```

### Arguments

| Argument | Description |
|---|---|
| `name` | Overlay name (`my-overlay`) or full path (`org/repo/name`) |
| `--add`, `-a` | Files to add to the overlay (1 or more paths) |
| `--remove`, `-r` | Files to remove from the overlay (1 or more paths) |
| `--interactive`, `-i` | Re-run interactive file selection with current files pre-selected |
| `--target`, `-t` | Target repository directory (defaults to `.`) |
| `--dry-run` | Show what would change without making changes |

### Constraints

- At least one of `--add`, `--remove`, or `--interactive` must be specified
- `--interactive` conflicts with `--add` and `--remove`
- `--add` and `--remove` can be combined in a single invocation

## Operations

### Add Files (`--add`)

Reuses the existing `add_files_to_overlay` logic:

1. Validate files exist in target repo
2. Check files aren't managed by another overlay
3. Copy each file to the overlay repo
4. Replace original with symlink
5. Add `FileEntry` to state
6. Update `.git/info/exclude`
7. Save state + external backup
8. Auto-commit to overlay repo

### Remove Files (`--remove`)

New logic:

1. Load overlay state
2. For each file path, find matching `FileEntry`
3. Remove symlink/copy from target repo
4. Clean up empty parent directories (walk up to target root)
5. Remove `FileEntry` from state
6. Remove file from overlay repo source
7. Rewrite `.git/info/exclude` section with remaining files
8. Save updated state + external backup
9. Auto-commit to overlay repo

### Interactive Re-selection (`--interactive`)

1. Load overlay state and resolve overlay source path
2. List all available files in the overlay source directory
3. Pre-select the currently applied files
4. Launch selection UI (reuse `selection.rs`)
5. Compute diff: newly selected = add, deselected = remove
6. Apply additions and removals
7. Update state, exclude, backup
8. Auto-commit to overlay repo

## State Changes

### New method on `OverlayState`

```rust
pub fn remove_file(&mut self, target: &Path) -> Option<FileEntry> {
    if let Some(pos) = self.files.iter().position(|f| f.target == target) {
        Some(self.files.remove(pos))
    } else {
        None
    }
}
```

### Git exclude handling

When removing files, the overlay's section in `.git/info/exclude` is rewritten with only the remaining files. The existing `update_git_exclude` with `add=false` removes the entire section, so we need a "replace section" variant that writes the remaining entries.

## Deprecation of `add`

The existing `Add` command variant gets `#[command(hide = true)]` (same pattern as `Publish`). When invoked, it prints a deprecation warning:

```
Warning: 'repoverlay add' is deprecated. Use 'repoverlay edit --add' instead.
```

Then delegates to the same underlying logic.

## Error Handling

| Scenario | Behavior |
|---|---|
| File to remove not in overlay | Error listing overlay's files |
| File to add already managed by another overlay | Error naming the other overlay |
| File to add doesn't exist | Error suggesting to create it first |
| No operations specified | Error with usage hint |
| Empty overlay after removal | Warn but allow |
| Interactive mode with non-overlay-repo source | Error (only works with overlay repo sources) |
| Overlay not applied | Error suggesting to apply first |

## Decisions

- **Subsume `add`**: `edit` becomes the single command for overlay modification. `add` deprecated with hidden alias.
- **File removal behavior**: Deletes the symlink/copy from target repo. The file remains in the overlay source.
- **Combined operations**: `--add` and `--remove` can be used together in one invocation.
- **Interactive shows overlay source files**: With currently-applied files pre-selected.
