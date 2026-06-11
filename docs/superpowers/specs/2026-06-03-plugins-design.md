# Plugins Design

> Extends [2026-06-02-profiles-design.md](2026-06-02-profiles-design.md). This spec
> revises how profiles model agent capabilities: it makes plugins the single bundling
> primitive for MCP servers and skills, **removes** the inline `mcps` and `skills`
> profile fields, and treats plugins as **cached, introspectable bundles** managed by
> repoverlay — the same model used for overlay sources.
>
> **Revision (2026-06-03):** during implementation the user/global scope was dropped in
> favor of a repo-local-only model. Everything a profile applies (plugin skills, MCP
> servers, instructions, delegate settings) lands inside the target repo's working tree;
> the delegate `scope` is now just `project` (`.claude/settings.json`) or `local`
> (`.claude/settings.local.json`). References to a `user` scope or `~/.claude` below have
> been updated accordingly.
>
> **Revision (2026-06-03):** the `repoverlay plugin new` scaffolder (originally Task 10)
> was dropped. Authoring a plugin is just creating a directory with a
> `.claude-plugin/plugin.json` manifest, documented by example in the profiles guide; a
> CLI scaffolder added command surface for little benefit and risked plugin-format drift.
>
> **Revision (2026-06-10):** the `claude --plugin-dir <cache>` ephemeral load path
> described below was dropped in the final implementation. Ephemeral Claude sessions use
> the same bundle decomposition as persistent applies (skills, agents, and merged
> `.mcp.json` placed into the repo working tree), with all placements rolled back when
> the session exits. References to `--plugin-dir` below reflect the original design, not
> shipped behavior.

## Summary

Profiles gain first-class support for **plugins** modeled on Claude Code's plugin
format. A plugin is a self-contained bundle of agent capabilities — MCP servers, skills,
agents, hooks, and LSP servers — described by a `.claude-plugin/plugin.json` manifest.

This spec makes plugins the **only** way a profile delivers MCP servers and skills. The
previous inline `mcps` map and `skills` list are removed. A one-off MCP server is a small
plugin containing a `.mcp.json`; a skill is a plugin containing a `skills/` directory.

The central design decision: **repoverlay caches and introspects plugins exactly like it
caches overlay sources.** A marketplace plugin is fetched with a shallow git clone into the
repoverlay cache, pinned to a commit, read directly, and placed by repoverlay. Because
repoverlay can read the bundle, every harness consumes the parts it understands. For a
persistent profile both harnesses decompose the cached bundle into their native locations
(skills, MCP config); for an ephemeral Claude session repoverlay instead launches
`claude --plugin-dir <cache>` to load the bundle natively as a full plugin. Delegating
enablement to Claude Code is kept only as a narrow fallback for sources repoverlay cannot
clone or introspect.

The profile schema stays harness-neutral. A plugin is mapped to concrete side effects by
per-harness applicators.

## Motivation

Today a profile can deliver MCP servers two ways — an inline `mcps.servers` map and (in the
prior plugin sketch) a plugin-bundled `.mcp.json`. Skills are similarly split between a bare
`skills` list and plugin-bundled skills. This duplication is confusing and grows worse as
more harnesses and capability types are added.

Claude plugins already define a clean, documented bundle format that packages MCP servers,
skills, agents, hooks, and LSP servers together. Adopting that bundle as the single
primitive removes the duplication: a capability comes from exactly one place — a plugin.

The earlier draft of this spec treated marketplace plugins as **opaque**, Claude-managed
bundles. That forced an awkward `local`-vs-`marketplace` split (introspectable vs opaque)
and left non-Claude harnesses unable to consume marketplace MCP servers. Caching plugins
the way repoverlay already caches overlay sources removes that split entirely.

## Goals

- Add a typed `plugins` profile field that references a plugin by source.
- Make plugins the single delivery mechanism for MCP servers and skills.
- Cache and introspect plugin bundles using the existing overlay cache machinery; pin to a
  resolved commit.
- Let every harness consume the parts of a cached plugin it understands.
- Have repoverlay own plugin placement and lifecycle (apply / remove / update), like
  overlays, rather than depending on Claude's marketplace machinery.
- Keep delegate-to-Claude as a narrow fallback for non-cloneable / opaque sources.
- Track plugin-owned placement (and, for the delegate fallback, settings entries) precisely
  so persistent removal and ephemeral cleanup are reliable.

