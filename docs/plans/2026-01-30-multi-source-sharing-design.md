# Multi-Source Overlay Sharing

Design for easier overlay sharing between users, addressing discovery, distribution, and contributing friction.

## Goals

- **Discovery**: Users can find overlays for repos they work on
- **Distribution**: Easy to get overlays without manual URL configuration
- **Contributing**: Anyone can publish/share overlays without central repo access

## Non-Goals

- Centralized registry infrastructure (leverage GitHub instead)
- Quality/moderation system (out of scope for initial implementation)

---

## Core Concepts

### Sources

Sources are repositories containing overlays. Users configure multiple sources; order determines priority (first = highest).

```
~/.config/repoverlay/config.ccl

sources =
  name = personal
  url = https://github.com/me/my-overlays

sources =
  name = my-team
  url = https://github.com/my-org/team-overlays

sources =
  name = community
  url = https://github.com/repoverlay/overlays
```

Personal overlays checked first, then team, then community. Moving a source up/down in the file changes its priority.

### Gists

Gists are lightweight single-overlay shares. A gist contains overlay files directly (no `org/repo/name` structure). Identified by ID or URL.

### Resolution Order

When applying `microsoft/FluidFramework/claude-config`:

1. Check each source top-to-bottom
2. First match wins
3. If no match and upstream detected, retry with upstream org/repo

Gist resolution is explicit - no source lookup:

```bash
repoverlay apply gist:abc123def456
repoverlay apply https://gist.github.com/user/abc123def456
```

---

## CLI Commands

### Source Management

```bash
# Add a new source (appends to end = lowest priority)
repoverlay source add https://github.com/my-org/overlays
repoverlay source add https://github.com/my-org/overlays --name my-team

# Add at specific position (1 = highest priority)
repoverlay source add https://github.com/me/personal --position 1

# List configured sources (shows order)
repoverlay source list
#  1. personal   https://github.com/me/my-overlays
#  2. my-team    https://github.com/my-org/team-overlays
#  3. community  https://github.com/repoverlay/overlays

# Remove a source
repoverlay source remove my-team

# Reorder (move source to position)
repoverlay source move community --position 1
```

> **Note:** These commands edit `~/.config/repoverlay/config.ccl`. Users can edit the file directly for the same effect. The commands provide convenience (auto-clone on add, validation, easier reordering) but the config file is the sole source of truth.

### Gist Sharing

```bash
# Apply overlay from gist
repoverlay apply gist:abc123def456

# Create gist from local overlay directory
repoverlay publish ./my-overlay --gist
# → Created: https://gist.github.com/you/abc123def456
# → Share with: repoverlay apply gist:abc123def456

# Create gist from currently applied overlay
repoverlay publish --from-applied my-overlay --gist
```

### Discovery

```bash
# Search across all configured sources
repoverlay search claude
repoverlay search microsoft/FluidFramework
```

---

## Gist Format & Publishing

### Gist Structure

A gist is a flat collection of files. No `org/repo/name` directory structure needed - gists are self-contained.

```
my-overlay-gist/
├── repoverlay.ccl      # Optional metadata
├── .claude/
│   └── settings.json
├── .cursorrules
└── CLAUDE.md
```

### Metadata File (Optional)

```
repoverlay.ccl

description = Claude configuration for React projects
author = tylerbutler
target = */*           # Applies to any repo (or "facebook/react" for specific)
```

The `target` field is informational - helps users understand what the overlay is for. It doesn't restrict where the overlay can be applied.

### Publishing Workflow

```bash
# From a local directory
repoverlay publish ./my-overlay --gist
# → Authenticates via gh CLI (must be installed/authed)
# → Creates gist with all files
# → Prints shareable command

# From an applied overlay (extracts files back out)
repoverlay publish --from-applied claude-config --gist

# Public vs secret gist
repoverlay publish ./my-overlay --gist              # Secret (unlisted) by default
repoverlay publish ./my-overlay --gist --public     # Publicly listed
```

### Fetching

Gists are fetched via GitHub API, cached in `~/.cache/repoverlay/gists/<gist-id>/`. Cache is refreshed on `repoverlay update`.

---

## Multi-Source Resolution

### How Apply Resolves an Overlay Reference

```bash
repoverlay apply microsoft/FluidFramework/claude-config
```

1. Parse as `org/repo/name` format
2. For each source (in config order):
   - Check if `<source-path>/microsoft/FluidFramework/claude-config/` exists
   - If found, apply from that source and stop
