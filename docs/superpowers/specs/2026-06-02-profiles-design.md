# Profiles Design

## Summary

Profiles are first-class, agent-centric configurations that compose overlays and other AI harness capabilities into a loadable unit. A profile describes intent, not file placement. Applying a profile always happens for a specific agent harness, such as GitHub Copilot or Claude, and that harness maps profile objects to concrete repo-local or user-level locations.

The first implementation should build the GitHub Copilot harness applicator. The design should still support additional harnesses without changing the profile schema.

## Goals

- Define named profiles in the main repoverlay CCL config.
- Let profiles compose existing overlays without replacing overlay behavior.
- Treat MCP servers, skills, plugins, and harness/user-level instruction files as first-class
  profile objects.
- Keep profiles harness-neutral and move placement logic into harness applicators.
- Support both repo-local and user-level/global harness locations.
- Track profile lifecycle and state separately from overlay state.
- Warn and skip capabilities unsupported by the selected harness.

## Non-goals

- Do not add MCP built-ins in v1. MCPs are explicit server definitions.
- Do not add grouped `default` plugin or skill channels in v1. Skills and plugins are plain lists.
- Do not make profiles arbitrary harness-specific config blobs.
- Do not use profile `instructions` for repo-level instruction files. Repo-level files such as
  `AGENTS.md` should be supplied by overlays.
- Do not require all harnesses to support every profile object.

## Profiles vs. overlays

Overlays remain file-tree bundles applied to git repositories. They own source resolution, mappings, symlink/copy behavior, git exclude updates, conflict handling, and overlay state.

Profiles are higher-level compositions. They can reference overlays, but they also express agent capabilities that are not naturally overlays: MCP servers, marketplace skills, plugins, and harness/user-level instruction files. Repo-level instruction files such as `AGENTS.md` should be represented as overlay files, because overlays already own repo-local file placement, conflict handling, git exclusion, and removal. Profiles do not encode where harness/user-level objects are written. A harness applicator decides that.

This keeps overlays reusable and makes profiles portable across agent harnesses.

## Configuration schema

Profiles live in `RepoverlayConfig` beside existing fields such as `sources` and `library_path`.

```ccl
profiles =
  oce =
    description = On-call engineer profile
    overlays =
      = oce-base

    mcps =
      servers =
        icm =
          command = uvx
          args =
            = mcp-icm
        yammer =
          command = uvx
          args =
            = mcp-yammer
        kusto =
          command = uvx
          args =
            = mcp-kusto

  rust-dev =
    description = Rust development profile
    overlays =
      = rust-base
      = rust-tools

    instructions =
      =
        source = copilot-instructions.md

    skills =
      = market:rust-reviewer@playground
      = local:skills/rust-debugging

    plugins =
      = market:rust-dev@playground
```

### Fields

- `description`: Optional user-facing metadata.
- `overlays`: Overlay references resolved with existing library and source semantics.
- `instructions`: Harness/user-level instruction entries. Each entry has a `source`; the harness
  decides the target, merge behavior, and whether the instruction type is supported. Repo-level
  instruction files belong in overlays instead.
- `mcps.servers`: User-defined MCP servers. Each server has a name and fields such as `command`, `args`, `env`, and future transport metadata.
- `skills`: Plain list of skill references.
- `plugins`: Plain list of plugin references.

### Merge behavior

Repo-local config has priority over global config. Profiles merge by profile name when a repo-local
profile has the same name as a global profile. The global profile is the base, and the repo-local
profile is the override.

Merge behavior is type-based:

| Field type | Merge behavior |
| --- | --- |
| Scalar fields, such as `description` | Repo-local value wins when set; otherwise the global value is kept. |
| Map fields, such as `mcps.servers` | Repo-local entries merge into the global map; repo-local keys win on conflict. |
| List fields, such as `overlays`, `instructions`, `skills`, `plugins`, and future `remote_config` | Repo-local list replaces the global list when non-empty; otherwise the global list is kept. |

V1 does not include `mcps.builtins` or grouped list fields such as `plugins.default`. If those fields
are added later, they should follow the same map/list rules: built-in maps merge by key, and grouped
lists replace the base list when the overriding list is non-empty.

## Architecture

Add focused modules rather than growing `lib.rs`:

```text
src/profile.rs
src/profile_plan.rs
src/profile_applicators/mod.rs
src/profile_applicators/copilot.rs
src/cli/commands/profile.rs
```

`profile.rs` owns config structs, lookup, validation, and profile state types.

`profile_plan.rs` converts a profile plus harness into an explicit `ProfilePlan`. Planning should be filesystem-light and easy to unit test.

`profile_applicators/` contains one applicator per harness. The first real applicator is GitHub
Copilot. Applicators map harness-neutral profile objects to concrete actions, destinations, merge
behavior, warnings, skips, and harness execution.

## Applicator trait

Profiles use a two-phase flow: plan, then apply.

```rust
pub(crate) trait ProfileApplicator {
    fn harness(&self) -> AgentHarness;
    fn capabilities(&self) -> HarnessCapabilities;
    fn plan(&self, profile: &ResolvedProfile, context: &ProfileContext) -> Result<ProfilePlan>;
    fn apply(&self, plan: &ProfilePlan) -> Result<ProfileApplyResult>;
    fn remove(&self, state: &ProfileState, context: &ProfileContext) -> Result<()>;
    fn command(&self, context: &ProfileContext, extra_args: &[String]) -> Result<Command>;
}
```