## Non-goals

- Do not introduce harness-specific config blobs into the profile schema. A plugin is a
  neutral reference; applicators own placement.
- Do not keep inline `mcps` or `skills` profile fields. They are removed.
- Do not reimplement Claude's plugin runtime. repoverlay reads the bundle and places it;
  Claude still executes hooks, MCP servers, and skills at runtime.
- Do not vendor plugin contents into the repo working tree. Cached bundles live in the
  repoverlay cache; placement into harness dirs is by symlink/copy, like overlays.

## Background: the Claude plugin model

A Claude plugin is a directory that can contain skills (`skills/<name>/SKILL.md` or
`commands/`), agents (`agents/`), hooks (`hooks/hooks.json`), MCP servers (`.mcp.json`),
LSP servers (`.lsp.json`), and monitors, described by a `.claude-plugin/plugin.json`
manifest. Plugins are distributed through a **marketplace** — a git repo with a
`.claude-plugin/marketplace.json` that lists plugins and, for each, a `source` (a subdir of
the marketplace repo, an external git repo, or a non-git source such as npm).

Claude's own install flow registers the marketplace and then writes
`enabledPlugins."<plugin>@<marketplace>" = true` into a settings file, after which Claude
fetches and loads the plugin. **repoverlay does not need to use this flow** when it can
clone the bundle itself.

Two native Claude load mechanisms are relevant (both verified by spike, see below):

- **`claude --plugin-dir <path>`** loads a folder containing `.claude-plugin/plugin.json`
  as a complete plugin (skills, MCP servers, hooks) for that session only, sourced as
  `<name>@inline`. repoverlay uses this for **ephemeral** sessions (`claude --profile`):
  point the flag at the cached bundle, get a full native load, and the load disappears when
  the process exits — no cleanup required.
- **Bare skills and native config files** are auto-loaded on a plain `claude` launch:
  `.claude/skills/<skill>/SKILL.md`, project `.mcp.json` servers, `.claude/agents/`, etc.
  repoverlay uses this for **persistent** placement (`profile apply`) by decomposing the
  cached bundle into these locations, mirroring how the Copilot applicator decomposes it.

> There is **no** "drop a plugin folder into a skills directory and it auto-loads"
> mechanism: a folder with `.claude-plugin/plugin.json` placed under `.claude/skills/` is
> ignored on a plain launch. Persistent placement therefore decomposes; it does not copy the
> whole bundle. When repoverlay writes a bundled MCP server into a plain `.mcp.json`, it
> resolves `${CLAUDE_PLUGIN_ROOT}` itself (to the cache path), since that variable is only
> substituted inside an actual plugin load.

Claude's own install flow (register marketplace → `enabledPlugins."<plugin>@<marketplace>"
= true`) remains the **delegate** fallback for opaque/non-git sources repoverlay cannot
clone.

## How repoverlay caches a plugin (the overlay parallel)

repoverlay already caches overlay sources: `CacheManager::ensure_cached` does a shallow
`git clone --depth 1`, resolves the commit SHA, supports a subpath, and records
`CacheMeta { clone_url, requested_ref, commit }` under
`~/.cache/repoverlay/github/<owner>/<repo>/` (see `src/cache.rs`). A marketplace is just a
git repo, so plugin caching reuses this directly.

Resolution for a marketplace plugin reference (`marketplace/plugin`):

1. Look up `marketplace` in the `marketplaces` registry to get its `url`.
2. Clone/cache the marketplace repo (shallow, commit-pinned).
3. Read `.claude-plugin/marketplace.json`; find the entry for the requested plugin `name`.
4. Resolve that entry's `source`:
   - **Subdir of the marketplace repo** → use a subpath into the already-cached clone.
   - **External git repo** → cache that repo too (shallow, commit-pinned).
   - **Non-git source** (npm, etc.) → cannot be cached/introspected → use the delegate
     fallback (below).
5. The resolved bundle directory is read by `plugin.rs` (manifest + `.mcp.json` + `skills/`).

A local (path) plugin skips steps 1–4: it is already a directory repoverlay can read. A
reference to an unregistered marketplace name fails fast.

## Configuration schema

`plugins` changes from `Vec<String>` to a typed list. The inline `mcps` and `skills` fields
are removed from `ProfileConfig`.

Marketplaces are declared once in a top-level `marketplaces` registry — a sibling of
`sources` and `library_path` — and plugins reference them by name. This mirrors the overlay
`sources` registry and the overlay-repo `org/repo/name` reference pattern, so a plugin that
lives in marketplace `playground` is referenced as `playground/<plugin>`.

```ccl
# Top-level registry. `url` accepts owner/repo shorthand, like overlay sources.
marketplaces =
  =
    name = playground
    url  = tylerbutler/playground-marketplace
  =
    name = official
    url  = anthropics/claude-code-plugins

