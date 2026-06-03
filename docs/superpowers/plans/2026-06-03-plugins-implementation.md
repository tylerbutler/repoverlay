# Plugins Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [x]`) syntax for tracking.

**Goal:** Implement first-class, cached, introspectable plugins for profiles per
[2026-06-03-plugins-design.md](../specs/2026-06-03-plugins-design.md): add a named
`marketplaces` registry, a typed `plugins` field, remove inline `mcps`/`skills`, cache and
introspect plugin bundles via the existing overlay cache, place them per-harness (persistent
Claude: decompose into `.claude/skills/` + `.mcp.json`; ephemeral Claude: `--plugin-dir`;
Copilot: `mcp.json`/skills), with a delegate-to-Claude fallback, plus state, removal,
ephemeral `claude --profile`, and a `plugin new` scaffolder.

**Architecture:** Reuse `CacheManager` (`src/cache.rs`) and `GitHubSource` (`src/github.rs`)
for fetching/pinning marketplaces. Add `src/plugin.rs` for reference resolution and bundle
introspection. Extend `RepoverlayConfig` (`src/config.rs`) with `marketplaces`. Extend the
profile applicator trait with a Claude applicator. Generalize the `MergeJson` ownership model
in `src/profile_plan.rs`. CLI wiring stays in `src/cli/mod.rs` + handlers under
`src/cli/commands/`.

**Tech Stack:** Rust 2024, clap derive, serde + sickle CCL, serde_json, anyhow, `git` via
`std::process::Command`, assert_cmd integration tests, `just test` / `just check`.

---

## Pre-requisite: validation spike (DONE — Claude Code 2.1.154)

Outcome recorded in the spec's "Validation spike (resolved)" section. Key findings that bind
the tasks below:

- **`claude --plugin-dir <path>` fully loads a cached bundle** (skills + MCP + hooks) for one
  session, sourced as `<name>@inline`. → Ephemeral Claude (Task 9) uses this; no placement.
- **A plugin folder under `.claude/skills/` does NOT auto-load** on a plain launch. The
  "skills-directory plugin" mechanism does not exist; persistent placement must **decompose**.
- **A bare `.claude/skills/<skill>/SKILL.md` DOES auto-load.** → Persistent Claude (Task 5)
  decomposes the bundle: each `skills/<skill>` → `.claude/skills/<skill>`, `.mcp.json` servers
  → project `.mcp.json` with `${CLAUDE_PLUGIN_ROOT}` resolved by repoverlay to the cache path.
- Tasks 5/7 reflect decomposition (not whole-bundle placement); Task 9 reflects `--plugin-dir`.

## File structure

- Modify `src/config.rs`: add `marketplaces: Vec<Marketplace>` to `RepoverlayConfig`; add the
  `Marketplace` struct (mirrors `Source`, lines 95–111); merge-by-name with repo-local
  override; remove `mcps`/`skills` usage from profile tests.
- Modify `src/profile.rs`: replace `mcps: McpConfig` + `skills: Vec<String>` with
  `plugins: Vec<PluginRef>`; delete `McpConfig`/`McpServerConfig`; update `merge_profile_config`
  and `ProfileState` plugin records.
- Create `src/plugin.rs`: `PluginRef` parsing (`marketplace/plugin`, local path, expanded
  table), registry resolution, `CacheManager`-based fetch, `marketplace.json` + `plugin.json`
  + `.mcp.json` + `skills/` introspection, bundle validation.
- Modify `src/profile_plan.rs`: generalize `MergeJson` ownership to JSON-pointer paths;
  add plugin placement actions; wire managed-plugin apply/remove; delegate fallback.
- Modify `src/profile_applicators/mod.rs`: add `AgentHarness::Claude`; register applicators.
- Create `src/profile_applicators/claude.rs`: Claude applicator (persistent: decompose into
  `.claude/skills/` + `.mcp.json`; delegate enablement fallback).
