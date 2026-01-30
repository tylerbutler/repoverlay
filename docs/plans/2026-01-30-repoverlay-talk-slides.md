---
marp: true
theme: default
paginate: true
title: repoverlay - Config Files Everywhere
---

# repoverlay

**Apply configuration files to git repositories without committing them.**

---

## The Problem

I have Claude configs that make my projects work great:

- `.claude/` directory
- `CLAUDE.md`
- Custom commands

**But...**

---

## The Problem (cont.)

- I shouldn't commit them (personal preferences, not project config)
- I want them in all my projects
- When I improve them, I want improvements everywhere

---

## The Solution

**repoverlay** symlinks files from an external source into your repo and automatically excludes them from git.

Files appear in your project. Git ignores them. They're stored somewhere you control.

---

## Demo: The End State

```bash
# Files are present
ls .claude/

# Git doesn't track them
git status

# Peek behind the curtain - they're symlinks
ls -la

# The tool's view
repoverlay status
```

---

## Creating an Overlay

First, you need an **overlay repository** - a git repo where your overlays live.

```
github.com/yourname/overlays
```

---

## Demo: Create an Overlay

```bash
repoverlay create my-ai-config
```

Interactive selection UI appears...

- [x] `.claude/`
- [x] `CLAUDE.md`
- [ ] `.envrc`
- [ ] `scratch/`

---

## What Happened?

1. Files copied to overlay repo
2. Originals replaced with symlinks
3. Auto-committed and pushed

Output shows: `yourname/overlays/my-ai-config`

---

## Applying to Another Project

```bash
cd ~/projects/other-project

repoverlay apply yourname/overlays/my-ai-config
```

---

## Demo: Verify the Apply

```bash
# Symlinks pointing to overlay repo
ls -la

# Confirms the overlay is applied
repoverlay status

# Clean - files are excluded from git
git status
```

---

## Key Insight

Both projects now share the **same files**.

Edit in one place, it's updated everywhere.

(Because they're symlinks to the same source.)

---

## Making Changes

Edit files normally - they're just files in your project.

When you're happy with changes:

```bash
repoverlay sync my-ai-config
```

Changes committed and pushed to overlay repo.

---

## Why Sync?

"If they're symlinks, why do I need to sync?"

The files live in the overlay repo on disk.

**Sync** commits and pushes so git has the latest version.

---

## Day-to-Day: Status

```bash
# See all applied overlays
repoverlay status

# Details on specific overlay
repoverlay status --name my-ai-config
```

---

## Day-to-Day: Remove

```bash
# Remove specific overlay
repoverlay remove my-ai-config

# Interactive selection
repoverlay remove
```

Removes symlinks, cleans up git exclude.

---

## Day-to-Day: Recovery

If `git clean -fdx` wipes your overlay files:

```bash
repoverlay restore
```

Restores from external backup in `~/.local/share/repoverlay/`

---

## Commands You'll Actually Use

| Task | Command |
|------|---------|
| Check status | `repoverlay status` |
| Sync changes | `repoverlay sync <name>` |
| Restore | `repoverlay restore` |

---

<!-- _class: invert -->

# Bonus

Directory Symlinks

---

## The Problem with File Symlinks

By default, repoverlay symlinks **individual files**.

If you add a new file to `.claude/commands/` in your overlay...

...it doesn't appear in projects automatically.

---

## Solution: Directory Symlinks

In your overlay's `repoverlay.ccl`:

```
directories =
  = .claude
```

---

## Re-apply the Overlay

```bash
repoverlay remove my-ai-config
repoverlay apply yourname/overlays/my-ai-config
```

Now `.claude/` itself is the symlink:

```bash
ls -la
# .claude -> /path/to/overlay/.claude
```

New files appear automatically.

---

## About CCL

The config uses **CCL format** - like a simpler TOML.

You probably won't edit it often, but `directories` is useful.

Learn more: [ccl.tylerbutler.com](https://ccl.tylerbutler.com)

---

## Concepts Recap

1. **Symlinks + git exclude** - core mechanism
2. **Overlay repository** - where overlays live
3. **`org/repo/overlay-name`** - how to reference overlays
4. **Sync workflow** - edit normally, sync pushes changes
5. **External backup** - recovery after `git clean`
6. **Directory symlinks** - atomic directory management

---

## Quick Reference

| Task | Command |
|------|---------|
| Create overlay | `repoverlay create <name>` |
| Apply overlay | `repoverlay apply <source>` |
| Check status | `repoverlay status` |
| Sync changes | `repoverlay sync <name>` |
| Remove overlay | `repoverlay remove <name>` |
| Restore | `repoverlay restore` |

---

<!-- _class: invert -->

# Questions?

github.com/tylerbutler/repoverlay