profiles =
  rust-dev =
    description = Rust development profile
    overlays =
      = rust-base

    instructions =
      =
        source = copilot-instructions.md

    plugins =
      = playground/rust-dev          # shorthand: <marketplace>/<plugin>, cached + introspected
      = playground/rust-formatter    # same marketplace, no repetition
      = ./plugins/local-mcp          # local path plugin (no marketplace), read in place
      =                              # expanded form, when options are needed
        marketplace = official
        name = rust-analyzer-lsp
        ref = v2.1.0                 # optional pin; resolved commit recorded in state
        install = delegate           # opt into the Claude-managed fallback
        scope = local                # delegate-only; settings file to write
```

### Marketplace registry fields

| Field  | Description |
| ------ | ----------- |
| `name` | Local name used in `marketplace/plugin` references and the `name@marketplace` settings key. |
| `url`  | Git source of the marketplace repo. Accepts owner/repo shorthand, expanded like overlay `sources`. |

A repo-local `marketplaces` entry with the same `name` as a global one overrides it (map
merge by name, matching the source/profile merge rules).

### Plugin reference forms

A plugin entry is either a **shorthand string** or an **expanded table**:

- `marketplace/plugin` — a plugin named `plugin` from the registered marketplace `marketplace`.
- `./path` or `/path` — a local plugin directory (in repo / library / overlay-shipped).
- Expanded table — used when a reference needs options.

### Plugin reference fields (expanded form)

| Field         | Applies to        | Description |
| ------------- | ----------------- | ----------- |
| `marketplace` | marketplace       | Name of a registered marketplace (from the `marketplaces` registry). |
| `name`        | marketplace       | Plugin name within that marketplace. |
| `source`      | local             | Local path / library ref / overlay-relative dir (alternative to the shorthand string). |
| `ref`         | marketplace       | Optional pin (branch/tag/commit). The resolved commit is recorded in state. |
| `install`     | marketplace       | `managed` (default; repoverlay caches + places) or `delegate` (Claude enablement). Auto-forced to `delegate` when the resolved plugin source is not cloneable. |
| `scope`       | delegate          | `project` / `local`. Optional; defaults by mode (see Scope defaults). Only meaningful for the delegate fallback. Both files are repo-local. |

The reference shape determines default behavior: a path is a local plugin; a
`marketplace/plugin` reference is `managed` caching by default; a plugin whose
`marketplace.json` source is non-cloneable forces `delegate`.

### Removed fields

- `mcps` (`McpConfig` / `McpServerConfig`) is removed. An MCP server is delivered as a
  plugin whose `.mcp.json` defines the server.
- `skills` (the bare list) is removed. A skill is delivered as a plugin whose `skills/`
  directory contains it.

### Merge behavior

`plugins` remains a list field and follows the existing list rule: a repo-local profile's
non-empty `plugins` list **replaces** the global list; otherwise the global list is kept.

## Overlays and plugins

With caching, overlays and plugins share machinery (clone, commit-pin, place by
symlink/copy) but keep distinct roles:

| | Overlay | Plugin |
| --- | --- | --- |
| Payload | Arbitrary files placed into the repo working tree | A capability bundle (MCP / skills / agents / hooks) |
| Placement target | Repo working tree, git-excluded | Native harness locations (Claude `.claude/skills/`, `.mcp.json`; Copilot `mcp.json`) — or `--plugin-dir` for an ephemeral Claude session |
| Fetch + pin | repoverlay cache (shallow clone, commit) | repoverlay cache (shallow clone, commit) — **same** |
| Lifecycle owner | repoverlay | repoverlay (managed); Claude (delegate fallback only) |

A plugin can also be **shipped through an overlay**: an overlay places a plugin directory
into the repo/library, and the profile references it as a local `path` plugin. The overlay
owns the files; the profile turns the capability on.

## Architecture

Build on the existing profile modules and the cache:

```text
src/config.rs                # add `marketplaces: Vec<Marketplace>` registry to RepoverlayConfig
src/profile.rs               # add PluginRef type; remove McpConfig/skills from ProfileConfig
src/plugin.rs                # NEW: plugin bundle model; marketplace registry + marketplace.json + plugin.json + .mcp.json parsing
src/profile_plan.rs          # add plan actions for plugin placement + delegate enablement
src/profile_applicators/mod.rs
src/profile_applicators/copilot.rs   # cached-plugin introspection -> mcp.json / skills
src/profile_applicators/claude.rs    # NEW: decompose cached bundle into .claude/skills + .mcp.json (persistent); --plugin-dir launch (ephemeral)
src/cache.rs                 # reused; possibly extended for non-overlay subpaths
```

`config.rs` gains a `marketplaces` registry (`name` + `url`) alongside `sources`, resolved
with the same global/repo-local merge-by-name rules. `plugin.rs` owns: resolving a
`marketplace/plugin` reference against that registry, cloning/resolving to a cached bundle
dir (via `CacheManager`), parsing `.claude-plugin/plugin.json` and `marketplace.json`,
reading a bundled `.mcp.json`, enumerating `skills/`, and validating a local plugin
directory.

### Applicator mapping

| Plugin form | Claude applicator | GitHub Copilot applicator |
| --- | --- | --- |
| Cached / local (introspectable), persistent | Decompose the cached bundle into native locations: each `skills/<skill>` → `.claude/skills/<skill>` (symlink/copy); `.mcp.json` servers → project `.mcp.json` with `${CLAUDE_PLUGIN_ROOT}` resolved to the cache path; `SkipCapability` for unmapped hooks/agents | Parse `.mcp.json` → `MergeJson` into Copilot `mcp.json`; place `skills/`; `SkipCapability` for hooks/agents with a warning |
| Cached / local (introspectable), ephemeral (`claude --profile`) | Launch `claude --plugin-dir <cache-path>` — full native plugin load (skills + MCP + hooks) for the session only; nothing to clean up | (Copilot ephemeral uses the persistent decomposition into a temp `local` scope) |
| Delegate fallback (opaque) | `MergeJson`: register marketplace + `enabledPlugins."name@marketplace" = true` in the scoped `settings.json` | `SkipCapability` (cannot introspect) |

Bundle reading lives in shared `plugin.rs`; each applicator decides where the introspected
parts land.

## Plan model

Reuse and extend `ProfileAction`:

- **Cached/local plugin (Claude, persistent)** → decompose: `WriteFile`/symlink each
  `skills/<skill>` into `.claude/skills/`, and `MergeJson` the `.mcp.json` servers into the
  project `.mcp.json` (resolving `${CLAUDE_PLUGIN_ROOT}` to the cache path). Recorded like
  overlay file entries plus owned JSON paths.
- **Cached/local plugin (Claude, ephemeral)** → no placement action; the runner launches
  `claude --plugin-dir <cache-path>`. Nothing recorded; nothing to remove.
- **Cached/local plugin (Copilot)** → `MergeJson` for the bundled `.mcp.json` servers, plus
  `WriteFile` for skills placement.
- **Delegate fallback** → `MergeJson { target: <scoped settings.json>, value, scope }`
  writing the known-marketplace entry and the `enabledPlugins` key.
- **Unsupported parts** → `SkipCapability { capability, reason }`.

### Generalized JSON ownership (delegate path)

The current `MergeJson` cleanup is specialized to the MCP `servers` key. Generalize it so
state records the exact JSON locations a profile owns, as JSON Pointer–style paths (not
ambiguous dotted strings — plugin and marketplace names may contain `.`, `/`, or `@`):

```text
target = .claude/settings.json
path   = /enabledPlugins/rust-dev@playground
prior  = <absent | previous value>
wrote  = true
```

## State

Profile state records plugin provenance with canonical identity and a resolved commit
(managed path) or owned settings entries (delegate path):

```ccl
plugins =
  =
    marketplace = playground
    name = rust-dev
    install = managed
    resolved_commit = 3c1f0f8e...        # pin recorded like overlay sources
    placed =
      =
        target = .claude/skills/rust-dev          # decomposed skill (symlink/copy)
        action = symlink
      =
        target = .mcp.json#/mcpServers/rust-dev   # owned JSON path (CLAUDE_PLUGIN_ROOT resolved)
        action = merge_json
  =
    marketplace = vendor
    name = cool
    install = delegate
    owned_settings =
      =
        target = .claude/settings.local.json
        path = /enabledPlugins/cool@vendor
        prior = absent