- Modify `src/profile_applicators/copilot.rs`: introspect plugins → `mcp.json` merge + skills.
- Create `src/cli/commands/marketplace.rs`: `marketplace add/list/remove` (mirror `source.rs`).
- Create `src/cli/commands/claude.rs`: `repoverlay claude --profile <name>` (mirror
  `copilot.rs`).
- Modify `src/cli/commands/profile.rs`: render `Plugins` section in `show`.
- Modify `src/cli/commands/cache.rs`: include marketplaces in `update`/cache views.
- Modify `src/cli/mod.rs` + `src/cli/commands/mod.rs`: add `MarketplaceCommand`, `Claude`
  command, `plugin new`, and dispatch.
- Modify `src/lib.rs`: register `plugin` module; re-export new functions used by CLI.
- Modify `tests/cli.rs` + `tests/common/mod.rs`: registry/plugin fixtures and assertions.
- Modify `website/src/content/docs/guides/profiles.md`: plugin-only model + marketplaces.

Test env overrides to reuse/add:
- `REPOVERLAY_COPILOT_HOME`, `REPOVERLAY_COPILOT_COMMAND` (existing).
- Add `REPOVERLAY_CLAUDE_HOME`, `REPOVERLAY_CLAUDE_COMMAND` for the Claude harness.
- Local marketplace fixtures: a `tempfile` git repo containing `.claude-plugin/marketplace.json`
  and a plugin subdir, referenced by `owner/repo` shorthand or a `file://`/path source.

---

## Task 1: Marketplace registry + PluginRef model; remove mcps/skills

**Files:** Modify `src/config.rs` (around 14–26, 95–111, profile tests 660–678), modify
`src/profile.rs` (13–85), create `src/plugin.rs` (types only).

- [x] **Step 1: Failing tests** — In `src/plugin.rs`, add tests:
  - Parse a `PluginRef` from CCL shorthand `playground/rust-dev` → `{ marketplace:
    "playground", name: "rust-dev", install: Managed, ref: None, scope: None }`.
  - Parse `./plugins/local-mcp` → `PluginRef::Local { source }`.
  - Parse the expanded table (with `marketplace`, `name`, `ref`, `install = delegate`,
    `scope = local`).
  - Reject a shorthand with >1 `/` segment after the marketplace name and an empty side.
  In `src/config.rs` profile tests, add: parse a `marketplaces` registry (`name` + `url`,
  `owner/repo` shorthand expands to `https://github.com/owner/repo`); a repo-local
  marketplace with the same `name` overrides the global `url`.

- [x] **Step 2: Implement types** —
  - `src/plugin.rs`: define `PluginRef` as an enum or struct-with-kind. Recommended:
    ```rust
    pub(crate) enum PluginRef {
        Marketplace { marketplace: String, name: String, r#ref: Option<String>,
                      install: InstallMode, scope: Option<ProfileScope> },
        Local { source: PathBuf },
    }
    pub(crate) enum InstallMode { Managed, Delegate }
    ```
    Implement a custom `Deserialize` that accepts a bare string (`marketplace/plugin` or a
    path starting with `.`/`/`) or a map. `InstallMode` defaults to `Managed`.
  - `src/config.rs`: add `Marketplace { name: String, url: Option<String> }` mirroring
    `Source` (reuse `deserialize_optional_source_url`); add `marketplaces: Vec<Marketplace>`
    to `RepoverlayConfig`; merge with the same name-keyed override rule used for sources.
  - `src/profile.rs`: remove `mcps`/`skills` fields and `McpConfig`/`McpServerConfig`;
    replace `plugins: Vec<String>` with `plugins: Vec<PluginRef>`; update
    `merge_profile_config` (plugins follow the list-replace rule).

- [x] **Step 3:** Update all references that read `profile.mcps`/`profile.skills` (compiler
  will flag them): `src/profile_applicators/copilot.rs` (MCP merge + skip blocks),
  `src/cli/commands/profile.rs:49–56`, and existing profile/config tests. Temporarily make
  Copilot skip plugins until Task 6.

