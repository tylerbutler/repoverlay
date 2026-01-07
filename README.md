# repoverlay

Overlay config files into git repositories without committing them.

repoverlay creates symlinks (or copies) of configuration files from an overlay source into your git repository, automatically excluding them from version control. This lets you maintain personal configuration (editor settings, environment files, local scripts) that stays out of the project history.

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
cargo install repoverlay
```

### From source

```bash
git clone https://github.com/tylerbutler/repoverlay
cd repoverlay
cargo install --path .
```

## Quick Start

```bash
# Apply a local overlay directory
repoverlay apply ./my-overlay

# Apply from a GitHub repository
repoverlay apply https://github.com/owner/repo

# Apply from a subdirectory of a GitHub repo
repoverlay apply https://github.com/owner/repo/tree/main/overlays/rust

# Check status
repoverlay status

# Remove an overlay
repoverlay remove my-overlay
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

# Apply to a specific target directory
repoverlay apply ./overlay --target /path/to/repo

# Use copy mode instead of symlinks
repoverlay apply ./overlay --copy

# Give the overlay a custom name
repoverlay apply ./overlay --name my-config
```

### Remove overlays

```bash
# Interactive removal (lists applied overlays)
repoverlay remove

# Remove a specific overlay by name
repoverlay remove my-overlay

# Remove all overlays
repoverlay remove --all
```

### Check status

```bash
# Show all applied overlays
repoverlay status

# Show a specific overlay
repoverlay status --name my-overlay
```

### Update GitHub overlays

```bash
# Check for and apply updates to all GitHub overlays
repoverlay update

# Check without applying
repoverlay update --dry-run

# Update a specific overlay
repoverlay update my-overlay
```

### Restore after git clean

If you run `git clean -fdx` and lose your overlay files, restore them:

```bash
repoverlay restore

# Preview what would be restored
repoverlay restore --dry-run
```

### Manage cache

GitHub repositories are cached locally to avoid repeated downloads:

```bash
# List cached repositories
repoverlay cache list

# Show cache location
repoverlay cache path

# Clear entire cache
repoverlay cache clear

# Remove a specific cached repo
repoverlay cache remove owner/repo
```

## Overlay Configuration

Create a `repoverlay.toml` in your overlay directory to configure it:

```toml
[overlay]
name = "my-config"
description = "Personal development configuration"

[mappings]
# Rename files when applying
".envrc.template" = ".envrc"
"vscode-settings.json" = ".vscode/settings.json"
```

Without a config file, all files in the overlay directory are symlinked with the same relative path.

## How It Works

1. **Symlinks (default)**: Files are symlinked from the overlay source to the target repository. Changes to the source are immediately reflected.

2. **Copies (`--copy`)**: Files are copied instead. Useful when the source might move or on systems without symlink support.

3. **Git exclusion**: Applied files are automatically added to `.git/info/exclude`, keeping them out of version control without modifying `.gitignore`.

4. **State tracking**: Overlay state is stored in `.repoverlay/` within the repository and backed up externally for recovery.

## Multiple Overlays

You can apply multiple overlays to the same repository:

```bash
repoverlay apply ./editor-config --name editor
repoverlay apply ./env-files --name env
repoverlay apply https://github.com/team/shared-config --name team
```

Each overlay's files must be unique - conflicts between overlays or with existing files will be rejected.

## License

MIT
