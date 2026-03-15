# Overlay Library: In-Repo Shareable Overlay Storage

**Date**: 2026-03-15
**Status**: Approved

## Problem

Overlays currently live in external locations (local paths, GitHub repos, shared overlay repos). There's no way to store overlays *within* a project repo so they're version-controlled and shareable with anyone who clones the repo. Users also lack ergonomic commands for moving overlays between storage locations.

## Solution

Add `.repoverlay/library/` as a git-tracked, in-repo overlay storage directory that is automatically registered as an overlay source. Add `library` subcommands for managing library contents and a `move` command for relocating overlays between any storage locations.

## Storage Model

### On-Disk Layout

```
target-repo/
├── .repoverlay/
│   ├── meta.ccl                    # existing global metadata
│   ├── repoverlay.ccl              # per-repo config (library path configured here)
│   ├── overlays/                   # existing: applied overlay state (git-ignored)
│   │   └── my-overlay.ccl
│   └── library/                    # NEW: in-repo overlay storage (git-tracked)
│       ├── claude-config/
│       │   ├── CLAUDE.md
│       │   ├── .claude/
│       │   └── repoverlay.ccl
│       └── dev-env/
│           ├── .envrc
│           └── repoverlay.ccl
```

### Auto-Registration as Source

When repoverlay detects the library directory in the current repo, it registers it as an implicit local source with the reserved name `@library`. This source is checked **before** any user-configured sources, so in-repo overlays take priority.

The library source is not persisted to global config — it's discovered at runtime from the working directory. Different repos each have their own library without global config pollution.

The `@library` name is reserved. `repoverlay source add` rejects source names starting with `@` to prevent conflicts with built-in sources. The `--from` flag on `apply` accepts `@library` to explicitly target the library source, bypassing priority ordering.

### Git Tracking

`.repoverlay/library/` is git-tracked. The existing `.repoverlay/overlays/` directory (applied state) remains git-ignored.

If the repo has a blanket `.repoverlay/` gitignore rule, repoverlay should warn that the library directory would be excluded from git and suggest updating the ignore pattern (e.g., adding `!.repoverlay/library/`).

### Configurable Path

The library path is configurable via `.repoverlay/repoverlay.ccl` (per-repo config):

```
library
  path = .overlays
```

Resolution: per-repo config → default (`.repoverlay/library/`).

No global config override — the library path is a repo-level concern.

The configured path must be relative and within the repo root. Absolute paths or paths that escape the repo (e.g., `../../other-repo`) are rejected with an error.

### Changing the Library Path

If the library path is changed in config while overlays are applied from the old path, `OverlaySource::Library { name }` entries resolve against the *current* configured path. If the overlay doesn't exist at the new path, `repoverlay status` shows a warning and `restore`/`update` skip it. The user is responsible for moving the library contents to the new path — repoverlay does not auto-migrate.

## State Representation

### New `OverlaySource::Library` Variant

A new `OverlaySource` variant is added:

```rust
OverlaySource::Library { name: String }
```

The `name` field is the overlay name within the library (e.g., `claude-config`). The actual path is resolved at runtime by combining the repo root + configured library path + name, rather than storing an absolute path. This ensures:

- External backups at `~/.local/share/repoverlay/applied/` are portable — they don't contain machine-specific paths.
- `repoverlay restore` can resolve the library overlay as long as the library still exists in the repo.
- `repoverlay update` detects library overlays and skips them with a message: `Skipping 'X' (library overlay — update via git)`. Library overlays are managed by git, not repoverlay's update mechanism.

Serialized in CCL as: `source = library|<name>`

### Source Resolution for Bare Names

When `repoverlay apply my-overlay` is called with a bare name (no `/` separators), resolution follows this order:

1. **Library lookup** — check `@library` source for a directory matching the name
2. **Existing source resolution** — fall through to `SourceManager.resolve()` which checks configured sources with the `org/repo/name` pattern

This is a new resolution path that runs *before* the existing `SourceManager`. The library uses flat name-based lookup (just the overlay name), not the `org/repo/name` hierarchy. The existing `resolve()` method is unchanged — library resolution is a separate step in the apply flow.

### Status Display

`repoverlay status` displays library-sourced overlays with `(library)` as the source type:

```
Overlay: claude-config
  Source:  claude-config (library)
  Applied: 2026-03-15T10:30:00Z
```

The `--json` output includes `"source_type": "library"` and `"library_name": "claude-config"`.

## CLI Commands

### `repoverlay library` Subcommand

```
repoverlay library list                          # List overlays in the repo's library
repoverlay library import <source>               # Copy overlay into library
repoverlay library import <source> --name <name> # Copy with a different name
repoverlay library export <overlay> --to <dest>  # Copy overlay out of library
repoverlay library remove <overlay>              # Remove overlay from library
```

`<source>` accepts the same inputs as `apply` — a local path, GitHub URL, applied overlay name, or `org/repo/name` reference.

When importing from an **applied overlay name**, repoverlay resolves the original source from the applied state and copies from there (following symlinks back to the source directory). If the original source is no longer available (e.g., deleted cache), repoverlay falls back to collecting the applied files from their target locations in the repo.

`<dest>` is a filesystem path or a source reference (e.g., `source:my-shared-repo`).

### `repoverlay move` Command