- [x] **Step 4:** `just test` (parsing/merge tests pass); `just lint`.

## Task 2: `marketplace` CLI command group

**Files:** Create `src/cli/commands/marketplace.rs`; modify `src/cli/mod.rs` (add
`MarketplaceCommand` near `SourceCommand` ~566 and dispatch), `src/cli/commands/mod.rs`.

- [x] **Step 1: Failing integration test** in `tests/cli.rs`: `repoverlay marketplace add
  playground owner/repo` writes the registry to config; `marketplace list` prints it;
  re-adding the same name with a different URL fails; `marketplace remove` deletes it.

- [x] **Step 2:** Implement `handle_marketplace_command` mirroring `handle_source_command`
  (`src/cli/commands/source.rs`): load config, validate URL scheme via existing validation,
  reject name re-registration with a conflicting URL, save config. Add a confirmation prompt
  for `add` (skippable with a `--yes` flag / non-interactive), per spec security.

- [x] **Step 3:** `just test`; `just lint`.

## Task 3: Plugin reference resolution + caching + introspection

**Files:** `src/plugin.rs` (resolution + introspection); reuse `src/cache.rs`
`ensure_cached` (98–146) and `src/github.rs` `GitHubSource` (12–33).

- [x] **Step 1: Failing tests** (use a local git fixture marketplace):
  - Resolve `playground/rust-dev` against a registry → cache the marketplace repo → read
    `.claude-plugin/marketplace.json` → locate plugin `rust-dev` → resolve its `source`
    (subdir) → return a `ResolvedPlugin { bundle_dir, resolved_commit }`.
  - Unregistered marketplace name → error.
  - Introspection: a bundle dir with `.mcp.json` yields its MCP servers; a bundle with
    `skills/<name>/SKILL.md` enumerates skill dirs; a bundle with neither yields empty sets.
  - Non-git `marketplace.json` source (e.g. `npm:`) → `ResolvedPlugin` flagged
    `requires_delegate = true` (no bundle_dir).

- [x] **Step 2:** Implement in `src/plugin.rs`:
  - `resolve_plugin(reference, registry, cache, update) -> Result<ResolvedPlugin>`.
  - Map a marketplace `url` + optional plugin subpath into a `GitHubSource` and call
    `CacheManager::ensure_cached`; resolve plugin `source` from `marketplace.json` (subdir →
    subpath in same clone; external git → second `ensure_cached`; non-git → delegate flag).
  - `PluginBundle::read(dir)` parsing `plugin.json`, `.mcp.json` (reuse serde_json), and
    `skills/`.
  - Validate local paths against traversal (mirror `validate_instruction_source` in
    `copilot.rs:14`).

- [x] **Step 3:** `just test`; `just lint`.

## Task 4: Generalize MergeJson ownership to JSON-pointer paths

**Files:** `src/profile_plan.rs` (MergeJson handling + `check_mcp_ownership_conflicts`),
possibly `src/json_merge.rs` (`deep_merge` 29, `merge_json_files` 99).

- [x] **Step 1: Failing tests:** applying a `MergeJson` records the exact JSON-pointer paths
  it created/changed plus prior values; an unmerge restores a prior value when the key
  pre-existed, removes the key when it was absent **and** the current value still equals what
  was written, and warns/leaves it when the current value differs.

- [x] **Step 2:** Generalize the existing MCP-specific ownership/backup logic so a
  `MergeJson` action carries (or the apply step computes) a set of owned JSON-pointer paths,
  stored in `ProfileState`. Implement conflict-aware unmerge. Keep the existing Copilot
  `mcp.json` behavior working (servers under `/servers/<name>`).

- [x] **Step 3:** `just test` (existing Copilot MCP tests still pass); `just lint`.

## Task 5: Claude applicator (persistent bundle decomposition)

**Files:** `src/profile_applicators/mod.rs` (add `AgentHarness::Claude`, register),
create `src/profile_applicators/claude.rs`; `src/profile_plan.rs` (plan actions).

