---
title: The In-Repo Library
sidebar:
  order: 6
---

The library is a directory of overlays stored *inside* a repository, at `.repoverlay/library/` by default. Unlike applied overlay files, library overlays are tracked by git — commit them and everyone who clones the repo gets them.

Use the library when:

- A team wants to ship optional config (editor settings, AI configs) with the repo itself, instead of hosting a separate overlay repository
- You want overlays that [extend or include](/guides/creating/#composing-overlays) other overlays — composition only works between library overlays

## Applying library overlays

Library overlays are resolved by bare name, before any configured source:

```bash
repoverlay apply my-overlay
```

If `.repoverlay/library/my-overlay/` exists, that's what gets applied.

## Adding overlays to the library

Create a new overlay directly in the library:

```bash
# Create into the library and apply it
repoverlay create my-overlay --into library

# Create into the library without applying
repoverlay create my-overlay --into library --no-apply
```

Import an existing overlay — from a path, a GitHub URL, an `org/repo/name` reference, or the name of an already-applied overlay:

```bash
repoverlay library import ./path/to/overlay
repoverlay library import org/repo/overlay-name --name shared-config
```

Or move an applied overlay's source into the library:

```bash
repoverlay move my-overlay --to library
```

If the library path is covered by `.gitignore`, repoverlay appends a negation pattern (for example `!.repoverlay/library/`) so the library stays tracked.

## Listing, exporting, and removing

```bash
# List overlays in the library
repoverlay library list

# Copy a library overlay out to a directory
repoverlay library export my-overlay --to ../shared-overlays/my-overlay

# Delete a library overlay (--force if it's currently applied)
repoverlay library remove my-overlay
```

## Custom library location

Set `library_path` in the per-repo config (`.repoverlay/config.ccl`) to move the library. The path must be relative to the repo root:

```
library_path = tools/overlays
```