```
repoverlay move <overlay> --to library           # Move into library
repoverlay move <overlay> --to <path>            # Move to filesystem path
repoverlay move <overlay> --to source:<name>     # Move to a named source
```

`<overlay>` can be an applied overlay name (resolved from state) or a path. After the move, the current repo's applied state references are updated automatically.

### Destination Shorthand

| Shorthand | Resolves To |
|-----------|-------------|
| `library` | Configured library path (default: `.repoverlay/library/`) |
| `source:<name>` | Root of a named overlay source |
| `/path/to/dir` | Literal filesystem path |

### `create --into library`

Extends the existing `create` command:

```
repoverlay create --into library              # Auto-detected name
repoverlay create --into library --name foo   # Explicit name
```

After creating into the library, prompts to apply:

```
Created overlay 'my-overlay' in .repoverlay/library/my-overlay/
Apply it now? [Y/n]
```

Default yes. `--yes` auto-confirms the apply prompt (overlay is applied). `--no-apply` explicitly skips the prompt and does not apply.

### Interaction with Existing Commands

- `repoverlay apply my-overlay` — resolves from library (if present) before checking other sources
- `repoverlay browse` — includes library overlays in the listing

## Export to Named Sources

When exporting to a source that uses `org/repo/name` structure:

1. **Infer from current repo** — detect the current repo's org/repo from git remotes and place at `<source-root>/org/repo/overlay-name/`
2. **Fallback** — if remotes can't be parsed, require `--target-repo org/repo`

The overlay remains in the library after export (export is a copy). Use `move` to remove it.

### Git-Backed Source Destinations

When exporting or moving to a git-backed source, repoverlay copies files into the local clone but does **not** commit or push automatically. It prints a message directing the user to commit and push the changes in the source repo:

```
Exported 'claude-config' to source 'my-shared-repo' at /path/to/clone/org/repo/claude-config/
Note: Changes are not committed. Commit and push in the source repo to make them available.
```

## Source Reference Updates on Move

### What Gets Updated

The state file at `.repoverlay/overlays/<name>.ccl` has its `source` field rewritten to point to the new location. The external backup at `~/.local/share/repoverlay/applied/` is also updated.

### Scope

Only the **current repo** is updated automatically. repoverlay has no registry of every repo that has applied an overlay. Other repos pick up new locations via `repoverlay restore` or `repoverlay update`.

### Symlink Handling

If the move changes the symlink target location, symlinks are re-created. The operation preserves state (`applied_at` doesn't change).

**Ordering**: copy to destination → update state files → re-create symlinks → delete from source. If interrupted, the worst case is a duplicate (overlay exists in both locations), which is safe and can be cleaned up manually.

### Link Type Preservation

Move preserves the original `link_type` from the applied state:
- **Symlink** entries get symlinks re-created pointing to the new location.
- **Copy** entries are left as-is — the files in the repo are independent copies and don't need updating. Only the state `source` field is updated.
- **Merged** entries (JSON merge) are left as-is — merged content is already baked into the target file. Only the state `source` field is updated.

## Error Handling

### Library Not Initialized

If the library directory doesn't exist when running `library import` or `create --into library`, create it automatically. No explicit init command.

### Move/Import from GitHub Sources

Overlays are fetched/cached as usual, then copied into the library as plain files. No submodule or symlink-to-cache indirection.

### Circular Moves

Moving an overlay from library to library (same repo) is a no-op with a warning.

### Name Conflicts

All commands that write to a destination check for existing overlays with the same name:
- **Default**: error with message
- **`--force`**: overwrite
- **`--name <new-name>`**: rename on write

### Applied Overlay Safety

`library remove` of an overlay that's currently applied from the library is **blocked** unless `--force`. Error message: "Overlay 'X' is currently applied. Remove it first with `repoverlay remove X`, or use `--force`."

With `--force`, the library directory is removed and the applied overlay state is updated to mark the source as missing. Applied files (symlinks or copies) are left in place — they still function but can no longer be updated or restored. `repoverlay status` shows a warning: `Source missing (library entry removed)`.

## Testing Strategy

### Unit Tests

- Library path resolution: default path, configured path, missing config
- Source auto-registration: library detected and registered with correct priority
- State reference updates: source field rewritten correctly after move for all source types
- Name collision detection: error on conflict, force overwrites, rename works

### Integration Tests (`tempfile::TempDir`)

- `library import` from local path: copies overlay, list shows it, apply resolves it
- `library import` from applied overlay: extracts applied overlay into library
- `library export` to path: copies out correctly
- `create --into library`: creates in library, prompts to apply
- `move` to library: removes from source, updates applied state, symlinks still work
- `move` from library: removes from library, updates applied state
- `library remove` while applied: blocks without `--force`
- Priority ordering: library overlay wins over same-name overlay in other sources

### Integration Tests (continued)

- Custom library path with applied overlays: changing library path in config while overlays are applied from old path
- `@library` source name reservation: `source add @library` rejected
- `--from @library` explicitly targets library source

### CLI Tests (`assert_cmd`)

- `library list`: output format, empty library, populated library
- `library list` with custom path: respects `repoverlay.ccl` config
- `status` output: library-sourced overlays show `(library)` source type
- `status --json`: includes `source_type: "library"` field