3. If no match and current repo has upstream remote:
   - Repeat search using upstream's org/repo
4. If still no match, error with helpful message listing searched sources

### Status Output Shows Source

```bash
repoverlay status

Overlay: claude-config
  Source: microsoft/FluidFramework/claude-config
  From:   my-team (https://github.com/my-org/team-overlays)
  Files:  3 applied
```

### Conflict Visibility

```bash
# Show what would be used from each source
repoverlay resolve microsoft/FluidFramework/claude-config

Found in 2 sources:
  1. my-team    → microsoft/FluidFramework/claude-config (would be used)
  2. community  → microsoft/FluidFramework/claude-config

# Force apply from specific source
repoverlay apply microsoft/FluidFramework/claude-config --source community
```

---

## Caching & Updates

### Source Cloning

When a source is added, it's shallow-cloned to `~/.cache/repoverlay/sources/<name>/`. Each source is an independent git repo.

```bash
repoverlay source add https://github.com/repoverlay/overlays --name community
# → Clones to ~/.cache/repoverlay/sources/community/
```

### Gist Caching

Gists are fetched via GitHub API and cached to `~/.cache/repoverlay/gists/<gist-id>/`.

### Update Commands

```bash
# Update all sources (git pull on each)
repoverlay update --sources

# Update specific source
repoverlay update --source my-team

# Update all applied overlays (re-applies from updated sources)
repoverlay update

# Update gist-based overlays (re-fetches gist content)
repoverlay update
```

### Cache Management

```bash
# Show cache usage
repoverlay cache status
# Sources: 3 (45 MB)
# Gists:   7 (120 KB)

# Clear unused (sources no longer in config, gists not applied)
repoverlay cache clean

# Clear everything
repoverlay cache clean --all
```

---

## Migration & Edge Cases

### Backwards Compatibility

The current config format uses a single `overlay_repo` key. Migration path:

```
# Old format
overlay_repo =
  url = https://github.com/my-org/overlays

# New format (automatically migrated on first run)
sources =
  name = default
  url = https://github.com/my-org/overlays
```

repoverlay detects old format and migrates automatically, printing a notice.

### Edge Cases

| Scenario | Behavior |
|----------|----------|
| No sources configured | `apply org/repo/name` fails with "no sources configured, use `repoverlay source add`" |
| Source URL unreachable | Skip source, continue to next; warn if all sources fail |
| Gist not found | Error with "gist not found: abc123" |
| Gist has no files | Error with "gist is empty" |
| Overlay exists in 0 sources | List all searched sources in error message |
| `--source` flag with unknown source | Error with "unknown source: foo, available: personal, my-team, community" |

### Direct URL Support

For quick local sharing, support direct GitHub URLs to overlay directories:

```bash
# These all work
repoverlay apply gist:abc123
repoverlay apply https://github.com/user/repo/tree/main/overlays/my-config
repoverlay apply microsoft/FluidFramework/claude-config  # via sources
```

---

## Implementation Approach (TDD)

### Testable Units in Dependency Order

1. **Config parsing** - Parse new `sources` list format, detect old format
   - Input: CCL string → Output: `Vec<Source>` or migration needed
   - Pure function, easy to test

2. **Source resolution** - Given sources + overlay ref → find matching path
   - Input: `&[Source]`, `"org/repo/name"` → Output: `Option<(PathBuf, &Source)>`
   - Test: priority order, first-match-wins, upstream fallback

3. **Multi-source resolver** - Orchestrates checking multiple sources
   - Mock source existence checks
   - Test: resolution order, `--source` override, `resolve` command output

4. **Gist parsing** - Parse gist ID from various formats
   - Input: `"gist:abc123"`, `"https://gist.github.com/user/abc123"` → Output: gist ID
   - Pure function

5. **Gist fetching** - Download gist contents via GitHub API
   - Mock HTTP responses
   - Test: success, not found, empty gist, rate limiting

6. **Gist publishing** - Create gist via `gh` CLI
   - Mock CLI execution
   - Test: file collection, public/secret flag, error handling

7. **Migration** - Detect and convert old config format
   - Input: old config → Output: new config
   - Test: preserves URL, assigns default name, idempotent

### Integration Tests

- Add source → apply overlay → verify files → remove source
- Publish gist → apply gist → verify files
- Multiple sources with same overlay → verify priority
