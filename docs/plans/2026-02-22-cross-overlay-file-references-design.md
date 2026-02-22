# Cross-Overlay File References

## Problem

AI agents and tools sometimes expect the same configuration files at different paths. Today, each overlay is self-contained — if two overlays need the same file at different target locations, you must duplicate it. This creates maintenance burden and drift risk.

## Solution

Extend the `mappings` and `directories` sections in `repoverlay.ccl` to support referencing files and directories from other overlays in the same overlay repository.

## Mapping key forms

Mapping keys gain two new prefixes. The right-hand side (target path) is unchanged.

| Key form | Resolves from | Use case |
|----------|---------------|----------|
| `foo.txt` | Current overlay directory | Existing behavior |
| `../sibling/foo.txt` | Relative to overlay directory | Sibling overlays in same org/repo |
| `//org/repo/name/foo.txt` | Overlay repo root | Cross-org/repo references |

### Examples

```
mappings =
  /= Local file, remapped target (existing behavior)
  CLAUDE.md = .ai/instructions.md

  /= Relative: sibling overlay in same org/repo
  ../claude-config/CLAUDE.md = .cursor/instructions.md

  /= Repo-root: fully qualified path to any overlay
  //microsoft/FluidFramework/claude-config/.envrc = .envrc
```

## Directory references

The `directories` section gains the same prefix support. Referenced directories are symlinked (or copied) as a unit, identical to local directories today.

```
directories =
  /= Local directory (existing behavior)
  = .claude

  /= Relative: directory from sibling overlay
  = ../claude-config/.claude

  /= Repo-root: directory from any overlay
  = //microsoft/FluidFramework/claude-config/scratch
```

When a directory reference uses `../` or `//`, it resolves to a directory in another overlay's source tree and is symlinked/copied as a unit into the target repo.

## Path resolution

All three forms resolve to an absolute filesystem path during overlay application:

1. **Plain key** (`foo.txt`): `<overlay_source_dir>/foo.txt` — unchanged.
2. **Relative key** (`../sibling/foo.txt`): Join with `<overlay_source_dir>`, then canonicalize.
3. **Repo-root key** (`//org/repo/name/foo.txt`): Strip `//`, join with `<overlay_repo_root>`.

After resolution, validate:
- The resolved path is within the overlay repo root (reject repo escapes).
- The file or directory exists (fail with a clear error if missing).
- The path doesn't point to `repoverlay.ccl`, `.git/`, or `.repoverlay-cache-meta.ccl`.

## Changes to `collect_overlay_files`

Today the function walks the overlay directory and renames targets via mappings. The change:

1. **Walk phase** (unchanged): Walk overlay directory, apply plain mappings as renames, skip directories listed in `directories`.
2. **External references phase** (new): After the walk, iterate over mappings whose keys start with `../` or `//`. For each:
   - Resolve the key to an absolute path.
   - Validate it (repo boundary, existence, filtered paths).
   - Add `(resolved_source_path, target_path)` to the file list.
3. **External directories phase** (new): For directory entries starting with `../` or `//`:
   - Resolve to an absolute path.
   - Validate it (repo boundary, existence, is a directory).
   - Add as a directory entry for atomic symlink/copy.

## State tracking

`FileEntry` already has separate `source` and `target` fields. For cross-overlay files:
- `source` records the path relative to the overlay repo root (e.g., `microsoft/FluidFramework/claude-config/CLAUDE.md`).
- `target` records the target path in the repo as today.
- `entry_type` is `File` or `Directory` as today.

No structural changes to `FileEntry` or `OverlayState`.

## Overlay repo root discovery

`collect_overlay_files` currently only receives the overlay source directory. To resolve `//` paths, it also needs the overlay repo root. This can be:
- Passed as an additional parameter (preferred — explicit).
- Inferred by walking up from the overlay source dir to find the repo root.

For `../` relative paths, only the overlay source dir is needed (resolve relative to it).

## Error messages

Clear errors for common mistakes:
- `Cross-overlay reference escapes repo root: ../../../etc/passwd`
- `Cross-overlay file not found: //microsoft/FluidFramework/missing/CLAUDE.md`
- `Cross-overlay reference points to config file: //org/repo/name/repoverlay.ccl`
- `Cross-overlay directory not found: ../nonexistent/.claude`

## Scope and deferral

**In scope:**
- `//` and `../` prefixes in `mappings` keys
- `//` and `../` prefixes in `directories` entries
- Path validation and error handling
- State tracking for cross-overlay sources
- Documentation updates

**Deferred:**
- Circular reference detection (overlay A imports from B which imports from A) — unlikely in practice, can add later if needed.
- Transitive resolution (overlay B references overlay A which itself has cross-overlay references) — files are resolved from the filesystem, so this works implicitly as long as the referenced path exists.
- `repoverlay create` support for generating cross-overlay configs — manual authoring for now.
