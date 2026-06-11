---
title: Creating & Sharing Overlays
sidebar:
  order: 2
---

Package existing files into an overlay, then share it so others can apply it.

## Creating an overlay

The `create` command packages files from your current repository into an overlay and saves them to your overlay repository:

```bash
# Auto-detect org/repo from git remote
repoverlay create my-overlay

# Explicit target path
repoverlay create microsoft/vscode/ai-config
```

### Selecting files

Use `--include` to specify which files to include:

```bash
repoverlay create my-overlay --include .claude/ --include CLAUDE.md --include .envrc
```

Without `--include`, repoverlay launches an interactive file selector that detects AI configs, gitignored files, and untracked files as candidates.

### Preview and overwrite

```bash
# See what would be created without writing anything
repoverlay create my-overlay --dry-run

# Overwrite an existing overlay
repoverlay create my-overlay --force
```

## Local output

If you don't have an overlay repository set up, or want to create an overlay in a local directory, use `--output`:

```bash
repoverlay create --output ./my-overlay
repoverlay create --output ./output --include .envrc --include .claude/
```

`create --output` performs two actions:
1. **Writes overlay files** to the specified directory
2. **Auto-applies the overlay** to your repository (symlinks replace originals, state saved, `.git/info/exclude` updated)

### Preview without applying

To see what would be created and applied without modifying your repository, use `--dry-run`:

```bash
# Preview: see what files would be created and applied
repoverlay create --output ./my-overlay --dry-run
```

This shows you the overlay contents and what would be applied, without writing files or mutating your repository.

## Overlay configuration (advanced)

:::note
Most overlays don't need a configuration file. Without one, all files in the overlay directory are symlinked with the same relative paths. Configuration is most useful for hand-authored overlays or cases where you need to remap files from their source location.
:::

Create a `repoverlay.ccl` in the root of your overlay directory to control how files are applied:

```
overlay =
  name = my-config

/= Rename files when applying
mappings =
  .envrc.template = .envrc
  vscode-settings.json = .vscode/settings.json

/= Symlink entire directories as a unit
directories =
  = .claude
  = scratch
```

### Overlay name

The `overlay.name` field sets the name used in `status`, `remove`, and other commands. If omitted, the directory name is used.

### Mappings

The `mappings` section renames files during apply. Each entry maps a source filename to a destination path. This is useful when the overlay uses different filenames than the target repo expects.

### Directories

The `directories` section lists directories to symlink (or copy) as a unit rather than walking individual files. This is important for directories like `.claude/` where the entire tree should be managed atomically.

### Configuration format

repoverlay uses [CCL (Categorical Configuration Language)](https://ccl.tylerbutler.com/) for configuration files. CCL uses `=` for key-value pairs and indentation for nesting. Lines starting with `/=` are comments.

## Overlay repository structure

An overlay repository organizes overlays by target project:

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

The structure is `<target-org>/<target-repo>/<overlay-name>/`. When someone runs `repoverlay apply org/repo/overlay-name`, repoverlay resolves the overlay from this directory structure.

## Sharing overlays

Once you've created overlays in a repository, push it to GitHub:

```bash
cd ~/my-overlays
git push origin main
```

Others can then apply your overlays using your GitHub username:

```bash
# Interactive selection
repoverlay apply tylerbutler
```

Direct three-part references use the *target* repository's org and repo plus the overlay name (`<target-org>/<target-repo>/<overlay-name>`), and resolve against configured sources — so consumers add your overlay repository as a source first:

```bash
repoverlay source add tylerbutler/my-overlays

# Applies the ai-config overlay defined for tylerbutler/tools-monorepo
repoverlay apply tylerbutler/tools-monorepo/ai-config
```

Or browse without applying:

```bash
repoverlay browse tylerbutler
```
