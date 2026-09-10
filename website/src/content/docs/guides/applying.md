---
title: Applying overlays
sidebar:
  order: 1
---

Apply overlays to a git repository from a local directory, a GitHub URL, or a configured source.

## Basic usage

To start, browse overlays from a GitHub username. repoverlay gets the available overlays and shows a selection menu:

```bash
repoverlay browse tylerbutler
```

To apply an overlay without the selection menu, specify a local directory, GitHub URL, or configured overlay reference:

```bash
# Local directory
repoverlay apply /path/to/overlay

# GitHub repository
repoverlay apply https://github.com/owner/repo
```

## Where overlays come from

repoverlay selects the source type from the value you give to `browse` or `apply`:

- A value that starts with `https://github.com/` specifies a GitHub URL.
- A file path that starts with `./`, `/`, or `~/` specifies a local directory.
- A three-part value such as `org/repo/name` specifies a configured source reference.
- A two-part value such as `owner/repo` opens a selection menu.
- A single word such as `tylerbutler` specifies a GitHub username.

### GitHub usernames

```bash
repoverlay browse tylerbutler
```

This command gets the default overlay repository for that user. It shows a selection menu with overlays for your current repository. The first time you use a source, repoverlay asks if you want to save it for future use.

A username alone expands to `username/repo-overlays`. To use a different repository name, set the `REPOVERLAY_DEFAULT_REPO_NAME` environment variable. For example, `REPOVERLAY_DEFAULT_REPO_NAME=overlays` expands `tylerbutler` to `tylerbutler/overlays`.

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

repoverlay stores GitHub sources locally as shallow clones. Use `repoverlay update` to get new changes later.

### Configured source references

If you have used a source before or added one manually, you can specify an overlay by its path:

```bash
repoverlay apply org/repo/overlay-name
```

### Local directories

```bash
repoverlay apply /path/to/overlay
repoverlay apply ./relative/overlay
```

repoverlay creates symlinks directly to the source files. Changes to the source appear immediately in the target repository.

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

repoverlay checks sources in priority order to resolve overlay references. Earlier sources have higher priority.

Local directory sources can use the shared `org/repo/overlay-name/` layout or a flat
layout. In a flat layout, each top-level directory is an overlay. If there are no
top-level overlay directories, repoverlay uses the source directory itself as one overlay.

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

Choose an action for each conflict:

```bash
repoverlay apply ./overlay --interactive
```

### `--merge` (JSON deep merge)

For JSON files, use a deep merge to combine the overlay content with the existing file:

```bash
repoverlay apply ./overlay --merge
```

Use this option to add default settings to the existing repository configuration. For example, an overlay can add recommended VS Code extensions to an existing `.vscode/settings.json`.

A deep merge combines objects recursively. It adds or updates overlay keys and keeps existing keys that are not in the overlay.

Merge targets must be real files with paths relative to the repository. repoverlay rejects symlinks in the target or its parent directories. For non-JSON files, `--merge` has no effect. repoverlay treats these files as conflicts.

:::note
You can combine `--merge` with `--force` or `--skip-conflicts`. With `--force`, repoverlay merges JSON files and overwrites non-JSON conflicts. With `--skip-conflicts`, repoverlay merges JSON files and skips non-JSON conflicts.
:::

To use merging by default, set the `REPOVERLAY_MERGE=true` environment variable. This sets the default for `--merge` in `apply`, `switch`, `restore`, and `update`.

## Other options

### Copy mode

Use `--copy` to copy files instead of creating symlinks:

```bash
repoverlay apply ./overlay --copy
```

:::tip
Use `--copy` if your Windows project does not support symlinks. Also use it in CI environments that do not handle symlinks as expected.
:::

### Custom overlay name

repoverlay generates a name from the source. Use `--name` to set a different name:

```bash
repoverlay apply ./overlay --name my-config
```

### Target directory

By default, repoverlay applies overlays to the current directory. Use `--target` to apply an overlay to a different repository:

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
To see available overlays before you apply one, use `repoverlay browse`:

```bash
repoverlay browse tylerbutler
```

This command gets and lists available overlays from the source. You can select and apply an overlay from the list. As with `apply`, repoverlay asks whether to save a new source for future use.
:::