- [x] **Step 1: Failing tests:** the Claude applicator, given a profile with one managed
  `marketplace/plugin`, plans **decomposition** actions: each `skills/<skill>` →
  `<claude-home>/skills/<skill>` (symlink/copy, recorded like overlay file entries), and each
  `.mcp.json` server → a `MergeJson` into the project `.mcp.json` (`/mcpServers/<name>`) with
  `${CLAUDE_PLUGIN_ROOT}` substituted to the resolved cache path; hooks/agents that can't be
  mapped emit `SkipCapability`. Harness home defaults to `~/.claude` and honors
  `REPOVERLAY_CLAUDE_HOME`; `command()` honors `REPOVERLAY_CLAUDE_COMMAND` (default `claude`).
  Plugins with `install = delegate` plan a settings `MergeJson` instead (Task 8 fills
  behavior; here just route).

- [x] **Step 2:** Implement `ClaudeApplicator` mirroring `CopilotApplicator`
  (`copilot.rs`): `harness()`, `plan()`, `command()`, `harness_home_from_env`. Decompose the
  resolved bundle (shared `src/plugin.rs` introspection) into skill placements + `.mcp.json`
  merges, resolving `${CLAUDE_PLUGIN_ROOT}` to the cache path. Register both applicators behind
  the `AgentHarness` dispatch (collapse the `"copilot"` literal per the design note in
  `mod.rs:13`).

- [x] **Step 3:** `just test`; `just lint`.

## Task 6: Copilot applicator plugin introspection

**Files:** `src/profile_applicators/copilot.rs`.

- [x] **Step 1: Failing tests:** given a profile with a managed/local plugin whose bundle has
  a `.mcp.json`, the Copilot applicator plans a `MergeJson` into `mcp.json` for those servers
  and a skills placement for `skills/`; it `SkipCapability`s hooks/agents and delegate
  plugins with a warning.

- [x] **Step 2:** Implement: resolve each plugin via `src/plugin.rs`, read the bundle, map
  `.mcp.json` servers into the existing `mcp.json` merge, place `skills/`, skip the rest.

- [x] **Step 3:** Re-enable the 6 CLI tests `#[ignore]`d in Task 1 by rewriting them to drive
  `MergeJson` through a local plugin fixture (a bundle dir with a `.mcp.json`) instead of the
  removed `mcps` config surface: `copilot_profile_removes_generated_mcp_json_after_cleanup`,
  `copilot_profile_restores_existing_mcp_json_after_cleanup`,
  `profile_remove_preserves_unrelated_mcp_changes`,
  `profile_apply_rolls_back_overlay_when_later_action_fails` (later-failing action now comes
  from a plugin merge into a pre-existing invalid `mcp.json`),
  `profile_apply_rejects_conflicting_mcp_server_ownership`,
  `profile_apply_allows_disjoint_mcp_server_ownership` (last two depend on Task 4's
  generalized ownership).

- [x] **Step 4:** `just test`; `just lint`.

**Files:** `src/profile_plan.rs` (apply/remove orchestration, `apply_profile*`,
`remove_profile*`), `src/profile.rs` (`ProfileState` plugin records), `src/update.rs` and/or
`src/cli/commands/cache.rs`.

- [x] **Step 1: Failing integration tests** (`tests/cli.rs`): `profile apply rust-dev
  --harness claude` caches + places a managed plugin and records `resolved_commit` + placed
  files in state; `profile remove` deletes the placement and cleans empty dirs; re-applying a
  changed source via `repoverlay update` re-places it. Use local fixture marketplaces and
  `REPOVERLAY_CLAUDE_HOME`.

- [x] **Step 2:** Wire plugin placement into the existing plan apply loop (mirror
  `ApplyOverlay` handling in `profile_plan.rs:122`), record `placed`/`resolved_commit`/
  `owned_settings` in `ProfileState`, and implement removal that reverses both placements and
  generalized MergeJson ownership. Add cache refcounting consistent with overlay cache
  semantics. Extend `update` to re-resolve managed plugins.