Overlay application should stay in shared profile code, not inside each applicator. The shared planner resolves profile overlays into existing resolved overlay values and applies them using existing overlay functions. Applicators own harness-specific placement, merge logic, and launch command behavior for MCPs, skills, plugins, and instruction files.

Unsupported capabilities produce warnings and skipped plan items. Conflicts and failed writes still fail the apply by default.

## Plan model

`ProfilePlan` should make every side effect explicit:

```rust
pub(crate) enum ProfileAction {
    ApplyOverlay { reference: String },
    WriteFile { source: PathBuf, target: PathBuf, scope: ProfileScope },
    MergeJson { target: PathBuf, value: serde_json::Value, scope: ProfileScope },
    InstallRef { reference: String, kind: InstallKind },
    SkipCapability { capability: String, reason: String },
    Warn { message: String },
}
```

The exact action enum can change during implementation, but the important boundary is that planning explains what will happen before applying it.

## State

Profiles get their own state because the applied unit is profile plus harness plus generated side effects.

Repo-local state:

```text
.repoverlay/profiles/<profile-name>.<harness>.ccl
```

External backup/state for user-level side effects:

```text
~/.local/share/repoverlay/profiles/<target-hash>/<profile-name>.<harness>.ccl
```

State records the applied profile, harness, fingerprint, overlays applied by this profile, files or merged config entries owned by this profile, and skipped capabilities.

```ccl
name = rust-dev
harness = copilot
applied_at = 2026-06-02T15:00:00Z
profile_fingerprint = sha256:3c1f0f8e8a1f4b9c
overlays =
  = rust-base
files =
  =
    source = copilot-instructions.md
    target = ~/.config/github-copilot/instructions.md
    scope = user
    action = write-file
skipped =
  =
    capability = plugins
    reason = unsupported-by-harness
```

Removal should remove profile-owned harness files or merged entries and remove overlays only when the profile applied them and no other applied profile still references them.

Ephemeral profile execution should use the same state schema with an additional session marker:

```ccl
mode = ephemeral
session_id = 2026-06-02T15:30:00Z-copilot-rust-dev
```

Ephemeral state is system-managed metadata. It exists only to make cleanup reliable while the harness
process is running or if repoverlay is interrupted.

## Commands

Add a `profile` command group:

```bash
repoverlay profile list
repoverlay profile show <name>
repoverlay profile apply <name> --harness copilot
repoverlay profile status [--harness copilot]
repoverlay profile remove <name> --harness copilot
```

Add harness-specific execution entrypoints. For GitHub Copilot:

```bash
repoverlay copilot --profile rust-dev
repoverlay copilot --profile oce -- <extra harness args>
```

`repoverlay copilot --profile <name>` performs an ephemeral profile session:

1. Resolve and plan the profile for the GitHub Copilot harness.
2. Apply the profile using session-scoped state.
3. Launch the Copilot harness command.
4. Wait for the harness process to exit.
5. Remove only the profile effects created by that session.
6. Exit with the harness process exit code unless cleanup fails.

Persistent profile commands and ephemeral harness execution are separate. `profile apply` leaves the
profile installed until `profile remove`. `repoverlay copilot --profile` installs the profile only for
the lifetime of the launched harness process.

`profile update` can come later. The v1 state should include enough source provenance and fingerprinting to support update without changing state format.

## Error handling

- Invalid profile config fails early.
- Missing overlay references fail.
- Unsupported harness capabilities warn and skip.
- File conflicts fail by default.
- Partial apply failures roll back completed actions where practical.
- Successful profile state is written only after required actions succeed.
- Ephemeral execution should attempt cleanup even when the harness exits with a non-zero code.
- If cleanup fails after the harness exits, repoverlay should report the cleanup error and leave
  enough session state for a later cleanup command or `profile remove`.

## Testing

Unit tests should cover:

- Profile config parsing and validation.
- Config precedence between global and repo-local profiles.
- Planning for the GitHub Copilot applicator.
- Warning and skip behavior for unsupported capabilities.
- Harness/user-level instruction file planning.
- MCP server planning.
- Ephemeral Copilot command construction.

Integration tests should cover:

- `profile list`, `profile show`, `profile apply`, `profile status`, and `profile remove`.
- Profile state file creation and removal.
- Overlay references applied through a profile.
- User-level action recording and repo-local overlay recording.
- Rollback or non-success state behavior on failed apply.
- `repoverlay copilot --profile <name>` applies the profile, runs the harness command, removes the
  session effects, and preserves the harness exit code.

## Initial implementation slice

1. Add profile config structs and parsing.
2. Add profile command group with `list` and `show`.
3. Add `ProfileApplicator` trait, plan model, and a dummy test applicator.
4. Add GitHub Copilot applicator.
5. Add `profile apply` for overlays and harness/user-level instruction files.
6. Add MCP server planning for GitHub Copilot.
7. Add profile state and `profile status`/`profile remove`.
8. Add `repoverlay copilot --profile <name>` with ephemeral apply/run/remove behavior.
9. Add skills and plugins once the GitHub Copilot placement semantics are clear.
