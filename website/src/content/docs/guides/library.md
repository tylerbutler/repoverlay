---
title: The in-repo library
sidebar:
  order: 6
---

The library is a directory of overlays inside a repository. Its default location is `.repoverlay/library/`. Unlike applied overlay files, library overlays are tracked by git. Commit them so everyone who clones the repository gets them.

Use the library when:

- Your team wants optional configuration, such as editor settings or AI configuration, in the repository instead of a separate overlay repository.
- You want overlays that [extend or include](/guides/creating/#composing-overlays) other overlays. Only library overlays support this.

## Applying library overlays

Use the overlay name alone to apply a library overlay. repoverlay checks the library before configured sources:

```bash
repoverlay apply my-overlay
```

If `.repoverlay/library/my-overlay/` exists, repoverlay applies it.

## Adding overlays to the library

Create a new overlay directly in the library:

```bash
# Create into the library and apply it
repoverlay create my-overlay --into library

# Create into the library without applying
repoverlay create my-overlay --into library --no-apply
```

To import an existing overlay, specify a path, GitHub URL, `org/repo/name` reference, or applied overlay name:

```bash
repoverlay library import ./path/to/overlay
repoverlay library import org/repo/overlay-name --name shared-config
```

Or move an applied overlay's source into the library:

```bash
repoverlay move my-overlay --to library
```

If `.gitignore` excludes the library path, repoverlay adds a negation pattern such as `!.repoverlay/library/`. This lets git track the library.

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

Set `library_path` in the repository configuration (`.repoverlay/config.ccl`) to change the library location. The path must be relative to the repository root:

```
library_path = tools/overlays
```