```

## Lifecycle and cleanup

### Managed (cached) plugins — overlay-like

- **Apply**: ensure cached (clone + pin commit), introspect, place into the owned harness
  dir / merge into harness config, record placement + commit.
- **Remove / ephemeral cleanup**: remove placed files/symlinks and unmerge any harness
  config entries, by recorded placement — exactly like overlay removal.
- **Update**: `repoverlay update` re-resolves the source, re-clones if the commit changed,
  and re-applies — the same flow overlays already use.
- **Cache refcounting**: a cached bundle may be referenced by multiple profiles; cache
  eviction follows the existing `cache` subcommand semantics and is independent of any one
  profile's removal.

### Delegate plugins — conflict-aware settings unmerge

- **Conflict-aware unmerge**: if a key existed before repoverlay wrote it (user already
  enabled the plugin), restore the prior value; if absent before, remove it only when the
  current value still equals what repoverlay wrote; otherwise warn and leave it.
- **Marketplace refcounting**: do not unregister a marketplace another enabled plugin still
  references; only remove a repoverlay-created marketplace entry when unused.
- **Source conflict**: if a marketplace name already maps to a different source, fail with
  an ownership/source conflict instead of overwriting.

## Scope defaults (delegate path only)

| Mode | Default scope | Rationale |
| --- | --- | --- |
| Persistent (`profile apply`) | `project` | Team-shareable, explicit in plan output. |
| Ephemeral (`copilot`/`claude --profile`) | `local` | Never dirty the tracked `.claude/settings.json` for a transient session. |

Managed plugins do not register marketplaces or write `enabledPlugins`, so scope does not
apply to them; their placement is decomposed into native harness locations
(`.claude/skills/`, project `.mcp.json`), or — for ephemeral sessions — loaded via
`--plugin-dir` with no on-disk placement at all.

## Security and trust

Enabling a plugin introduces executable behavior (hooks, MCP servers, agents) — a stronger
trust boundary than overlay files or inline MCP config.

- Validate marketplace `url` schemes (`https://`, `ssh://`, `git@`, or a safe `owner/repo`
  shorthand) when registered, reusing repoverlay's existing URL validation. Validate local
  plugin `path` sources against traversal, consistent with instruction-source validation.
