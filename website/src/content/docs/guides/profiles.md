---
title: Profiles
sidebar:
  order: 5
---

A **profile** is a named, agent-centric configuration that composes overlays together with AI harness capabilities — instruction files and plugins — into a single loadable unit.

Where an overlay describes *files to place in a repo*, a profile describes *intent*: "give me everything I need to do Rust development with this agent." Applying a profile always happens for a specific agent harness (currently GitHub Copilot and Claude Code), and the harness decides where each capability is placed.

## Profiles vs. overlays

The key thing to understand is the difference between a **definition** and its **application**:

- An **overlay** is a reusable, *repo-agnostic* definition — just a named bundle of files. It only becomes associated with a repo when you `apply` it, at which point the files land in that repo's working tree. So an overlay being "tied to a repo" is a property of the *applied instance*, not the overlay itself.
- A **profile** is a *recipe* one layer up: it references overlays (ingredients) and adds harness capabilities through plugins.

| | Overlay | Profile |
| --- | --- | --- |
| Role | Ingredient (files) | Recipe (overlays + capabilities) |
| Payload | A file tree applied to a repo | A composition of overlays + harness capabilities |
| Owns | Symlinks, git excludes, conflict handling | Instructions, plugins (skills + agents + MCP servers) |
| Portable across harnesses? | N/A (files only) | Yes — placement lives in the harness applicator |

Everything a profile applies lands **inside the target repo's working tree** (git-excluded, never committed). There is no user- or machine-global scope — a profile only ever touches the repo you apply it to.

## Capabilities come from plugins

Profiles do not define MCP servers or skills directly. Instead, those capabilities are bundled into **plugins** — the same Claude-style plugin format used by the Claude Code ecosystem. A plugin can ship:

- **skills** — placed under `.claude/skills/` (Claude) or `.agents/skills/` (Copilot)
- **MCP servers** — merged into the repo's `.mcp.json`
- plugin metadata in `.claude-plugin/plugin.json`

This keeps a single, portable bundling unit: the same plugin works whether it is enabled through Claude's native plugin machinery or placed by repoverlay for Copilot.

## Defining a profile

Profiles live in your repoverlay CCL config under a `profiles` key, alongside `sources` and `marketplaces`:

- Global config: `~/.config/repoverlay/config.ccl`
- Per-repo config: `.repoverlay/config.ccl`

```ccl
marketplaces =
  =
    name = playground
    url = obra/claude-plugins
  =
    name = vendor
    url = https://example.com/vendor/market.git

profiles =
  rust-dev =
    description = Rust development profile
    overlays =
      = rust-base
      = rust-tools

    instructions =
      =
        source = copilot-instructions.md
      =
        content =
          Be concise in all responses.
          Prefer composition over inheritance.

    plugins =
      = playground/rust-dev
      =
        marketplace = vendor
        name = cool
        install = delegate
        scope = local
```

### Profile fields

| Field | Type | Description |
| --- | --- | --- |
| `description` | scalar | Optional user-facing text shown by `profile list` and `profile show`. |
| `overlays` | list | Overlay references, resolved with the usual source/library semantics. |
| `instructions` | list | Harness instruction entries. Each entry sets exactly one of `source` (a file path resolved relative to the config file that defines it) or `content` (inline text). |
| `plugins` | list | Plugin references (see below). |

### Marketplaces registry

The top-level `marketplaces` key maps a short name to a marketplace git repository. Plugin references use these names. The `url` accepts a full git URL or GitHub `owner/repo` shorthand (expanded to `https://github.com/owner/repo`). Marketplace repositories are cached locally under `~/.cache/repoverlay/`, the same way overlay sources are.

### Plugin references

Each entry in `plugins` is one of:

- **Marketplace shorthand** — `marketplace/plugin`, e.g. `playground/rust-dev`.
- **Marketplace table** — for pinning a ref or choosing an install mode:

  | Key | Description |
  | --- | --- |
  | `marketplace` | Marketplace name from the registry. |
  | `name` | Plugin name within the marketplace. |
  | `ref` | Optional git ref (tag/branch/commit) to pin. |
  | `install` | `managed` (default) or `delegate`. |
  | `scope` | Delegate only: `project` or `local`. |

- **Local path** — a `source` table or a path starting with `.` or `/`, pointing at a plugin directory on disk.

#### Managed vs. delegate

| Install mode | Behavior |
| --- | --- |
| `managed` (default) | repoverlay caches the plugin bundle and places its skills/MCP servers into the repo itself. Works for both Copilot and Claude. |
| `delegate` | repoverlay records the plugin in the harness's own enablement config and lets the harness load it. Currently meaningful for Claude. |

For `delegate` plugins on Claude, the `scope` selects which settings file records the enablement:

| Scope | File |
| --- | --- |
| `project` | `.claude/settings.json` |
| `local` | `.claude/settings.local.json` |

When `scope` is omitted, the default depends on how the profile is applied: persistent applies use `project`, and ephemeral sessions use `local`.

## Authoring a plugin

A plugin is just a directory with a `.claude-plugin/plugin.json` manifest plus the
capabilities it ships. To create one by hand:

```bash
mkdir -p my-plugin/.claude-plugin my-plugin/skills
```

