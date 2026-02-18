# repoverlay

[![CI](https://github.com/tylerbutler/repoverlay/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/tylerbutler/repoverlay/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/repoverlay)](https://crates.io/crates/repoverlay)
[![codecov](https://codecov.io/gh/tylerbutler/repoverlay/graph/badge.svg)](https://codecov.io/gh/tylerbutler/repoverlay)
[![MIT License](https://img.shields.io/crates/l/repoverlay)](LICENSE)

Overlay config files into git repositories without committing them. Files are symlinked (or copied with `--copy`) from overlay sources and automatically excluded via `.git/info/exclude`.

## Quick Reference

| Task | Command |
|------|---------|
| Apply overlay | `repoverlay apply <source>` |
| Check status | `repoverlay status` |
| Remove overlay | `repoverlay remove <name>` |
| Remove all | `repoverlay remove --all` |
| Update from remote | `repoverlay update` |
| Restore after git clean | `repoverlay restore` |
| Create overlay | `repoverlay create <name>` |
| Create local overlay | `repoverlay create --output <path>` |
| Add files to overlay | `repoverlay add <name> <files>` |
| Sync changes back | `repoverlay sync <name>` |
| Switch overlays | `repoverlay switch <source>` |
| Browse available overlays | `repoverlay browse` |

## Concepts

repoverlay manages four kinds of objects:

- **Overlay** — a set of config files applied to a repo. Lifecycle: `create` → `apply` → `update` → `remove`.
- **Source** — a configured location (GitHub repo) to find overlays. Lifecycle: `source add` → `source list` → `source remove`.
- **Cache** — local clones of GitHub repos used by overlays. Managed automatically on `apply`; inspect with `cache list`, clean with `cache remove --all`.
- **File** — an individual file within an overlay. Managed via `add` and `sync`.

## Installation

### Homebrew (macOS/Linux)

```bash
brew install tylerbutler/tap/repoverlay
```

### Shell installer (macOS/Linux)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tylerbutler/repoverlay/releases/latest/download/repoverlay-installer.sh | sh
```

### PowerShell installer (Windows)

```powershell
irm https://github.com/tylerbutler/repoverlay/releases/latest/download/repoverlay-installer.ps1 | iex
```

### Cargo

```bash
cargo binstall repoverlay  # pre-built binary
cargo install repoverlay   # build from source
```

## Usage

### Apply an overlay

```bash
# From local directory
repoverlay apply /path/to/overlay

# From GitHub (default branch, specific ref, or subdirectory)
repoverlay apply https://github.com/owner/repo
repoverlay apply https://github.com/owner/repo/tree/v1.0.0
repoverlay apply https://github.com/owner/repo --ref develop
repoverlay apply https://github.com/owner/repo/tree/main/overlays/rust

# From overlay repository
repoverlay apply org/repo/overlay-name

# Options
repoverlay apply ./overlay --target /path/to/repo  # Apply to specific directory
repoverlay apply ./overlay --copy                   # Copy instead of symlink
repoverlay apply ./overlay --name my-config         # Custom overlay name
```

### Remove overlays

```bash
repoverlay remove              # Interactive selection
repoverlay remove my-overlay   # Remove specific overlay
repoverlay remove --all        # Remove all overlays
```

### Check status

```bash
repoverlay status                  # All applied overlays
repoverlay status --name my-overlay # Specific overlay
```

### Update remote overlays

```bash
repoverlay update              # Pull and apply updates
repoverlay update --dry-run    # Preview changes
repoverlay update my-overlay   # Update specific overlay
```

### Restore after git clean

```bash
repoverlay restore             # Re-apply overlays from backup
repoverlay restore --dry-run   # Preview what would be restored
```

### Create overlays

```bash
# Detect org/repo from git remote
repoverlay create my-overlay

# Explicit path
repoverlay create microsoft/vscode/ai-config

# Include specific files
repoverlay create my-overlay --include .claude/ --include CLAUDE.md

# Local output (no overlay repo)
repoverlay create --output ./output --include .envrc

# Preview / overwrite
repoverlay create my-overlay --dry-run
repoverlay create my-overlay --force
```

### Add files to an existing overlay

```bash
repoverlay add my-overlay newfile.txt              # Single file
repoverlay add my-overlay file1.txt file2.txt      # Multiple files
repoverlay add my-overlay config.json --dry-run    # Preview
```

Copies files to the overlay source, replaces originals with symlinks, and auto-commits/pushes.

### Sync changes back

```bash
repoverlay sync my-overlay          # Sync modified files back to source
repoverlay sync my-overlay --dry-run
```

### Switch overlays

Replace all existing overlays with a new one:

```bash
repoverlay switch ~/overlays/typescript-ai
repoverlay switch https://github.com/user/ai-configs/tree/main/rust
```

### Manage cache

```bash
repoverlay cache list              # List cached repositories
repoverlay cache path              # Show cache location
repoverlay cache remove owner/repo # Remove specific cached repo
repoverlay cache remove --all      # Remove all cached repos
```

## Overlay Configuration

Create a `repoverlay.ccl` in your overlay directory:

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

**`mappings`** — Rename files during apply (source = destination).

**`directories`** — Directories to symlink (or copy) as a unit rather than walking individual files. Useful for directories like `.claude/` that should be managed atomically.

Without a config file, all files are symlinked with the same relative path.

## License

MIT
