---
title: Applying Overlays
sidebar:
  order: 1
---

<!-- TODO: Add screenshots or terminal output examples -->

This guide covers the different ways to apply overlays to a git repository.

## Basic usage

Apply an overlay from a local directory:

```bash
repoverlay apply /path/to/overlay
```

Apply from a GitHub repository:

```bash
repoverlay apply https://github.com/owner/repo
```

## Targeting a specific repo

By default, repoverlay applies to the current directory. Use `--target` to specify a different repository:

```bash
repoverlay apply ./overlay --target /path/to/repo
```

## Naming overlays

repoverlay auto-generates a name from the source. Use `--name` to set a custom name:

```bash
repoverlay apply ./overlay --name my-config
```

The name is used to identify the overlay in `status`, `remove`, and other commands.

## Copy mode

Use `--copy` to copy files instead of creating symlinks:

```bash
repoverlay apply ./overlay --copy
```

This is useful when symlinks aren't supported or when you want independent copies that won't change when the source is modified.

:::tip
Use `--copy` on Windows if your project doesn't support symlinks, or in CI environments where symlinks may not behave as expected.
:::

## GitHub-specific options

### Branches and tags

```bash
# Specific branch
repoverlay apply https://github.com/owner/repo --ref develop

# Tag (via URL path)
repoverlay apply https://github.com/owner/repo/tree/v1.0.0
```

### Subdirectories

Apply only a subdirectory of a GitHub repository:

```bash
repoverlay apply https://github.com/owner/repo/tree/main/overlays/rust
```

## Conflict handling

<!-- TODO: Document conflict strategies and --force behavior -->

:::caution
If an overlay file conflicts with an existing file in the repo, repoverlay will warn you and skip the conflicting file by default.
:::
