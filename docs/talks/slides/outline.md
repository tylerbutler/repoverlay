---
marp: true
theme: darcula
paginate: true
title: repoverlay Talk Outline
---

# repoverlay Talk Outline

A conversational walkthrough for developers who want consistent AI configs across their projects.

**Format:** Informal walkthrough, ~15-20 minutes
**Audience:** Developers familiar with git basics, briefly frame the problem before diving in
**Golden path:** Apply-first workflow - use existing overlays, then learn to create your own

---

## Section 1: Problem & End State (2-3 min)

### The Problem

"I work across multiple projects - maybe my own repos, maybe forks of open source projects. I've got Claude configs that make my workflow great:
- `.claude/` directory with custom commands
- `CLAUDE.md` with project context
- Maybe some local scratch files

But these are *my* preferences, not project config. I shouldn't commit them. And I want them everywhere."

### The Core Concept

repoverlay symlinks files from an external source into your repo and automatically excludes them from git. Files appear in your project, git ignores them, and they're actually stored somewhere you control.

### Demo: Show the End State

Start in a repo that already has an overlay applied.

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

**Key point:** Show the destination before explaining how to get there.

**Demo tip:** Most commands support `--dry-run` to preview changes without applying them.

---

## Section 2: Apply an Existing Overlay (3-4 min)

### The Simple Case

"Let's say a teammate already has overlays set up, or you've found some shared configs online. Getting them is one command."

```bash
# Just the username - assumes tylerbutler/repo-overlays
repoverlay apply tylerbutler
```

Interactive selection appears:
```
? Select an overlay from tylerbutler/repo-overlays:
  > claude-config
    dev-setup
    rust-tools
```

Select `claude-config`, done.

### Show the Results

```bash
# Symlinks pointing to cached overlay
ls -la

# Confirms the overlay is applied
repoverlay status

# Clean - files are excluded from git
git status
```

### The Syntax Shorthand

"repoverlay figures out what you mean based on how specific you are:"

| You type | repoverlay understands |
|----------|------------------------|
| `tylerbutler` | Browse `tylerbutler/repo-overlays` on GitHub |
| `tylerbutler/my-configs` | Browse `tylerbutler/my-configs` on GitHub |
| `tylerbutler/my-configs/claude` | Apply `claude` directly |

"If you know exactly what you want, be specific. If you want to browse, be vague."

### Key Points

- GitHub is assumed - no URLs to type
- Convention: `username/repo-overlays` is the default repo name
- You can also use full GitHub URLs or local paths if needed

---

## Section 3: Day-to-Day Usage (3-4 min)

### Checking Status

```bash
# See all applied overlays
repoverlay status

# Details on specific overlay
repoverlay status --name claude-config
```

Shows: source, files managed, when applied.

### Making Changes

"You edit files normally - they're just files in your project. When you're happy with changes, `sync` pushes them back to the source."

```bash
# Edit CLAUDE.md or add a command to .claude/commands/
# Then sync changes back
repoverlay sync claude-config
```

**Output shows:** Changes detected, committed, pushed.

### Potential Confusion

"Wait, if they're symlinks, why do I need to sync?"

The files live in a cached copy of the overlay repo. Sync commits and pushes those changes so the git repo stays up to date (and so others get your improvements).

### Removing an Overlay

```bash
# Remove specific overlay
repoverlay remove claude-config

# Interactive selection if you have multiple
repoverlay remove
```

Removes symlinks, cleans up git exclude.

### Recovery After `git clean`

"If you ever run `git clean -fdx` and your overlay files disappear..."

```bash
repoverlay restore
```

Restores from external backup in `~/.local/share/repoverlay/applied/`.

### Key Point

"These are the commands you'll use most: `status` to see what's applied, `sync` to push changes, and occasionally `restore` if something gets cleaned up."

---

## Section 4: Create Your Own Overlay (3-4 min)

### When You're Ready to Share (Or Just Reuse)

"Once you've got configs you want to reuse across projects - or share with your team - you can create your own overlay."

### Setup: The Overlay Repository

"If you want to share overlays via GitHub, create a repo to store them. The convention is `repo-overlays`:"

```
github.com/yourname/repo-overlays
```

"If you don't have one yet, repoverlay can set it up:"

```bash
repoverlay create claude-config
# No overlay repository configured. Create one?
# > Yes, create yourname/repo-overlays on GitHub
#   No, I'll use a local directory
#   No, let me configure it manually
```

Selecting "Yes" runs `gh repo create yourname/repo-overlays --private` and configures it automatically.

### Local-Only Option

"If you don't want to share at all - just reuse configs across your own machines - use a local path:"

```bash
repoverlay create-local ~/my-overlays/claude-config
```

This creates the overlay in a local directory. No GitHub, no sharing - just symlinks to a folder you control.

### Demo: Create an Overlay

Start in a project that has `.claude/` and `CLAUDE.md`.

```bash
repoverlay create claude-config
```

Interactive selection UI appears - checkbox list of discovered files:
```
? Select files to include:
  [x] .claude/
  [x] CLAUDE.md
  [ ] .envrc
  [ ] scratch/
```

Confirm selection.

### What Happened

1. Files copied to your overlay repo (under `yourname/repo-overlays/claude-config/`)
2. Originals replaced with symlinks
3. Auto-committed and pushed to GitHub

**Output shows:** The full path where the overlay now lives.

### Now Others Can Use It

```bash
# Your teammate runs:
repoverlay apply yourname

# Selects claude-config, done.
```

### Key Point

"Creating is the advanced flow - most people start by applying existing overlays. But when you're ready, it's just `create` with a name, or `create-local` if you want to keep things private."

---

## Bonus: Directory Symlinks via Config (2 min)

### The Problem

"By default, repoverlay walks directories and symlinks individual files. But sometimes you want the whole directory as one symlink."

"If I add a new file to `.claude/commands/` in my overlay, it doesn't automatically appear in projects - because each file is symlinked individually."

### Solution: Configure Directory Symlinks

Open `repoverlay.ccl` in the overlay directory:

```
directories =
  = .claude
```

Re-apply the overlay:

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

### Key Point

"This is the one config option worth knowing. Directory symlinks mean new files in the overlay appear everywhere without re-applying."

### CCL Format

"The config uses CCL format - like a simpler TOML. You probably won't edit it often, but `directories` is the useful one."

Learn more: [ccl.tylerbutler.com](https://ccl.tylerbutler.com)

---

## Concepts Recap

### Introduced (in order)

1. **Symlinks + git exclude** - core mechanism
2. **GitHub shorthand** - `username`, `owner/repo`, `owner/repo/overlay`
3. **`repo-overlays` convention** - default repo name for single-username form
4. **Sync workflow** - edit files normally, sync pushes to source
5. **External backup** - enables recovery after `git clean`
6. **Overlay repository** - where your overlays live (GitHub or local)
7. **Directory symlinks** - config option for atomic directory management

### Intentionally Minimized

- Full URL syntax (mention "also works" if asked)
- `--copy` mode
- Multi-source configuration
- Fork inheritance / upstream resolution
- Cache management commands

---

## Command Quick Reference

| Task | Command |
|------|---------|
| Apply overlay (browse) | `repoverlay apply <username>` |
| Apply overlay (direct) | `repoverlay apply <owner/repo/overlay>` |
| Check status | `repoverlay status` |
| Sync changes | `repoverlay sync <name>` |
| Remove overlay | `repoverlay remove <name>` |
| Restore after clean | `repoverlay restore` |
| Create overlay (shared) | `repoverlay create <name>` |
| Create overlay (local) | `repoverlay create-local <path>` |
