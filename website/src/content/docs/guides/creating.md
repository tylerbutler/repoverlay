---
title: Creating & Sharing Overlays
sidebar:
  order: 2
---

<!-- TODO: Walk through a complete example from creation to sharing -->

This guide covers how to create overlays from existing files and share them with others.

## Creating an overlay

The `create` command packages files from your current repository into an overlay:

```bash
# Auto-detect org/repo from git remote
repoverlay create my-overlay

# Explicit path
repoverlay create microsoft/vscode/ai-config
```

## Selecting files

Use `--include` to specify which files to include:

```bash
repoverlay create my-overlay --include .claude/ --include CLAUDE.md --include .envrc
```

Without `--include`, repoverlay launches an interactive file selector that detects AI configs, gitignored files, and untracked files as candidates.

## Local output

Create an overlay in a local directory instead of pushing to an overlay repository:

```bash
repoverlay create --local ./my-overlay --include .envrc --include .claude/
```

## Preview changes

Use `--dry-run` to see what would be created without writing anything:

```bash
repoverlay create my-overlay --dry-run
```

## Overwriting an existing overlay

Use `--force` to overwrite an existing overlay:

```bash
repoverlay create my-overlay --force
```

## Sharing overlays

<!-- TODO: Document the full sharing workflow -->

Once created, overlays can be shared by pushing the overlay repository to GitHub. Others can then apply your overlays using the overlay repository reference syntax:

```bash
repoverlay apply org/repo/overlay-name
```
