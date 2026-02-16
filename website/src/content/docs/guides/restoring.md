---
title: Restoring After Git Clean
sidebar:
  order: 5
---

<!-- TODO: Add detail about what gets restored and from where -->

If you run `git clean -fd` or otherwise lose your overlay files, repoverlay can restore them from its external backup.

## Restoring overlays

```bash
repoverlay restore
```

This re-applies all previously applied overlays using the information stored in `~/.local/share/repoverlay/applied/`.

## Preview before restoring

```bash
repoverlay restore --dry-run
```

## How backups work

Every time an overlay is applied, repoverlay saves a copy of the overlay state to an **external backup location** outside the git repository. This backup survives `git clean` and other operations that remove untracked files.

The external backup stores:
- The overlay name and source
- The list of files and their link types
- Enough information to re-apply the overlay from the original source

## When to use restore

- After running `git clean -fd` or `git clean -fdx`
- After checking out a branch that removes the `.repoverlay/` directory
- After any operation that deletes untracked files from the repository
