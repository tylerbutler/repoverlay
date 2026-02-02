# repoverlay Talk Outline

A conversational walkthrough for developers who want consistent AI configs across their projects.

**Format:** Informal walkthrough, ~15-20 minutes
**Audience:** Developers familiar with git basics, briefly frame the problem before diving in
**Golden path:** Full lifecycle with an overlay repository - create, apply, sync, manage

---

## Section 1: Problem & End State (2-3 min)

### The Problem

"I have a project with Claude configs - `.claude/` directory, `CLAUDE.md`, maybe some custom commands. These files make Claude Code work great for this project. But:
- I shouldn't commit them (they're personal preferences, not project config)
- I want them in my other projects too
- When I improve them, I want the improvements everywhere"

### The Core Concept

repoverlay symlinks files from an external source into your repo and automatically excludes them from git. That's it. Files appear in your project, git ignores them, and they're actually stored somewhere you control.

### Demo: Show the End State

Start in a repo that already has an overlay applied.

```bash
# Files are present
ls .claude/

# Git doesn't track them
git status

# Peek behind the curtain - they're symlinks
ls -la

# The tool's view - overlay name, source, managed files
repoverlay status
```

**Key point:** Show the destination before explaining how to get there.

---

## Section 2: Create Your First Overlay (3-5 min)

### Setup Requirement

You need an **overlay repository** - a git repo where your overlays live. This could be `github.com/yourname/overlays` or similar.

### Demo: Interactive Overlay Creation

Start in a project that has `.claude/` and `CLAUDE.md` (not yet managed by repoverlay).

```bash
repoverlay create my-ai-config
```

1. **Interactive selection UI appears** - checkbox list of discovered files (AI configs, gitignored files, untracked files)
2. Select `.claude/` and `CLAUDE.md` using the checkbox interface
3. Confirm

**What happened:**
- Files copied to overlay repo
- Originals replaced with symlinks
- Auto-committed and pushed to overlay repo

**Output shows:** The `org/repo/overlay-name` path where the overlay now lives.

### Concepts Introduced

- Overlay repository ("a git repo where your overlays live")
- The `org/repo/overlay-name` naming convention

---

## Section 3: Apply to Another Project (2-3 min)

### Demo: Apply the Overlay

```bash
# Move to a different project (no AI configs yet)
cd ~/projects/other-project

# Apply using the short-form syntax
repoverlay apply yourname/overlays/my-ai-config
```

**Show the results:**

```bash
# Symlinks pointing to overlay repo
ls -la

# Confirms the overlay is applied
repoverlay status

# Clean - files are excluded from git
git status
```

### Key Point

"Both projects now share the same files. Edit in one place, it's updated everywhere - because they're symlinks to the same source."

### Concepts Introduced

- The short-form `org/repo/overlay-name` syntax
- Git exclude (briefly: "repoverlay automatically adds these to `.git/info/exclude` so git ignores them")

### If Asked

Why `.git/info/exclude` instead of `.gitignore`? The exclude file is local to your clone; `.gitignore` is tracked and would affect everyone.

---

## Section 4: Making Changes and Syncing Back (2-3 min)

### Demo: Edit and Sync

```bash
# Make a change to an overlay file
# (e.g., edit CLAUDE.md or add a command to .claude/commands/)

# Sync changes back to the overlay repo
repoverlay sync my-ai-config
```

**Output shows:** Changes detected, committed, pushed to overlay repo.

**Optional:** `cd` to the other project, show the change is already there (because symlinks).

### Key Point

"You edit files normally - they're just files in your project. When you're happy with changes, `sync` pushes them back to the overlay repo. Since everything is symlinked, other projects already have the update."

### Potential Confusion

"Wait, if they're symlinks, why do I need to sync?"

The files live in the overlay repo on disk. Sync commits and pushes those changes so the git repo stays up to date (and so GitHub has the latest version).

---

## Section 5: Day-to-Day Management (2-3 min)

### Status Check

```bash
# See all applied overlays
repoverlay status

# Details on specific overlay
repoverlay status --name my-ai-config
```

Shows: source, files managed, when applied.

### Removing an Overlay

```bash
# Remove specific overlay
repoverlay remove my-ai-config

# Interactive selection if multiple overlays
repoverlay remove
```

Removes symlinks, cleans up git exclude.

### Recovery After `git clean`

"If you ever run `git clean -fdx` and your overlay files disappear..."

```bash
repoverlay restore
```

Restores from external backup.

**Brief explanation:** repoverlay keeps a backup of what's applied in `~/.local/share/repoverlay/` so it can recover.

### Key Point

"These are the commands you'll use day-to-day: `status` to see what's applied, `sync` to push changes, and occasionally `restore` if something gets cleaned up."

---

## Bonus: Directory Symlinks via Config

### The Problem

By default, repoverlay walks directories and symlinks individual files. But sometimes you want the whole directory as one symlink.

"If I add a new file to `.claude/commands/` in my overlay, it doesn't automatically appear in projects - because each file is symlinked individually."

### Demo: Configure Directory Symlinks

Open `repoverlay.ccl` in the overlay repo (first time seeing this file):

```
directories =
  = .claude
```

Re-apply the overlay:

```bash
repoverlay remove my-ai-config
repoverlay apply yourname/overlays/my-ai-config
```

Now `.claude/` itself is the symlink:

```bash
ls -la
# .claude -> /path/to/overlay/repo/.claude
```

### Key Point

"This is the one config option worth knowing. Directory symlinks mean the overlay stays in sync without re-applying."

### CCL Format

"The config uses CCL format - it's like a simpler TOML. You probably won't need to edit it often, but `directories` is the useful one. If you're curious about CCL, check out [ccl.tylerbutler.com](https://ccl.tylerbutler.com)."

---

## Concepts Reference

### Introduced (in order)

1. **Symlinks + git exclude** - core mechanism
2. **Overlay repository** - where overlays live
3. **`org/repo/overlay-name` naming** - how to reference overlays
4. **Sync workflow** - edit files normally, sync pushes to overlay repo
5. **External backup** - enables recovery after `git clean`
6. **Directory symlinks** - config option for atomic directory management

### Intentionally Minimized

- CCL config format details (just show `directories`)
- State file internals (`.repoverlay/` exists, don't explain)
- Copy mode (`--copy`)
- GitHub URL sources (mention "also works" if asked)
- Fork inheritance
- Cache management

---

## Command Quick Reference

| Task | Command |
|------|---------|
| Create overlay | `repoverlay create <name>` |
| Apply overlay | `repoverlay apply <source>` |
| Check status | `repoverlay status` |
| Sync changes | `repoverlay sync <name>` |
| Remove overlay | `repoverlay remove <name>` |
| Restore after clean | `repoverlay restore` |
