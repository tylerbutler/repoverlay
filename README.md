# repoverlay

Apply configuration files to git repositories without committing them.

Files are symlinked (or copied with `--copy`) from overlay sources and automatically excluded from git tracking via `.git/info/exclude`.

## Quick Reference

| Task | Command |
|------|---------|
| Apply overlay | `repoverlay apply <source>` |
| Check status | `repoverlay status` |
| Remove overlay | `repoverlay remove <name>` |
| Remove all | `repoverlay remove --all` |
| Update from GitHub | `repoverlay update` |
| Restore after git clean | `repoverlay restore` |
| Create overlay | `repoverlay create <name>` |
| Sync changes back | `repoverlay sync <name>` |
| Switch overlays | `repoverlay switch <source>` |

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

### Cargo binstall

```bash
cargo binstall repoverlay
```

### Cargo

```bash
cargo install repoverlay
```

### From source

```bash
git clone https://github.com/tylerbutler/repoverlay
cd repoverlay
cargo install --path .
```

## Usage

### Apply an overlay

```bash
# From local directory
repoverlay apply /path/to/overlay

# From GitHub (uses default branch)
repoverlay apply https://github.com/owner/repo

# From GitHub with specific branch/tag
repoverlay apply https://github.com/owner/repo/tree/v1.0.0
repoverlay apply https://github.com/owner/repo --ref develop

# From a subdirectory within a repo
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
repoverlay remove              # Interactive (lists applied overlays)
repoverlay remove my-overlay   # Remove specific overlay
repoverlay remove --all        # Remove all overlays
```

### Check status

```bash
repoverlay status                  # Show all applied overlays
repoverlay status --name my-overlay # Show specific overlay
```

### Update GitHub overlays

```bash
repoverlay update              # Check and apply updates to all GitHub overlays
repoverlay update --dry-run    # Check without applying
repoverlay update my-overlay   # Update specific overlay
```

### Restore after git clean

```bash
repoverlay restore             # Restore overlays from external backup
repoverlay restore --dry-run   # Preview what would be restored
```

### Create overlays

Create overlays and store them in the overlay repository:

```bash
# Short form: detect org/repo from git remote
repoverlay create my-overlay                    # Creates org/repo/my-overlay

# Explicit form: specify full path
repoverlay create microsoft/vscode/ai-config   # Creates microsoft/vscode/ai-config

# Include specific files
repoverlay create my-overlay --include .claude/ --include CLAUDE.md

# Local output (no overlay repo)
repoverlay create --local ./output --include .envrc

# Preview what would be created
repoverlay create my-overlay --dry-run

# Overwrite existing overlay
repoverlay create my-overlay --force
```

### Sync changes back

After modifying files in an applied overlay, sync changes back to the overlay repo:

```bash
repoverlay sync my-overlay          # Sync changes from applied overlay
repoverlay sync org/repo/my-overlay # Explicit path
repoverlay sync my-overlay --dry-run # Preview what would be synced
```

Both `create` and `sync` automatically commit and push to the remote overlay repo.

### Switch overlays

Replace all existing overlays with a new one:

```bash
repoverlay switch ~/overlays/typescript-ai
repoverlay switch https://github.com/user/ai-configs/tree/main/rust
repoverlay switch ~/overlays/new-config --name my-config
```

### Manage cache

```bash
repoverlay cache list           # List cached repositories
repoverlay cache path           # Show cache location
repoverlay cache clear          # Clear entire cache
repoverlay cache remove owner/repo  # Remove specific cached repo
```

## Overlay Configuration

Create a `repoverlay.ccl` in your overlay directory to configure it:

```
overlay =
  name = my-config

mappings =
  /= Rename files when applying
  .envrc.template = .envrc
  vscode-settings.json = .vscode/settings.json
```

Without a config file, all files in the overlay directory are symlinked with the same relative path.

## License

MIT