- Pin managed plugins to a resolved commit (recorded in state); support `ref`.
- Record marketplace identity (`name` + `url`) and resolved commit in state.
- Treat marketplace registration as a confirmation point: prompt before `marketplace add`
  registers a new source (skippable with `--yes`/non-interactive), and fail when a
  marketplace `name` is re-registered with a different `url`.

## Commands

Extend the profile command group:

```bash
repoverlay marketplace add <name> <url>   # register a marketplace (mirrors `source add`)
repoverlay marketplace list               # list registered marketplaces
repoverlay profile show <name>            # render plugins (marketplace/name, install, resolved commit / scope)
repoverlay claude --profile <name>        # ephemeral Claude session (mirrors `copilot --profile`)
repoverlay plugin new <name>              # scaffold a local plugin (.claude-plugin/plugin.json + stub .mcp.json)
repoverlay update                         # re-resolve + re-apply managed plugins (and overlays)
repoverlay cache ...                      # already manages cached repos; now also caches marketplaces
```

`marketplace add`/`list` mirror the existing `source add`/`source list` commands, writing the
`marketplaces` registry. `profile show` replaces the old `MCP servers` / `Skills` sections
with a `Plugins` section listing each reference's `marketplace/name` (or local path),
`install` mode, and resolved commit (managed) or `scope` (delegate).

## Ergonomics

Expressing a single MCP server as a plugin directory is heavier than the old inline block.
Mitigations:

- `repoverlay plugin new <name>` scaffolds the directory + manifest + stub `.mcp.json`.
- Local plugins can live in the repo's overlay library and be referenced by name.

An inline-MCP shorthand that desugars into a generated plugin is intentionally out of scope:
it would reintroduce the duplication this spec removes.

## Validation spike (resolved)

A spike against Claude Code 2.1.154 settled the placement mechanism:

- **`claude --plugin-dir <path>`** fully loads a cached bundle for one session. The
  `stream-json` init event reported `plugins: [{name, path, source: "<name>@inline"}]`, the
  bundled skill as a slash command (`<name>:<skill>`), and the bundled MCP server
  (`plugin:<name>:<server>`). This is the **ephemeral** mechanism — native, zero cleanup.
- **A plugin folder dropped under `.claude/skills/` does NOT auto-load** on a plain launch
  (init event listed none of its components). The earlier assumption of a "skills-directory
  plugin" auto-load is therefore **false** and has been removed from this spec.
- **A bare `.claude/skills/<skill>/SKILL.md` DOES auto-load** on a plain launch (it appeared
  in the init `skills` array). This is the basis for **persistent** placement: repoverlay
  decomposes the cached bundle into bare skills plus native `.mcp.json` entries.
- Because `${CLAUDE_PLUGIN_ROOT}` is only substituted inside a real plugin load, repoverlay
  resolves it to the cache path itself when writing decomposed MCP entries into a plain
  `.mcp.json`.

Net effect on the design: persistent Claude placement **decomposes** (it does not copy the
whole bundle), symmetric with the Copilot applicator; ephemeral Claude placement uses
`--plugin-dir`. The delegate path remains the fallback for opaque/non-git sources only.

## Testing

Unit tests:

- Parsing the `marketplaces` registry and the typed `plugins` field (shorthand
  `marketplace/plugin`, local path, expanded table); rejecting references to unregistered
  marketplaces and otherwise-invalid entries.
- `mcps`/`skills` fields are removed (configs using them fail to parse or are ignored per
  the chosen behavior).
- Marketplace resolution: registry lookup → `marketplace.json` → plugin `source` (subdir,
  external repo, non-git → delegate).
- Cached-plugin introspection: a bundle with `.mcp.json` plans an `mcp.json` merge for
  Copilot and, for persistent Claude, a `.claude/skills/` decomposition plus a project
  `.mcp.json` merge with `${CLAUDE_PLUGIN_ROOT}` resolved to the cache path.
- Commit pinning recorded in state; `ref` honored.
- Delegate path: generalized JSON ownership, conflict-aware unmerge, marketplace
  refcounting, source-scheme validation.

Integration tests:

- `profile show` renders the `Plugins` section.
- `profile apply`/`remove` for a managed marketplace plugin caches, places, and cleanly
  removes the placement.
- `repoverlay update` re-applies a managed plugin when its source commit changes.
- Ephemeral `claude --profile` launches `claude --plugin-dir <cache-path>` (full native
  load) and preserves the harness exit code; no on-disk placement to remove.
- A local plugin shipped via an overlay is enabled through the profile.
- Delegate fallback writes and cleanly removes scoped `settings.json` entries.

## Migration

The profiles feature is unreleased, so removing `mcps` and `skills` is not a breaking change
to any shipped release. No deprecation window is required. Update the profiles guide and the
`profile show` output to reflect the plugin-only model.

## Implementation slice

1. Add the `marketplaces` registry to `RepoverlayConfig` (mirroring `sources`, with
   `marketplace add`/`list` commands) and the `PluginRef` typed model; remove `mcps`/`skills`
   from `ProfileConfig`; update parsing, merge, and `profile show`.
2. Add `plugin.rs`: registry resolution of `marketplace/plugin` references + `marketplace.json`
   resolution via `CacheManager`, commit pinning, `.mcp.json` and `skills/` introspection.
3. Add the Claude applicator: for persistent profiles, decompose a cached/local bundle into
   `.claude/skills/` + project `.mcp.json` (resolving `${CLAUDE_PLUGIN_ROOT}` to the cache
   path); record skill placements and owned JSON paths.
4. Teach the Copilot applicator to introspect cached/local plugins (`.mcp.json` → `mcp.json`,
   `skills/` placement) and skip what it can't map.
5. Wire managed-plugin lifecycle into `profile apply`/`remove`/`update` reusing overlay-style
   placement + cache refcounting.
6. Add the delegate fallback (settings.json enablement) with generalized JSON ownership,
   conflict-aware unmerge, marketplace refcounting, and trust handling.
7. Add `repoverlay claude --profile <name>` ephemeral execution via `claude --plugin-dir
   <cache-path>` (full native load, no on-disk placement).
8. Add `repoverlay plugin new <name>` scaffolder.