```json
// my-plugin/.claude-plugin/plugin.json
{
  "name": "my-plugin",
  "version": "0.1.0",
  "description": "What this plugin provides"
}
```

```json
// my-plugin/.mcp.json (optional — only if the plugin ships MCP servers)
{
  "mcpServers": {
    "example": { "command": "uvx", "args": ["example-mcp"] }
  }
}
```

Add skills under `my-plugin/skills/<skill>/SKILL.md`, then reference the plugin from a
profile as a local path (`= ./my-plugin`).

## Inspecting profiles

List configured profiles (with descriptions):

```bash
repoverlay profile list
```

Show the resolved contents of one profile:

```bash
repoverlay profile show rust-dev
```

Example output:

```text
rust-dev
  Description: Rust development profile
  Overlays:
    - rust-base
    - rust-tools
  Instructions:
    - copilot-instructions.md
  Plugins:
    - playground/rust-dev (managed)
    - vendor/cool (delegate, scope: local)
```

## Applying a profile

There are two ways to apply a profile. Choose based on whether you want the configuration to stick around.

### Persistent mode

`profile apply` installs the profile and leaves it in place until you remove it:

```bash
repoverlay profile apply rust-dev --harness copilot
repoverlay profile apply rust-dev --harness claude
```

Check what is currently applied:

```bash
repoverlay profile status
repoverlay profile status --harness copilot
```

Remove it when you're done:

```bash
repoverlay profile remove rust-dev --harness copilot
```

Removal deletes the harness files and merged config entries the profile created, and removes overlays **only** if this profile applied them and no other applied profile still references them.

#### Keeping managed plugins up to date

Managed plugins are pinned to the commit they resolved to at apply time (recorded in profile state). Running `repoverlay update` (with no overlay name filter) re-resolves the managed plugins of every persistently-applied profile, and re-applies a profile when any of its plugin sources changed. Delegate plugins and plugins pinned to a fixed `ref` are left untouched. Use `repoverlay update --dry-run` to preview which profiles would be re-applied.

### Ephemeral mode

`repoverlay copilot --profile` (or `repoverlay claude --profile`) applies the profile only for the lifetime of the launched agent process, then cleans up automatically:

```bash
repoverlay copilot --profile rust-dev
repoverlay claude --profile rust-dev
```

The flow is:

1. Resolve and plan the profile for the harness.
2. Apply it using session-scoped state.
3. Launch the agent harness.
4. Wait for the agent to exit.
5. Remove the session's profile effects.
6. Exit with the agent's exit code (unless cleanup fails).

Pass extra arguments straight through to the agent after `--`:

```bash
repoverlay copilot --profile rust-dev -- --help
```

For Claude, managed plugin bundles are loaded directly via Claude's native `--plugin-dir` flag from the cache, so the ephemeral session does not place plugin files into the repo.

:::note
A profile that is already applied persistently (or already running an ephemeral session) cannot be launched ephemerally at the same time. Remove it first, or wait for the running session to finish. A lock file guards against concurrent sessions and is recovered automatically if a previous session was killed.
:::

## How harnesses map capabilities

Each capability in a profile is translated into a concrete, **repo-local** action by the harness applicator:

| Capability | Claude | Copilot |
| --- | --- | --- |
| `overlays` | Applied with the normal overlay machinery (symlinks, git excludes, state). | Same. |
| plugin skills | `<repo>/.claude/skills/<skill>` | `<repo>/.agents/skills/<skill>` |
| plugin agents | `<repo>/.claude/agents/<agent>` | `<repo>/.github/agents/<agent>` |
| plugin MCP servers | Merged into `<repo>/.mcp.json` (`mcpServers` key). | Merged into `<repo>/.mcp.json` (`servers` key). |
| delegate plugins | Recorded in `.claude/settings.json` / `.claude/settings.local.json`. | n/a |
| `instructions` | n/a | Each entry's `source` file or inline `content` is concatenated into the `<repo>/AGENTS.md` managed region for the profile. |

Everything is placed inside the target repo and git-excluded; nothing is written to a user- or machine-global location.

Instruction `source` paths must be relative and stay within the directory of the config file that defines them — paths that escape that directory (for example `../secret.md`) or absolute paths are rejected. A repo profile's sources therefore live next to `<repo>/.repoverlay/config.ccl`; a global profile's live next to the global config. Use inline `content` to avoid companion files entirely.

## Merge behavior across configs

When a repo-local profile shares a name with a global profile, the global profile is the base and the repo-local profile overrides it. Merging is type-based:

| Field type | Merge behavior |
| --- | --- |
| Scalars (`description`) | Repo-local value wins when set; otherwise the global value is kept. |
| Lists (`overlays`, `instructions`, `plugins`) | Repo-local list replaces the global list when non-empty; otherwise the global list is kept. |

The top-level `marketplaces` registry merges by name: repo-local entries win, and global-only marketplaces are kept.

## State and recovery

Profile state is tracked separately from overlay state:

```text
.repoverlay/profiles/<profile-name>.<harness>.ccl
```

State records the applied profile, harness, a fingerprint, the overlays it applied, and the files and merged config entries it owns. This is what lets `profile remove` (and ephemeral cleanup) undo exactly what a profile created.

If an ephemeral session is interrupted and cleanup fails, repoverlay reports the error and leaves enough session state behind to clean up later with `profile remove`.
