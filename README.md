# repoverlay

[![CI](https://github.com/tylerbutler/repoverlay/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/tylerbutler/repoverlay/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/repoverlay)](https://crates.io/crates/repoverlay)
[![codecov](https://codecov.io/gh/tylerbutler/repoverlay/graph/badge.svg)](https://codecov.io/gh/tylerbutler/repoverlay)
[![MIT License](https://img.shields.io/crates/l/repoverlay)](LICENSE)

Overlay config files into git repositories without committing them. Files are symlinked (or copied with `--copy`) from overlay sources and automatically excluded via `.git/info/exclude`.

## Quick Reference

| Task | Command |
|------|---------|
| **Browse & apply overlays** | **`repoverlay browse`** |
| Apply overlay (scripting) | `repoverlay apply <source>` |
| Check status | `repoverlay status` |
| Remove overlay | `repoverlay remove <name>` |
| Remove all | `repoverlay remove --all` |
| Update from remote | `repoverlay update` |
| Restore after git clean | `repoverlay restore` |
| Create overlay | `repoverlay create <name>` |
| Create local overlay | `repoverlay create --output <path>` |
| Edit overlay | `repoverlay edit add <name> <files>` |
| Sync changes back | `repoverlay sync <name>` |
| Switch overlays | `repoverlay switch <source>` |
| Move overlay source | `repoverlay move <name> --to <destination>` |
| Manage sources | `repoverlay source add/list/remove` |
| Manage cache | `repoverlay cache list/remove/path` |
| Manage in-repo library | `repoverlay library list/import/export/remove` |
| Shell completions | `repoverlay completions <shell>` |

## Concepts

### Definitions vs. application

The key distinction in repoverlay is between a **definition** (a reusable, repo-agnostic thing) and its **application** (what happens when you bind that definition to a specific repo).

An **overlay** is a *definition*: a named bundle of config files. Nothing about an overlay is tied to a repo — `rust-base` is just "these files." It only becomes repo-associated when you `apply` it, at which point the files land in *that* repo's working tree and are excluded via `.git/info/exclude`. So "overlays are associated with a repo" is only true of the **applied instance**, not the overlay itself.

A **profile** is a *recipe* one layer up: it composes overlays **and** adds AI-harness capabilities (MCP servers, skills, plugins, harness/user-level instructions). Like overlays, a profile is a portable definition that gets applied to a specific repo — but its effects span two scopes:

| | Payload | Scope of effect when applied |
| --- | --- | --- |
| **Overlay** | files only | always **repo-scoped** (working tree) |
| **Profile** | overlays **+** capabilities | spans **repo-scoped** *and* **user/harness-scoped** |

In short: an **overlay** is an *ingredient* (repo-facing, files only), a **profile** is a *recipe* (harness-facing) that lists overlays plus the capabilities to turn on. A profile *references* overlays rather than replacing them.

### Object reference

repoverlay manages these kinds of objects:

- **Overlay** — a reusable, repo-agnostic bundle of config files. Becomes repo-associated only when applied. Lifecycle: `create` → `apply` → `update` → `remove`.
- **Profile** — a recipe that composes overlays with AI harness capabilities (MCP servers, skills, plugins, harness/user-level instruction files). Applied persistently with `profile apply` or ephemerally with harness commands such as `repoverlay copilot --profile rust-dev`.
- **Source** — a configured location (GitHub repo or local directory) to find overlays. Lifecycle: `source add` → `source list` → `source remove`.
- **Cache** — local clones of GitHub repos used by overlays. Managed automatically on `apply`; inspect with `cache list`, clean with `cache remove --all`.
- **File** — an individual file within an overlay. Managed via `edit` and `sync`.

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

The easiest way to get started is with `browse`, which interactively lists available overlays and lets you select which to apply:

```bash
# Add a source, then browse and apply interactively
repoverlay source add owner/repo
repoverlay browse

# Or browse an ephemeral source directly
repoverlay browse ./path/to/overlays
repoverlay browse owner/repo
```

For scripting or power-user workflows, use `apply` directly:

```bash
# Apply from a local directory
repoverlay apply /path/to/overlay

# Apply from GitHub
repoverlay apply https://github.com/owner/repo

# Remove an overlay
repoverlay remove my-overlay
```

Profiles can declare overlays and AI harness configuration:

```bash
repoverlay profile list
repoverlay profile show rust-dev
```

Choose one of the following application modes.

Persistent mode applies the profile until you remove it:

```bash
repoverlay profile apply rust-dev --harness copilot
```

Ephemeral mode applies the profile only while the harness process runs:

```bash
repoverlay copilot --profile rust-dev -- --help
```

Copilot v1 applies overlays, MCP servers, and harness/user-level instruction files. Skills and plugins are accepted in profile config but skipped with warnings until Copilot placement semantics are defined.

For the full command reference with all options and flags, see the [CLI reference](https://repoverlay.tylerbutler.com/cli-reference/).

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
