---
title: Sources
sidebar:
  order: 2
---

<!-- TODO: Add more examples and edge cases -->

repoverlay supports three types of overlay sources: local directories, GitHub repositories, and overlay repository references.

## Local directories

The simplest source — a directory on your filesystem:

```bash
repoverlay apply /path/to/overlay
repoverlay apply ./relative/overlay
```

Files are symlinked (or copied) directly from the source directory. Changes to the source are reflected immediately when using symlinks.

## GitHub repositories

Pull overlays from any public GitHub repository:

```bash
# Default branch
repoverlay apply https://github.com/owner/repo

# Specific branch or tag
repoverlay apply https://github.com/owner/repo/tree/v1.0.0
repoverlay apply https://github.com/owner/repo --ref develop

# Subdirectory within a repo
repoverlay apply https://github.com/owner/repo/tree/main/overlays/rust
```

GitHub sources are **cached locally** using shallow clones. Use `repoverlay update` to pull new changes.

## Overlay repository references

If you've configured a shared overlay repository, you can reference overlays by name:

```bash
repoverlay apply org/repo/overlay-name
```

This is the most convenient way to share overlays across a team or organization. See [Overlay Repositories](/concepts/overlay-repos/) for setup details.

## How sources are resolved

<!-- TODO: Detail the resolution algorithm and priority ordering -->

When you provide a source string, repoverlay determines the type automatically:

1. Strings starting with `https://github.com/` are treated as **GitHub sources**
2. Strings that look like filesystem paths (`./`, `/`, `~/`) are treated as **local sources**
3. Strings matching the `org/repo/name` pattern are treated as **overlay repo references**
