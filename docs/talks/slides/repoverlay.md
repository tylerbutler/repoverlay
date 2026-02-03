---
marp: true
theme: nord-theme
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

## Applying an Overlay

One command to get someone's configs:

```bash
repoverlay apply tylerbutler
```

---

## Interactive Selection

```
? Select an overlay from tylerbutler/repo-overlays:
  > claude-config
    dev-setup
    rust-tools
```

Select one, done.

---

## Demo: Verify the Apply

```bash
# Symlinks pointing to cached overlay
ls -la

# Confirms the overlay is applied
repoverlay status

# Clean - files are excluded from git
git status
```

---

## The Syntax Shorthand

repoverlay figures out what you mean:

| You type | repoverlay understands |
|----------|------------------------|
| `tylerbutler` | Browse `tylerbutler/repo-overlays` |
| `tylerbutler/my-configs` | Browse `tylerbutler/my-configs` |
| `tylerbutler/my-configs/claude` | Apply `claude` directly |

---

## Key Insight

GitHub is assumed - no URLs to type.

Convention: `username/repo-overlays` is the default repo name.

"If you know exactly what you want, be specific. If you want to browse, be vague."

---

## Day-to-Day: Status

```bash
# See all applied overlays
repoverlay status

# Details on specific overlay
repoverlay status --name claude-config
```

---

## Day-to-Day: Making Changes

Edit files normally - they're just files in your project.

When you're happy with changes:

```bash
repoverlay sync claude-config
```

Changes committed and pushed to the source repo.

---

## Why Sync?

"If they're symlinks, why do I need to sync?"

The files live in a cached copy of the overlay repo.

**Sync** commits and pushes so the git repo stays up to date.

---

## Day-to-Day: Remove

```bash
# Remove specific overlay
repoverlay remove claude-config

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

Restores from external backup in `~/.local/share/repoverlay/applied/`

---

<!-- _class: invert -->

# Creating Your Own

---

## When You're Ready to Share

Once you have configs to reuse or share, create your own overlay.

First, you need an **overlay repository**:

```
github.com/yourname/repo-overlays
```

---

## No Repo Yet? No Problem

```bash
repoverlay create claude-config
```

```
No overlay repository configured. Create one?
> Yes, create yourname/repo-overlays on GitHub
  No, I'll use a local directory
  No, let me configure it manually
```

---

## Local-Only Option

Don't want to share? Use a local path:

```bash
repoverlay create-local ~/my-overlays/claude-config
```

No GitHub, no sharing - just symlinks to a folder you control.

---

## Demo: Create an Overlay

```bash
repoverlay create claude-config
```

```
? Select files to include:
  [x] .claude/
  [x] CLAUDE.md
  [ ] .envrc
  [ ] scratch/
```

---

## What Happened?

1. Files copied to overlay repo
2. Originals replaced with symlinks
3. Auto-committed and pushed

Output shows: `yourname/repo-overlays/claude-config`

---

## Now Others Can Use It

```bash
# Your teammate runs:
repoverlay apply yourname

# Selects claude-config, done.
```

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
repoverlay remove claude-config
repoverlay apply yourname/repo-overlays/claude-config
```

Now `.claude/` itself is the symlink:

```bash
ls -la
# .claude -> /path/to/cached/overlay/.claude
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
2. **GitHub shorthand** - `username`, `owner/repo`, `owner/repo/overlay`
3. **`repo-overlays` convention** - default repo name
4. **Sync workflow** - edit normally, sync pushes changes
5. **External backup** - recovery after `git clean`
6. **Directory symlinks** - atomic directory management

---

## Quick Reference

| Task | Command |
|------|---------|
| Apply (browse) | `repoverlay apply <username>` |
| Apply (direct) | `repoverlay apply <owner/repo/overlay>` |
| Check status | `repoverlay status` |
| Sync changes | `repoverlay sync <name>` |
| Remove | `repoverlay remove <name>` |
| Restore | `repoverlay restore` |
| Create (shared) | `repoverlay create <name>` |
| Create (local) | `repoverlay create-local <path>` |

---

<!-- _class: invert -->

# Questions?

github.com/tylerbutler/repoverlay
