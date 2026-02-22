---
title: Managing Applied Overlays
sidebar:
  order: 3
---

This guide covers how to check, edit, update, and remove overlays after they've been applied.

## Checking status

See what overlays are currently applied:

```bash
repoverlay status
```

Check a specific overlay:

```bash
repoverlay status --name my-overlay
```

Status shows each overlay's name, source, and the files it manages.

## Editing an overlay

The `edit` command lets you add or remove files from an applied overlay.

### Add files

```bash
repoverlay edit my-overlay --add newfile.txt
repoverlay edit my-overlay --add file1.txt --add file2.txt
```

This copies the files to the overlay source, replaces the originals with symlinks, and updates the overlay state.

### Remove files

```bash
repoverlay edit my-overlay --remove oldfile.txt
```

### Interactive re-selection

Re-run the interactive file selector with current files pre-selected:

```bash
repoverlay edit my-overlay --interactive
```

### Preview changes

```bash
repoverlay edit my-overlay --add new.txt --dry-run
```

## Syncing changes back

If you've modified overlay files in your repo (e.g., edited a symlinked config), the `sync` command copies those changes back to the overlay source:

```bash
repoverlay sync my-overlay
```

Preview what would be synced:

```bash
repoverlay sync my-overlay --dry-run
```

This is useful when you've tweaked a config in one repo and want to propagate the change to all repos using that overlay.

## Updating remote overlays

When overlays come from GitHub, repoverlay can pull the latest changes and re-apply them:

```bash
# Update all GitHub-sourced overlays
repoverlay update

# Update a specific overlay
repoverlay update my-overlay

# Preview changes
repoverlay update --dry-run
```

:::note
Local overlays don't need updating — symlinks already point to the source files, so changes are reflected immediately.
:::

### When to update

- After the overlay source has been updated on GitHub
- When you want to pick up config changes shared by your team
- Periodically, to stay in sync with upstream overlay changes

## Removing overlays

```bash
# Remove a specific overlay
repoverlay remove my-overlay

# Remove all applied overlays
repoverlay remove --all

# Interactive selection
repoverlay remove --interactive

# Preview what would be removed
repoverlay remove my-overlay --dry-run
```

Removing an overlay deletes its symlinks (or copies), cleans up the git exclude entries, and removes the state files.

## Switching overlays

The `switch` command atomically replaces all existing overlays with a new one:

```bash
repoverlay switch ~/overlays/typescript-ai
repoverlay switch https://github.com/user/ai-configs/tree/main/rust
```

:::caution
Switch removes **all** existing overlays before applying the new one. If you want to keep some overlays and add another, use `repoverlay apply` instead.
:::

This is equivalent to running `repoverlay remove --all` followed by `repoverlay apply`, but as a single atomic operation.

### When to use switch

- Changing between language-specific overlay sets (e.g., Rust vs TypeScript configs)
- Swapping between personal and team overlay configurations
- Resetting to a known overlay state
