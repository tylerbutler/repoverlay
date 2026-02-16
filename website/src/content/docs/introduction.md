---
title: What is repoverlay?
---

repoverlay is a command-line tool that **overlays config files into git repositories without committing them**. Files are symlinked (or copied) from overlay sources and automatically excluded from git tracking.

## The problem

Many development workflows require configuration files that shouldn't be committed to a repository:

- **AI assistant configs** (`.claude/`, `CLAUDE.md`, `.cursor/`) — personal preferences that vary by developer
- **Editor settings** (`.vscode/settings.json`, `.idea/`) — team members use different editors
- **Environment files** (`.envrc`, `.env.local`) — machine-specific paths and secrets
- **Dev tooling** (`.prettierrc`, `biome.json`) — standards you apply across multiple repos

You could copy these files manually and add them to `.gitignore`, but then you have to keep them updated across dozens of repositories. And if someone runs `git clean`, they're gone.

## How repoverlay helps

repoverlay lets you **define overlay sources** — local directories or GitHub repos — and apply them to any repository with a single command. Applied files are:

- **Symlinked** (or copied) into the target repo
- **Excluded from git** via `.git/info/exclude` (not `.gitignore`)
- **Tracked in state files** so they can be removed, restored, or updated
- **Backed up externally** so `git clean` doesn't destroy them

## Key features

- **Multiple sources** — apply overlays from local directories, GitHub URLs, or shared overlay repositories
- **Fork inheritance** — overlays for upstream repos automatically apply to your forks
- **Update & restore** — pull remote changes with `repoverlay update`; recover after `git clean` with `repoverlay restore`
- **Create & share** — package your configs into overlays and share them via GitHub
- **Overlay configuration** — rename files, symlink directories atomically, and more via `repoverlay.ccl`
