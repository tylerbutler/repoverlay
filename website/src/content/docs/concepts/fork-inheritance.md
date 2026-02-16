---
title: Fork Inheritance
sidebar:
  order: 4
---

<!-- TODO: Add a concrete walkthrough with a real-world example -->

When you work on a **fork** of a repository, repoverlay can automatically inherit overlays from the **upstream** (parent) repository. This means you don't need to duplicate overlay configurations for every fork.

## How it works

When you apply an overlay from a shared overlay repository, repoverlay follows this resolution order:

1. **Direct match** — look for an overlay matching your fork's `org/repo`
2. **Upstream fallback** — if no direct match exists and an upstream remote is configured, look for an overlay matching the upstream's `org/repo`

## Example

Suppose you've forked `microsoft/FluidFramework` to `tylerbutler/FluidFramework`:

```bash
# Your fork's remotes
git remote -v
# origin    git@github.com:tylerbutler/FluidFramework.git
# upstream  git@github.com:microsoft/FluidFramework.git

# This will first check for tylerbutler/FluidFramework/claude-config,
# then fall back to microsoft/FluidFramework/claude-config
repoverlay apply microsoft/FluidFramework/claude-config
```

## Upstream detection

repoverlay detects the upstream repository by scanning git remotes for one named `upstream` — the standard convention for forks. Both HTTPS and SSH remote URLs are supported.

## Status display

When an overlay is resolved via upstream fallback, `repoverlay status` shows how it was resolved:

```
Overlay: claude-config
  Source:  microsoft/FluidFramework/claude-config (via upstream) (overlay repo)
  Commit:  abc123def456
```
