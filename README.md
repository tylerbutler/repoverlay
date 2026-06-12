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
| Manage profiles | `repoverlay profile list/show/apply/status/remove` |
| Run agent with profiles | `repoverlay claude --profile <name>` / `repoverlay copilot --profile <name>` |
| Manage plugin marketplaces | `repoverlay marketplace add/list/remove` |
| Shell completions | `repoverlay completions <shell>` |

## Concepts

### Definitions vs. application

The key distinction in repoverlay is between a **definition** (a reusable, repo-agnostic thing) and its **application** (what happens when you bind that definition to a specific repo).

An **overlay** is a *definition*: a named bundle of config files. Nothing about an overlay is tied to a repo — `rust-base` is just "these files." It only becomes repo-associated when you `apply` it, at which point the files land in *that* repo's working tree and are excluded via `.git/info/exclude`. So "overlays are associated with a repo" is only true of the **applied instance**, not the overlay itself.

A **profile** is a *recipe* one layer up: it composes overlays **and** adds AI-harness capabilities — instruction files and plugins. (Profiles are [experimental](#profiles-experimental).) Plugins are the bundling unit for skills and MCP servers, in the same Claude-style format used by the Claude Code ecosystem. Like overlays, a profile is a portable definition that gets applied to a specific repo, and everything it applies lands **inside that repo's working tree** (git-excluded):

| | Payload | Scope of effect when applied |
| --- | --- | --- |
| **Overlay** | files only | **repo-scoped** (working tree) |
| **Profile** | overlays **+** capabilities | **repo-scoped** (working tree) |

In short: an **overlay** is an *ingredient* (files only), a **profile** is a *recipe* that lists overlays plus the plugins/instructions to turn on. A profile *references* overlays rather than replacing them. There is no user- or machine-global scope — a profile only ever touches the repo you apply it to.

### Object reference

repoverlay manages these kinds of objects:

- **Overlay** — a reusable, repo-agnostic bundle of config files. Becomes repo-associated only when applied. Lifecycle: `create` → `apply` → `update` → `remove`.
- **Profile** *(experimental)* — a recipe that composes overlays with AI harness capabilities (plugins bundling skills + MCP servers, and instruction files), all placed repo-local. Applied persistently with `profile apply` or ephemerally with harness commands such as `repoverlay copilot --profile rust-dev` or `repoverlay claude --profile rust-dev`.
- **Plugin** *(experimental)* — a Claude-style bundle of skills and/or MCP servers. Referenced from a profile via a marketplace (`marketplace/plugin`) or a local path (a directory with a `.claude-plugin/plugin.json` manifest).
- **Marketplace** *(experimental)* — a named git repository registry that plugins are resolved from, cached locally like overlay sources.
- **Source** — a configured location (GitHub repo or local directory) to find overlays. Lifecycle: `source add` → `source list` → `source remove`.
- **Cache** — local clones of GitHub repos used by overlays. Managed automatically on `apply`; inspect with `cache list`, clean with `cache remove --all`.
- **File** — an individual file within an overlay. Managed via `edit` and `sync`.

## Installation

### Homebrew (macOS/Linux)

**Install:**
```bash
brew install tylerbutler/tap/repoverlay
```

**Update:**
```bash
brew update && brew upgrade repoverlay
```

**Uninstall:**
```bash
brew uninstall repoverlay
```

### Shell installer (macOS/Linux)

**Install:**
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tylerbutler/repoverlay/releases/latest/download/repoverlay-installer.sh | sh
```

The binary is installed to `$CARGO_HOME/bin` (or `$HOME/.cargo/bin` if `CARGO_HOME` is not set).

**Update:**
Rerun the install command in the Install section to replace the binary.

**Uninstall:**
```bash
rm "${CARGO_HOME:-$HOME/.cargo}/bin/repoverlay"
```

### PowerShell installer (Windows)

**Install:**
```powershell
irm https://github.com/tylerbutler/repoverlay/releases/latest/download/repoverlay-installer.ps1 | iex
```

The binary is installed to `$env:CARGO_HOME\bin` (or `$env:USERPROFILE\.cargo\bin` if `CARGO_HOME` is not set).

**Update:**
Rerun the install command in the Install section to replace the binary.

**Uninstall:**
```powershell
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
Remove-Item (Join-Path $cargoHome "bin\repoverlay.exe")
```

### Cargo

**Install:**
```bash
cargo binstall repoverlay  # pre-built binary
cargo install repoverlay   # build from source
```

**Update:**
```bash
cargo binstall repoverlay --force
# or
cargo install repoverlay --force
```

**Uninstall:**
```bash
cargo uninstall repoverlay
```

### Manual binaries

Download the pre-built binaries for your platform from the [repoverlay releases](https://github.com/tylerbutler/repoverlay/releases).

**Update:** Download the latest release artifact and replace the `repoverlay` binary on your `PATH`.

**Uninstall:** Delete the `repoverlay` binary from your `PATH`.

## Usage

The easiest way to start is with `browse`, which interactively lists available overlays and lets you select which to apply:

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

### Profiles (experimental)

> [!WARNING]
> Profiles, plugins, and the harness commands are **experimental**. The `profile`, `claude`, and `copilot` commands work but are hidden from `--help` and the CLI reference, and their interface may change before they are announced.

Profiles can declare overlays and AI harness configuration. Define them under the `profiles` key of the global config (`~/.config/repoverlay/config.ccl`) or the repo-local config (`.repoverlay/config.ccl`):

```
profiles =
  rust-dev =
    description = Rust development with AI tooling
    overlays =
      = org/repo/rust-base
    instructions =
      =
        source = instructions/rust.md
      =
        content = Prefer cargo nextest for running tests.
    plugins =
      = my-marketplace/rust-tools
      = ./plugins/local-bundle
```

All four keys are optional. `overlays` lists overlay references applied through the normal apply machinery. Each `instructions` entry sets exactly one of `source` (a file path relative to the config file's directory) or `content` (inline text). `plugins` entries are `marketplace/plugin` references (register marketplaces with `repoverlay marketplace add <name> <url>`) or local paths to a Claude-style plugin bundle; a table form supports pinning a git `ref` and choosing `install = managed|delegate`.

Inspect profiles with:

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
repoverlay claude --profile rust-dev
```

Capabilities are placed repo-local: plugin skills go to `.agents/skills/` (Copilot) or `.claude/skills/` (Claude), plugin MCP servers merge into the repo's `.mcp.json`, and instruction files are written into a managed region of `CLAUDE.md` (Claude) or `AGENTS.md` (Copilot). Claude can also *delegate* plugin enablement to its own settings instead of placing files. A full `repoverlay update` re-resolves applied profiles' managed plugins and re-applies any whose source changed.

For the full command reference with all options and flags, see the [CLI reference](https://repoverlay.tylerbutler.com/cli-reference/).

## Migrating to 1.0

repoverlay 1.0 removes hidden deprecated CLI aliases and unsupported source URL schemes.
If you use older command forms such as `create-local`, top-level `list`, `edit --add`,
or `cache clear`, see the [1.0 migration guide](https://repoverlay.tylerbutler.com/guides/migrating-to-1-0/)
for replacements.

## Overlay Configuration

Create a `repoverlay.ccl` in your overlay directory:

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

**`overlay`** — Metadata: `name` (defaults to the directory name) and an optional `description`.

**`mappings`** — Rename files during apply (source = destination). Repeat a source key to map one file to multiple destinations.

**`directories`** — Directories to symlink (or copy) as a unit rather than walking individual files. Useful for directories like `.claude/` that should be managed atomically.

**`extends`** / **`includes`** — Inherit all files from a parent overlay, or cherry-pick files from other overlays. Composition works between overlays in the [in-repo library](https://repoverlay.tylerbutler.com/guides/library/); see [Composing overlays](https://repoverlay.tylerbutler.com/guides/creating/#composing-overlays).

Without a config file, all files are symlinked with the same relative path.

## License

MIT
