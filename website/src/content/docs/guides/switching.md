---
title: Switching Overlays
sidebar:
  order: 6
---

<!-- TODO: Add use case examples for when switching is useful -->

The `switch` command atomically replaces all existing overlays with a new one. This is useful when you want to swap between different overlay configurations.

## Usage

```bash
repoverlay switch ~/overlays/typescript-ai
repoverlay switch https://github.com/user/ai-configs/tree/main/rust
```

## What happens during a switch

1. All currently applied overlays are **removed** (files, git excludes, state)
2. The new overlay is **applied** in their place

This is equivalent to running `repoverlay remove --all` followed by `repoverlay apply`, but as a single atomic operation.

## When to use switch

- Changing between language-specific overlay sets (e.g., Rust vs TypeScript configs)
- Swapping between personal and team overlay configurations
- Resetting to a known overlay state
