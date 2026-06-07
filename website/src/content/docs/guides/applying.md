---
title: Applying Overlays
sidebar:
  order: 1
---

This guide covers the different ways to apply overlays to a git repository.

## Basic usage

The simplest way to get started is to browse overlays from a GitHub username. repoverlay fetches the available overlays and lets you pick interactively:

```bash
repoverlay browse tylerbutler
```

For scripting or power-user workflows, use `apply` to apply a specific local directory, GitHub URL, or configured overlay reference directly:

```bash
# Local directory
repoverlay apply /path/to/overlay

# GitHub repository
repoverlay apply https://github.com/owner/repo
```

## Where overlays come from

repoverlay supports several source types. It determines the type automatically from what you pass to `browse` or `apply`:

- Strings starting with `https://github.com/` are treated as **GitHub URLs**
- Strings that look like filesystem paths (`./`, `/`, `~/`) are treated as **local directories**
- Three-part strings like `org/repo/name` are treated as **overlay repository references**
- Two-part strings like `owner/repo` enter **browse mode** (interactive selection)
- Single words like `tylerbutler` are treated as **GitHub usernames**

### GitHub usernames

```bash
repoverlay browse tylerbutler
```

This fetches a default overlay repository for that user, shows available overlays filtered to your current repo, and lets you pick from an interactive list. The first time you use a source, repoverlay will ask if you want to save it for future use.

### GitHub URLs

```bash
# Default branch
repoverlay apply https://github.com/owner/repo

# Specific branch or tag
repoverlay apply https://github.com/owner/repo --ref develop
repoverlay apply https://github.com/owner/repo/tree/v1.0.0

# Subdirectory within a repo
repoverlay apply https://github.com/owner/repo/tree/main/overlays/rust
```

GitHub sources are cached locally using shallow clones. Use `repoverlay update` to pull new changes later.

### Overlay repository references

If you've used a source before (or added one manually), you can reference a specific overlay by its path:

```bash
repoverlay apply org/repo/overlay-name
```

### Local directories

```bash
repoverlay apply /path/to/overlay
repoverlay apply ./relative/overlay
```

Files are symlinked directly from the source. Changes to the source are reflected immediately.

## Managing sources

When you apply from a username or `owner/repo` for the first time, repoverlay prompts you to save the source. You can also manage sources manually:

```bash
# Add a source
repoverlay source add tylerbutler

# List configured sources
repoverlay source list

# Remove a source
repoverlay source remove tylerbutler
```

Sources are checked in priority order when resolving overlay references. Earlier sources have higher priority.

Local directory sources may use the shared `org/repo/overlay-name/` layout or a flat
layout. In a flat layout, each top-level directory is an overlay; if there are no
top-level overlay directories, the source directory itself is treated as one overlay.

## Conflict handling

If an overlay file conflicts with an existing file in the repo, repoverlay fails by default. You can control this behavior:

### `--force`

Overwrite existing files:

```bash
repoverlay apply ./overlay --force
```

### `--skip-conflicts`

Skip conflicting files silently and continue with the rest:

```bash
repoverlay apply ./overlay --skip-conflicts
```

### `--interactive`

Prompt for each conflict individually:

```bash
repoverlay apply ./overlay --interactive
```

### `--merge` (JSON deep merge)

For JSON files, deep merge the overlay's content into the existing file instead of replacing it:

```bash
repoverlay apply ./overlay --merge
```

This is useful when an overlay provides default settings that should be merged with a repository's existing configuration. For example, an overlay might add recommended VS Code extensions to an existing `.vscode/settings.json`.

Deep merge combines objects recursively — overlay keys are added or updated, but existing keys not in the overlay are preserved. Merge targets must be repo-relative real files; repoverlay rejects target symlinks and symlinked parent directories instead of following them. For non-JSON files, `--merge` has no effect (the file is treated as a conflict).

:::note
`--merge` can be combined with `--force` or `--skip-conflicts`. When combined with `--force`, JSON files are merged while non-JSON conflicts are overwritten. When combined with `--skip-conflicts`, JSON files are merged while non-JSON conflicts are skipped.
:::

## Other options

### Copy mode

Use `--copy` to copy files instead of creating symlinks:

```bash
repoverlay apply ./overlay --copy
```

:::tip
Use `--copy` on Windows if your project doesn't support symlinks, or in CI environments where symlinks may not behave as expected.
:::

### Custom overlay name

repoverlay auto-generates a name from the source. Use `--name` to override it:

```bash
repoverlay apply ./overlay --name my-config
```

### Target directory

By default, repoverlay applies to the current directory. Use `--target` to apply to a different repo:

```bash
repoverlay apply ./overlay --target /path/to/repo
```

### Dry run

Preview what would happen without making changes:

```bash
repoverlay apply ./overlay --dry-run
```

## Browsing without applying

:::tip[Explore first]
Want to see what overlays are available without applying anything? Use `repoverlay browse`:

```bash
repoverlay browse tylerbutler
```

This fetches and lists available overlays from the source. You can still select and apply from the interactive list, but the source is not saved to your configuration.
:::
