---
title: Overlay Repositories
sidebar:
  order: 3
---

<!-- TODO: Explain the full overlay repository structure and setup -->

An **overlay repository** is a GitHub repository that contains multiple named overlays organized by project. This lets you maintain a central collection of overlays that can be applied to any repo with a short reference like `org/repo/overlay-name`.

## Repository structure

An overlay repository is organized by org, repo, and overlay name:

```
my-overlays/
├── microsoft/
│   └── FluidFramework/
│       ├── claude-config/
│       │   ├── CLAUDE.md
│       │   └── .claude/
│       └── dev-tools/
│           └── .envrc
└── tylerbutler/
    └── tools-monorepo/
        └── ai-config/
            └── CLAUDE.md
```

## Using overlay repositories

<!-- TODO: Document the source add/remove/list commands -->

Once configured, reference overlays by their path within the repository:

```bash
repoverlay apply microsoft/FluidFramework/claude-config
```

## Setting up a shared repository

<!-- TODO: Step-by-step guide for creating and configuring an overlay repo -->

*This section is coming soon.*
