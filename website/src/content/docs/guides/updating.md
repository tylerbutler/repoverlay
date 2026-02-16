---
title: Updating Remote Overlays
sidebar:
  order: 4
---

<!-- TODO: Explain what happens during an update in more detail -->

When overlays come from GitHub, repoverlay can pull the latest changes and re-apply them.

## Updating all overlays

```bash
repoverlay update
```

This checks each GitHub-sourced overlay for new commits, pulls updates, and re-applies changed files.

## Updating a specific overlay

```bash
repoverlay update my-overlay
```

## Preview changes

See what would be updated without making changes:

```bash
repoverlay update --dry-run
```

## When to update

- After the overlay source has been updated on GitHub
- When you want to pick up config changes shared by your team
- Periodically, to stay in sync with upstream overlay changes

Local overlays don't need updating — symlinks already reflect the latest source files.
