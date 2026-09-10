---
title: Creating and sharing overlays
sidebar:
  order: 2
---

Create an overlay from existing files. Share it so others can apply it.

## Creating an overlay

The `create` command puts files from your current repository into an overlay. It saves them to your overlay repository:

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

Without `--include`, repoverlay opens a file selection menu. It suggests AI configuration files, files that git ignores, and untracked files.

### Preview and overwrite

```bash
# See what would be created without writing anything
repoverlay create my-overlay --dry-run

# Overwrite an existing overlay
repoverlay create my-overlay --force
```

## Local output

Use `--output` to create an overlay in a local directory. You do not need an overlay repository:

```bash
repoverlay create --output ./my-overlay
repoverlay create --output ./output --include .envrc --include .claude/
```

`create --output` performs two actions:
1. It writes overlay files to the specified directory.
2. It applies the overlay to your repository. Symlinks replace the original files. repoverlay saves the state and updates `.git/info/exclude`.

### Preview without applying

To preview the overlay without changes to your repository, use `--dry-run`:

```bash
# Preview: see what files would be created and applied
repoverlay create --output ./my-overlay --dry-run
```

This command shows the overlay contents and the planned changes. It does not write files or change your repository.

## Overlay configuration (advanced)

:::note
Most overlays do not need a configuration file. Without one, repoverlay creates symlinks for all files with the same relative paths. Use configuration for overlays you create manually or to change the target paths.
:::

Create `repoverlay.ccl` in the root of your overlay directory to control how repoverlay applies files:

```
overlay =
  name = my-config
  description = Shared editor and environment config

/= Rename files when applying
mappings =
  .envrc.template = .envrc
  vscode-settings.json = .vscode/settings.json

/= Symlink entire directories as a unit
directories =
  = .claude
  = scratch
```

### Overlay name and description

The `overlay.name` field sets the name for `status`, `remove`, and other commands. If you omit it, repoverlay uses the directory name. Use the optional `overlay.description` field to explain the purpose of the overlay.

### Mappings

The `mappings` section renames files when you apply the overlay. Each entry maps a source filename to a target path. Use this when the overlay filenames differ from those the target repository needs.

One source file can map to multiple target paths. Repeat the key with a different target path each time:

```
mappings =
  config.json = .vscode/settings.json
  config.json = .zed/settings.json
```

### Directories

The `directories` section lists directories to symlink or copy as a unit instead of as individual files. Use this for directories such as `.claude/` when repoverlay must manage the full directory tree as one unit.

### Configuration format

repoverlay uses [CCL (Categorical Configuration Language)](https://ccl.tylerbutler.com/) for configuration files. CCL uses `=` for key-value pairs and indentation for nesting. Lines starting with `/=` are comments.

## Composing overlays

Overlays in the [in-repo library](/guides/library/) can reuse files from other overlays. Two keys in `repoverlay.ccl` control this:

### extends

Inherit every file from a single parent overlay:

```
extends =
  overlay = base-config
```

The overlay inherits all files, mappings, and directories from its parent. The parent can also extend another overlay. repoverlay detects cycles in this chain.

### includes

Select specific files from other overlays:

```
includes =
  =
    overlay = tools
    files =
      = .editorconfig
      = scripts/lint.sh
```

You can list several `includes` entries. Included overlays can also use `extends` or `includes`. repoverlay resolves them recursively.

### Precedence

If multiple files have the same target path, repoverlay uses this priority order:

1. The overlay's own files
2. Files from `extends`
3. Files from `includes` (later entries override earlier ones)

:::note
Only library overlays (`.repoverlay/library/`) can extend or include other overlays. An overlay applied from GitHub or a local path cannot do this. See [The in-repo library](/guides/library/) to move overlays into the library.
:::

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

## Global overlays

A **global overlay** applies to any repository, regardless of its git remote. Store global overlays in the reserved `@global/` namespace beside the project-specific `<org>/<repo>/` directories:

```
my-overlays/
├── microsoft/
│   └── FluidFramework/
│       └── claude-config/
└── @global/
    └── dotfiles/
        └── .gitconfig
```

To create one, use `--global` with the overlay name alone. This skips git remote detection:

```bash
repoverlay create dotfiles --global
```

`repoverlay browse` lists global overlays for every repository under the **Global** heading. It shows each overlay as `*/<name>`. Apply it with the name alone:

```bash
repoverlay apply dotfiles
```

If a global overlay and a repository-specific overlay have the same name, repoverlay prefers the repository-specific overlay within that source. It checks `org/repo/name` before `@global/name`. It still checks sources in priority order.

You can apply any overlay to any repository. repoverlay checks file paths and conflicts, not the repository for which you created the overlay. The `@global` namespace marks an overlay for use in all repositories. It lets repoverlay find the overlay without an `org/repo` match.

:::caution[Minimum version]
Global overlays require the reserved `@global` namespace. repoverlay **0.16.0** and earlier cannot resolve global overlays. These versions either do not recognize `@global` or skip it safely. repoverlay **0.17.0** is the first release that can read sources with global overlays and create and apply them. Make sure that everyone who shares a source uses 0.17.0 or later.
:::

## Sharing overlays

After you create overlays in a repository, push the repository to GitHub:

```bash
cd ~/my-overlays
git push origin main
```

Others can then apply your overlays using your GitHub username:

```bash
# Interactive selection
repoverlay apply tylerbutler
```

A three-part reference uses the target organization, target repository, and overlay name: `<target-org>/<target-repo>/<overlay-name>`. repoverlay resolves this reference against configured sources. Users must first add your overlay repository as a source:

```bash
repoverlay source add tylerbutler/my-overlays

# Applies the ai-config overlay defined for tylerbutler/tools-monorepo
repoverlay apply tylerbutler/tools-monorepo/ai-config
```

Or browse without applying:

```bash
repoverlay browse tylerbutler
```
