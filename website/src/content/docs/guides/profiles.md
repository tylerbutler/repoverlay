---
title: Profiles
sidebar:
  order: 5
---

A **profile** is a named, agent-centric configuration that composes overlays together with AI harness capabilities — MCP servers, instruction files, skills, and plugins — into a single loadable unit.

Where an overlay describes *files to place in a repo*, a profile describes *intent*: "give me everything I need to do Rust development with Copilot." Applying a profile always happens for a specific agent harness (currently GitHub Copilot), and the harness decides where each capability is placed.

## Profiles vs. overlays

The key thing to understand is the difference between a **definition** and its **application**:

- An **overlay** is a reusable, *repo-agnostic* definition — just a named bundle of files. It only becomes associated with a repo when you `apply` it, at which point the files land in that repo's working tree. So an overlay being "tied to a repo" is a property of the *applied instance*, not the overlay itself.
- A **profile** is a *recipe* one layer up: it references overlays (ingredients) and adds harness capabilities. Like an overlay, it's a portable definition that you apply to a specific repo — but a profile's effects span two scopes.

| | Overlay | Profile |
| --- | --- | --- |
| Role | Ingredient (files) | Recipe (overlays + capabilities) |
| Payload | A file tree applied to a repo | A composition of overlays + harness capabilities |
| Scope of effect | Always **repo-scoped** (working tree) | Spans **repo-scoped** *and* **user/harness-scoped** |
| Owns | Symlinks, git excludes, conflict handling | MCP servers, instructions, skills, plugins |
| Portable across harnesses? | N/A (files only) | Yes — placement lives in the harness applicator |

Profiles *reference* overlays rather than replacing them. Repo-level instruction files such as `AGENTS.md` should still be shipped as overlay files; profile `instructions` are for harness/user-level files.

## Defining a profile

Profiles live in your repoverlay CCL config under a `profiles` key, alongside `sources` and `library_path`:

- Global config: `~/.config/repoverlay/config.ccl`
- Per-repo config: `.repoverlay/config.ccl`

```ccl
profiles =
  rust-dev =
    description = Rust development profile
    overlays =
      = rust-base
      = rust-tools

    instructions =
      =
        source = copilot-instructions.md

    mcps =
      servers =
        rust-analyzer =
          command = uvx
          args =
            = mcp-rust
          env =
            RUST_LOG = info

    skills =
      = market:rust-reviewer@playground

    plugins =
      = market:rust-dev@playground
```

### Fields

| Field | Type | Description |
| --- | --- | --- |
| `description` | scalar | Optional user-facing text shown by `profile list` and `profile show`. |
| `overlays` | list | Overlay references, resolved with the usual source/library semantics. |
| `instructions` | list | Harness/user-level instruction files. Each entry has a `source` relative to the repo root. |
| `mcps.servers` | map | MCP servers keyed by name. Each has `command`, optional `args`, and optional `env`. |
| `skills` | list | Skill references. *(Accepted but skipped by the Copilot harness in v1.)* |
| `plugins` | list | Plugin references. *(Accepted but skipped by the Copilot harness in v1.)* |

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
  MCP servers:
    - rust-analyzer (uvx)
```

## Applying a profile

There are two ways to apply a profile. Choose based on whether you want the configuration to stick around.

### Persistent mode

`profile apply` installs the profile and leaves it in place until you remove it:

```bash
repoverlay profile apply rust-dev --harness copilot
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

### Ephemeral mode

`repoverlay copilot --profile` applies the profile only for the lifetime of the launched Copilot process, then cleans up automatically:

```bash
repoverlay copilot --profile rust-dev
```

The flow is:

1. Resolve and plan the profile for Copilot.
2. Apply it using session-scoped state.
3. Launch the Copilot harness.
4. Wait for Copilot to exit.
5. Remove the session's profile effects.
6. Exit with Copilot's exit code (unless cleanup fails).

Pass extra arguments straight through to Copilot after `--`:

```bash
repoverlay copilot --profile rust-dev -- --help
```

:::note
A profile that is already applied persistently (or already running an ephemeral session) cannot be launched ephemerally at the same time. Remove it first, or wait for the running session to finish. A lock file guards against concurrent sessions and is recovered automatically if a previous session was killed.
:::

## How the Copilot harness maps capabilities

Each capability in a profile is translated into a concrete action by the Copilot applicator:

| Capability | What happens |
| --- | --- |
| `overlays` | Applied to the target repo using the normal overlay machinery (symlinks, git excludes, state). |
| `mcps.servers` | Deep-merged into the harness `mcp.json` under a `servers` key. |
| `instructions` | Each `source` (resolved relative to the repo root) is written to `instructions/<profile-name>/<file-name>`. |
| `skills` / `plugins` | Skipped with a warning — Copilot placement is not defined in v1. |

The harness home defaults to `~/.config/github-copilot/`, so:

- MCP servers are written to `~/.config/github-copilot/mcp.json`
- The `rust-dev` profile's `copilot-instructions.md` is written to `~/.config/github-copilot/instructions/rust-dev/copilot-instructions.md`

:::tip
You can override the harness home with the `REPOVERLAY_COPILOT_HOME` environment variable, which is useful for testing or isolated setups.
:::

Instruction `source` paths must be relative and stay within the repo — paths that escape the directory (for example `../secret.md`) or absolute paths are rejected.

## Merge behavior across configs

When a repo-local profile shares a name with a global profile, the global profile is the base and the repo-local profile overrides it. Merging is type-based:

| Field type | Merge behavior |
| --- | --- |
| Scalars (`description`) | Repo-local value wins when set; otherwise the global value is kept. |
| Maps (`mcps.servers`) | Repo-local entries merge into the global map; repo-local keys win on conflict. |
| Lists (`overlays`, `instructions`, `skills`, `plugins`) | Repo-local list replaces the global list when non-empty; otherwise the global list is kept. |

For example, given a global `rust-dev` profile with a `global` MCP server and overlay `global-rust`, and a repo-local `rust-dev` with a `repo` MCP server and overlay `repo-rust`, the merged profile keeps the global description, uses only the `repo-rust` overlay (list replacement), and exposes **both** MCP servers (map merge).

## State and recovery

Profile state is tracked separately from overlay state:

```text
.repoverlay/profiles/<profile-name>.<harness>.ccl
```

State records the applied profile, harness, a fingerprint, the overlays it applied, the files and merged config entries it owns, and any skipped capabilities. This is what lets `profile remove` (and ephemeral cleanup) undo exactly what a profile created.

If an ephemeral session is interrupted and cleanup fails, repoverlay reports the error and leaves enough session state behind to clean up later with `profile remove`.