- [x] **Step 3:** `just test`; `just lint`.

## Task 8: Delegate-to-Claude fallback

**Files:** `src/profile_applicators/claude.rs`, `src/profile_plan.rs`.

- [x] **Step 1: Failing tests:** a plugin with `install = delegate` (or a non-git
  marketplace source) plans a `MergeJson` into the scoped Claude settings file
  (`user`→`~/.claude/settings.json`, `project`→`.claude/settings.json`,
  `local`→`.claude/settings.local.json`) writing the known-marketplace entry and
  `enabledPlugins."name@marketplace" = true`; ephemeral default scope is `local`/`user`,
  persistent default `project`; marketplace refcounting avoids unregistering a shared
  marketplace; a name→different-URL conflict fails.

- [x] **Step 2:** Implement delegate planning + scope→file mapping + the trust checks. Reuse
  the generalized JSON ownership (Task 4) for clean removal.

- [x] **Step 3:** `just test`; `just lint`.

## Task 9: `repoverlay claude --profile <name>` ephemeral execution

**Files:** Create `src/cli/commands/claude.rs` (mirror `copilot.rs`); `src/cli/mod.rs`
(add `Claude` command + dispatch), `src/cli/commands/mod.rs`.

- [x] **Step 1: Failing integration test:** `repoverlay claude --profile rust-dev` resolves
  and caches the profile's plugins, then launches the (overridden) Claude command with a
  `--plugin-dir <cache-path>` flag for each cached bundle (full native load, no on-disk
  placement to clean up), and returns the harness exit code; extra args pass through after
  `--`. Lock-file guarding matches the Copilot flow.

- [x] **Step 2:** Implement `handle_claude_command` mirroring `handle_copilot_command`
  (`copilot.rs:22`): ensure each plugin is cached (reuse `src/plugin.rs` resolution), build the
  `--plugin-dir` args, launch via the Claude command, and propagate the exit code with the
  `wait_for_*`/exit-code helpers. (Delegate-only plugins still go through apply/remove.)

- [x] **Step 3:** `just test`; `just lint`.

## Task 10: `repoverlay plugin new <name>` scaffolder

**Files:** Create `src/cli/commands/plugin.rs`; `src/cli/mod.rs` (`PluginCommand`),
`src/cli/commands/mod.rs`.

- [x] **Step 1: Failing test:** `repoverlay plugin new my-mcp` creates
  `my-mcp/.claude-plugin/plugin.json` (valid manifest) and a stub `my-mcp/.mcp.json`; refuses
  to overwrite an existing directory; validates the name against traversal.

- [x] **Step 2:** Implement the scaffolder writing the manifest + stub `.mcp.json`.

- [x] **Step 3:** `just test`; `just lint`.

## Task 11: `profile show` rendering + docs

**Files:** `src/cli/commands/profile.rs` (`show`, `print_list` 104), website guide.

- [x] **Step 1: Failing test:** `profile show` prints a `Plugins` section listing each
  reference as `marketplace/name` (or local path), `install` mode, and resolved commit
  (managed) or `scope` (delegate); no `MCP servers`/`Skills` sections remain.

- [x] **Step 2:** Implement rendering. Update
  `website/src/content/docs/guides/profiles.md` to the plugin-only model: remove
  `mcps`/`skills` field rows, document the `marketplaces` registry, `marketplace/plugin`
  references, managed vs delegate, scopes, and the Claude harness mapping table. Add a
  `changie new` entry **only if** profiles ship in the same release as a user-facing feature
  (per repo policy, skip changelog for unreleased-only plumbing).

- [x] **Step 3:** `just check` (format + lint + full test suite).

---

## Final verification

- [x] `just check` passes.
- [x] Spec "Validation spike (resolved)" section reflects the spike outcome (done).
- [x] `repoverlay marketplace add`, `profile apply/show/remove --harness claude`,
  `claude --profile`, and `plugin new` exercised end-to-end against a local fixture
  marketplace.
