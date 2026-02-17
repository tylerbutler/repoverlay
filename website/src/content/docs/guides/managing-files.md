---
title: Managing Files
sidebar:
  order: 3
---

<!-- TODO: Add examples with real-world file management scenarios -->

This guide covers adding files to existing overlays and syncing changes back to overlay sources.

## Adding files to an overlay

The `add` command copies new files to an existing overlay source, replaces the originals with symlinks, and auto-commits/pushes the changes:

```bash
# Add a single file
repoverlay add my-overlay newfile.txt

# Add multiple files
repoverlay add my-overlay file1.txt file2.txt .config/settings.json
```

## Preview before adding

Use `--dry-run` to see what would happen without making changes:

```bash
repoverlay add my-overlay config.json --dry-run
```

## Syncing changes back

If you've modified overlay files in your repo (e.g., edited a symlinked config), the `sync` command copies those changes back to the overlay source:

```bash
repoverlay sync my-overlay
repoverlay sync my-overlay --dry-run
```

This is useful when you've tweaked a config in one repo and want to propagate the change to all repos using that overlay.
