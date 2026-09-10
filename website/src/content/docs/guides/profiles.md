---
title: Profiles
sidebar:
  order: 5
---

:::note[New in v0.17.0]
Profiles are available in repoverlay **v0.17.0** and later. See the [release notes](https://github.com/tylerbutler/repoverlay/releases/tag/v0.17.0) for details.
:::

A **profile** is a named configuration for an AI agent. It combines overlays with instruction files and plugins so you can apply them with one command.

An overlay defines files to add to a repository. A profile defines the agent setup for a task, such as Rust development.

You always apply a profile to a specific **harness**, the application that runs the agent. repoverlay supports GitHub Copilot and Claude Code. The harness determines where repoverlay puts each capability.

## Profiles vs. overlays :badge[v0.17.0]{variant=tip}

An overlay or profile definition is separate from its application to a repository:

- An overlay is a named set of files that you can reuse in any repository. When you apply it, repoverlay adds the files to that repository's working tree. The applied instance belongs to the repository, but the definition does not.
- A profile refers to overlays and adds harness capabilities through plugins.

| | Overlay | Profile |
| --- | --- | --- |
| Role | Defines a set of files | Defines an agent setup |
| Contents | A file tree to apply to a repository | Overlays and harness capabilities |
| Manages | Symlinks, git excludes, and file conflicts | Instructions and plugins with skills, agents, and MCP servers |
| Works across harnesses? | Not applicable: files only | Managed capabilities that repoverlay can cache work across harnesses. Delegate support depends on the harness. |

repoverlay applies profile capabilities to the target repository, not globally for the user or machine. It excludes new files from git when possible. Profiles can also update existing files through managed regions or JSON merges. repoverlay keeps cache and recovery snapshots outside the repository to restore profiles after cleanup.

## Capabilities come from plugins

Profiles do not define MCP servers or skills directly. They use **plugins** in the Claude Code plugin format. A plugin can contain:

- Skills for `.claude/skills/` (Claude) or `.agents/skills/` (Copilot).
- Agents for `.claude/agents/` (Claude) or `.github/agents/` (Copilot).
- MCP server definitions to merge into the repository's `.mcp.json`.
- Plugin metadata in `.claude-plugin/plugin.json`.

The same plugin can supply capabilities to different harnesses. For managed plugins that it can cache, repoverlay extracts the parts and puts them in the paths for the selected harness. This works for persistent applications and temporary (ephemeral) sessions.

Claude can also load plugins through its native settings. Copilot skips delegate plugins and plugins that repoverlay cannot cache, with a warning. repoverlay removes temporary placements when the session ends.

## Defining a profile

Define profiles under the `profiles` key in your repoverlay CCL configuration, beside `sources` and `marketplaces`:

- Global configuration: `~/.config/repoverlay/config.ccl`
- Repository configuration: `.repoverlay/config.ccl`

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
        source = instructions.md
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
| `description` | scalar | Optional text for `profile list` and `profile show`. |
| `overlays` | list | Overlay references. repoverlay uses the standard source and library resolution rules. |
| `instructions` | list | Instructions for a managed region in `CLAUDE.md` (Claude) or `AGENTS.md` (Copilot). Each entry must set either `source` or `content`, but not both. `source` is a path relative to the configuration file that defines it. `content` is inline text. |
| `plugins` | list | Plugin references (see below). |

### Marketplaces registry

The top-level `marketplaces` key maps a short name to a marketplace git repository. Plugin references use these names. The `url` accepts a full git URL or GitHub `owner/repo` shorthand. The shorthand expands to `https://github.com/owner/repo`. repoverlay caches marketplace repositories under `~/.cache/repoverlay/`, as it does for overlay sources.

### Plugin references

Each entry in `plugins` is one of:

- **Marketplace shorthand**: `marketplace/plugin`, such as `playground/rust-dev`.
- **Marketplace table**: use this to pin a ref or select an install mode:

  | Key | Description |
  | --- | --- |
  | `marketplace` | Marketplace name from the registry. |
  | `name` | Plugin name within the marketplace. |
  | `ref` | Optional git tag, branch, or commit that pins the marketplace repository checkout. The marketplace manifest defines refs for external plugin sources. |
  | `install` | `managed` (default) or `delegate`. |
  | `scope` | Delegate only: `project` or `local`. |

- **Local path**: a `source` table or a path that starts with `.` or `/`. The path identifies a local plugin directory.

#### Managed vs. delegate

| Install mode | Behavior |
| --- | --- |
| `managed` (default) | repoverlay caches the plugin when it can read its contents. It adds the skills, agents, and MCP servers to the repository. Both Copilot and Claude support plugins that repoverlay can cache. |
| `delegate` | repoverlay enables the plugin in the harness configuration. The harness loads the plugin. Only Claude supports this mode. Copilot skips delegate plugins with a warning. |

For `delegate` plugins on Claude, `scope` selects the settings file that enables the plugin:

| Scope | File |
| --- | --- |
| `project` | `.claude/settings.json` |
| `local` | `.claude/settings.local.json` |

If you omit `scope`, persistent applications use `project`. Ephemeral sessions use `local`.

## Authoring a plugin

A plugin is a directory with a `.claude-plugin/plugin.json` manifest and the
capabilities it supplies. To create one manually:

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

Add skills under `my-plugin/skills/<skill>/SKILL.md`. Then use a local path
(`= ./my-plugin`) to refer to the plugin in a profile.

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
    - instructions.md
  Plugins:
    - playground/rust-dev (managed)
    - vendor/cool (delegate, scope: local)
```

## Applying a profile

Choose persistent mode to keep the configuration until you remove it. Choose ephemeral mode to use it for one agent session.

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

Remove it when you finish:

```bash
repoverlay profile remove rust-dev --harness copilot
```

Removal deletes the harness files and merged configuration entries that the profile created. repoverlay removes an overlay only if both conditions apply:

- This profile applied the overlay.
- No other applied profile refers to the overlay.

#### Keeping managed plugins up to date

repoverlay pins managed plugins to their resolved commit when you apply a profile. It records that commit in profile state.

Run `repoverlay update` without an overlay name filter to resolve managed plugins again for every persistent profile. If a plugin source has changed, repoverlay applies the profile again. It does not change delegate plugins or plugins pinned to a fixed `ref`.

Use `repoverlay update --dry-run` to preview which profiles need a new application.

### Ephemeral mode

Use `repoverlay copilot --profile` or `repoverlay claude --profile` to apply profiles for one agent session. repoverlay removes the session changes automatically when the agent exits:

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

Put extra arguments for the agent after `--`:

```bash
repoverlay copilot --profile rust-dev -- --help
```

#### Applying several profiles at once :badge[v0.17.0]{variant=tip}

Repeat `--profile` to use multiple profiles in one ephemeral session. repoverlay applies and locks each profile separately. It removes all of them when the agent exits:

```bash
repoverlay copilot --profile rust-dev --profile docs-dev
repoverlay claude --profile rust-dev --profile docs-dev -- --help
```

Profiles can share instruction files: `CLAUDE.md` for Claude and `AGENTS.md` for Copilot. Each profile has its own managed region. repoverlay merges servers in `.mcp.json` and records which profile owns each server.

If one profile fails to apply, repoverlay rolls back the profiles already applied by the same command. This prevents a partially configured repository. Do not repeat a profile name in one command.

For Claude and Copilot, repoverlay extracts managed plugins that it can cache into repository files. These include skills, agents, and merged `.mcp.json` servers. Persistent applications and ephemeral sessions use the same process.

Claude loads delegate plugins and plugins that repoverlay cannot cache through its own settings. Copilot skips them with a warning. repoverlay removes ephemeral placements when the session exits.

:::note
You cannot start an ephemeral session for a profile that is already applied persistently or active in another ephemeral session. Remove the persistent profile first, or wait for the active session to finish. A lock file prevents concurrent sessions. repoverlay recovers the lock automatically if a previous session was killed.
:::

## How harnesses map capabilities

repoverlay applies each capability to the target repository according to the selected harness:

| Capability | Claude | Copilot |
| --- | --- | --- |
| `overlays` | Uses the standard overlay process: symlinks, git excludes, and state. | Same. |
| plugin skills | `<repo>/.claude/skills/<skill>` | `<repo>/.agents/skills/<skill>` |
| plugin agents | `<repo>/.claude/agents/<agent>` | `<repo>/.github/agents/<agent>` |
| plugin MCP servers | Merged into `<repo>/.mcp.json` (`mcpServers` key). | Merged into `<repo>/.mcp.json` (`mcpServers` key). |
| delegate plugins | Recorded in `.claude/settings.json` / `.claude/settings.local.json`. | Skipped with a warning. |
| `instructions` | repoverlay combines each entry's `source` file or inline `content` in the profile's managed region of `<repo>/CLAUDE.md`. | Same, in `<repo>/AGENTS.md`. |

repoverlay applies capabilities to the target repository, not globally for the user or machine. It excludes new files from git when possible. It can update existing files through managed regions or JSON merges. Cache and recovery snapshots stay outside the repository.

Instruction `source` paths must be relative to the configuration file that defines them. They must stay within that file's directory. repoverlay rejects absolute paths and paths outside the directory, such as `../secret.md`.

Store repository profile sources within the directory that contains `<repo>/.repoverlay/config.ccl`. Store global profile sources within the global configuration directory. To avoid separate files, use inline `content`.

## Merge behavior across configs

If a repository profile and a global profile have the same name, repoverlay starts with the global profile. It applies repository values according to the field type:

| Field type | Merge behavior |
| --- | --- |
| Scalars (`description`) | Uses the repository value if set. Otherwise, keeps the global value. |
| Lists (`overlays`, `instructions`, `plugins`) | Uses the repository list if it is not empty. Otherwise, keeps the global list. |

The top-level `marketplaces` registry merges by name. Repository entries replace global entries with the same name. repoverlay keeps other global marketplaces.

## State and recovery

repoverlay records profile state separately from overlay state:

```text
.repoverlay/profiles/<profile-name>.<harness>.ccl
```

State records the applied profile, harness, fingerprint, applied overlays, managed files, and merged configuration entries. `profile remove` and ephemeral cleanup use these records to remove only what the profile created.

If an ephemeral session stops unexpectedly and cleanup fails, repoverlay reports the error. It keeps enough session state for later cleanup. Use `profile remove` to finish the cleanup.
