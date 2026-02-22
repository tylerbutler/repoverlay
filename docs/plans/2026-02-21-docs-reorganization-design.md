# Docs Reorganization & Source UX Design

**Date:** 2026-02-21
**Issues:** #113, #127, #84

## Summary

Reorganize the documentation site around task-based guides (dropping the separate Concepts section), and change `browse`/`apply` behavior to simplify the onboarding experience. The primary Quick Start path becomes `repoverlay apply tylerbutler`.

## Feature Changes

### `browse` — no source required

Currently `browse` errors if no source is configured. Change it to accept a username/org/repo argument directly and fetch ephmerally (no source saved to config):

```
repoverlay browse tylerbutler        # fetches overlays, interactive pick, applies
repoverlay browse tylerbutler/repo   # browse a specific repo
repoverlay browse                    # uses configured sources (current behavior)
```

No source is saved. The repo is fetched to cache but the source config is not modified.

### `apply` — prompt to add source

When `apply` receives a username or two-part reference and the source is not already configured, prompt to persist it:

```
repoverlay apply tylerbutler
# → fetches, shows interactive picker
# → "Save tylerbutler as a source for future use? [Y/n]"
# → applies selected overlay
```

If the source is already configured, no prompt — just apply.

## Docs Site Reorganization

### New sidebar structure

```
Start Here
  What is repoverlay?
  Installation
  Quick Start                ← "repoverlay apply tylerbutler" walk-through

Guides
  Applying Overlays          ← all source types, conflict handling, --merge
                               aside: "browse without applying"
  Creating & Sharing         ← create, create-local, overlay repo structure, sharing
  Managing Applied Overlays  ← status, edit, sync, update, remove, switch
  Restoring After Git Clean  ← restore, how backups work
  How It Works               ← symlinks, git exclude, state, caching, fork inheritance

CLI Reference
```

### Current page disposition

| Current page | Disposition |
|---|---|
| `concepts/how-overlays-work.md` | → `guides/how-it-works.md` (expanded with caching + fork inheritance) |
| `concepts/sources.md` | Absorbed into `guides/applying.md` |
| `concepts/configuration.md` | Absorbed into `guides/creating.md` (as advanced section) |
| `concepts/overlay-repos.md` | Split: repo structure → creating, browsing → applying |
| `concepts/fork-inheritance.md` | Absorbed into `guides/how-it-works.md` |
| `guides/applying.md` | Rewritten |
| `guides/creating.md` | Rewritten |
| `guides/managing-files.md` | → `guides/managing.md` (rewritten, covers edit/sync/update/remove/switch) |
| `guides/updating.md` | Absorbed into `guides/managing.md` |
| `guides/switching.md` | Absorbed into `guides/managing.md` |
| `guides/restoring.md` | Stays, mostly unchanged |
| `guides/cache.md` | Absorbed into `guides/how-it-works.md` |

### Quick Start rewrite

Walk-through using `tylerbutler` as the example:

1. Install repoverlay
2. `cd ~/projects/my-repo`
3. `repoverlay apply tylerbutler` — interactive picker, select an overlay
4. `repoverlay status` — see what's applied
5. `repoverlay remove <name>` — clean up

Next steps link to the Applying guide and Creating guide.

## Page Content Outlines

### Applying Overlays

1. **Basic usage** — `apply ./path`, `apply https://github.com/...`, `apply tylerbutler`
2. **Source types** (explained inline) — local dirs, GitHub URLs, usernames, org/repo/name
3. **Interactive selection** — what happens with username or two-part reference
4. **Source persistence** — the prompt to save, `source add/remove/list` for manual management
5. **Conflict handling** — `--force`, `--skip-conflicts`, `--merge` (JSON deep merge), `--interactive`
6. **Other options** — `--copy`, `--name`, `--ref`, `--target`, `--dry-run`, `--from`
7. **Aside:** "Want to explore without applying? Use `repoverlay browse`"

### Creating & Sharing

1. **Creating from a repo** — `create my-overlay`, `create org/repo/name`, `--include`, interactive selector
2. **Local output** — `create-local ./output`
3. **Overlay configuration (advanced)** — `repoverlay.ccl` format: name, mappings, directories. Positioned as advanced, most useful for hand-authored overlays or cases where you need to remap files from their source location.
4. **Overlay repository structure** — the `org/repo/name` directory layout
5. **Sharing workflow** — push to GitHub, others apply via username or org/repo/name

### Managing Applied Overlays

1. **Check status** — `status`, `status --name` (start here to show nothing applied)
2. **Edit an overlay** — `edit --add`, `edit --remove`, `edit --interactive`
3. **Sync changes back** — `sync`
4. **Update from remote** — `update`, `update --dry-run`
5. **Remove overlays** — `remove <name>`, `remove --all`, `remove --interactive`
6. **Switch overlays** — `switch` as atomic remove-all + apply

### Restoring After Git Clean

Mostly unchanged from current content.

### How It Works

1. **Symlinks vs copies** — default behavior, `--copy`, platform considerations
2. **Git exclusion** — `.git/info/exclude` sections, why not `.gitignore`
3. **State tracking** — in-repo `.repoverlay/` + external backup at `~/.local/share/repoverlay/applied/`
4. **Caching** — shallow clones, cache location, `cache list/clear/remove/path`
5. **Fork inheritance** — upstream detection, resolution order, status display

## Fixes & Cleanup

1. **`index.mdx` banner** — Fix "Reoverlay" typo → "repoverlay"
2. **CLI reference** — Regenerate to include `browse` and reflect current command set
3. **Remove `concepts/` directory** — Delete all 5 files, remove sidebar section from `astro.config.mjs`
