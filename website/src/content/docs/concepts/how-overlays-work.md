---
title: How Overlays Work
sidebar:
  order: 1
---

<!-- TODO: Flesh out with diagrams and examples -->

An overlay is a collection of files that get applied on top of a git repository without being committed. This page explains the mechanics behind how repoverlay manages overlay files.

## Symlinks vs copies

By default, repoverlay creates **symlinks** from the target repo to the overlay source. This means changes to the source are immediately reflected in the target.

Use the `--copy` flag to copy files instead, which is useful when symlinks aren't supported (e.g., some Docker setups or Windows without developer mode).

## Git exclusion

When files are applied, repoverlay adds them to `.git/info/exclude` — a per-repo gitignore file that isn't tracked by git. This means:

- Overlay files don't show up in `git status`
- No changes to the tracked `.gitignore` file
- Each overlay gets its own named section for clean removal

## State tracking

repoverlay tracks applied overlays in two locations:

- **In-repo state** (`.repoverlay/overlays/<name>.ccl`) — primary record of what's applied
- **External backup** (`~/.local/share/repoverlay/applied/`) — recovery copy in case `git clean` removes the in-repo state

## The apply/remove lifecycle

<!-- TODO: Add a diagram showing the full lifecycle -->

1. **Apply** — resolve source, create symlinks/copies, update git exclude, save state
2. **Status** — read state files, display applied overlays and their sources
3. **Update** — check for remote changes, re-apply if updated (GitHub sources only)
4. **Restore** — re-apply from external backup after `git clean`
5. **Remove** — delete symlinks/copies, clean git exclude, remove state files
