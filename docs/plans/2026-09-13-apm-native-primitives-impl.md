# APM native primitives implementation plan

Status: Implemented

Date: 2026-09-13

## Goal

Extend `repoverlay apm install` to apply APM commands, prompts, instructions, and
hooks for Claude Code and GitHub Copilot CLI.

Keep the existing responsibility boundary:

| Concern | Owner |
|---------|-------|
| Package resolution, policy, target conversion, and lockfiles | APM |
| Durable artifact storage, file ownership, conflicts, removal, and restore | repoverlay |

Repoverlay must consume the target-native files that `apm install` creates in
the temporary project. It must not reimplement APM's primitive conversion.

## Supported output

| APM output | Claude target | Copilot target |
|------------|---------------|----------------|
| Commands or prompts | `.claude/commands/*.md` | `.github/prompts/*.prompt.md` |
| Instructions | `.claude/rules/*.md` | `.github/instructions/*.instructions.md` |
| Hooks | `hooks` in `.claude/settings.json` | `.github/hooks/` |

The existing plugin export remains the source for skills and agents. The
resolved MCP configuration continues to use the separate managed JSON path.

## Artifact layout

Store captured native output inside each durable APM artifact:

```text
<external-state>/apm/<package>/<harness>/<package>/
  plugin.json
  skills/
  agents/
  .mcp.json
  .repoverlay-native/
    files/
      .claude/commands/...
      .claude/rules/...
      .github/prompts/...
      .github/instructions/...
      .github/hooks/...
    claude-hooks.json
```

Only copy paths from the allowlist in the table above. Reject symlinks and any
path that escapes the temporary project. Do not copy the complete temporary
project.

## Task 1: Capture target-native APM output

**Files:**

- Modify: `src/cli/commands/apm.rs`
- Test: `src/cli/commands/apm.rs`

Add a helper that runs after `apm install` and before `apm pack`. It must:

1. Select the allowlisted source paths for the chosen harness.
2. Copy regular files and directories into a temporary
   `.repoverlay-native/files/` capture directory.
3. Preserve executable permissions on hook scripts.
4. Reject symlinks in the source tree.
5. Extract only the top-level `hooks` object from Claude's temporary
   `.claude/settings.json`.
6. Store the extracted object in `.repoverlay-native/claude-hooks.json`.
7. Return the capabilities that it captured so the profile does not report
   them as skipped.

After `apm pack`, copy the capture directory into the bundle staging directory
before the staging directory becomes the active durable artifact.

Do not capture unrelated Claude settings. Do not capture APM manifests,
lockfiles, caches, or Git metadata through this path.

Test with an APM executable stub that creates every supported native output.
Verify the durable artifact contains only allowlisted files.

Run:

```bash
cargo test cli::commands::apm::tests
```

## Task 2: Represent resolved native output in an in-memory profile

**Files:**

- Modify: `src/profile.rs`
- Modify: `src/profile_applicators/mod.rs`
- Test: `src/profile.rs`

Add runtime-only profile fields, using `#[serde(skip)]`, for:

```text
resolved_placements
resolved_json_merges
handled_plugin_capabilities
```

Each resolved placement records its durable source and repository-relative
target.
Each JSON merge records its target, value, and owned JSON pointers.

Keep these fields out of CCL. APM creates them during `apm install`; ordinary
profiles must continue to use the public profile schema.

Extend `AgentHarness` with the native managed roots listed in this plan. Broaden
the existing `PlacePluginDir` validation so it accepts one direct child under
skills, agents, commands, prompts, instructions, or hooks roots. The action
already supports files and directories and already participates in snapshot,
removal, and restore. Keep its serialized snapshot representation compatible.

Add shared planning helpers that convert `resolved_placements` and
`resolved_json_merges` into existing `PlacePluginDir` and `MergeJson` actions.
Do not add a second placement implementation.

Reject:

- Absolute repository targets.
- `..` components.
- Targets outside the harness allowlist.
- Duplicate targets in one profile.

Run:

```bash
cargo test profile::
cargo test profile_applicators::
```

## Task 3: Apply commands, prompts, and instructions

**Files:**

- Modify: `src/cli/commands/apm.rs`
- Modify: `src/profile_applicators/claude.rs`
- Modify: `src/profile_applicators/copilot.rs`
- Modify: `src/plugin.rs`
- Test: the same modules

Map captured files without changing their contents:

```text
Claude:
  .claude/commands/ -> .claude/commands/
  .claude/rules/    -> .claude/rules/

Copilot:
  .github/prompts/      -> .github/prompts/
  .github/instructions/ -> .github/instructions/
```

Use one managed placement per command, prompt, or instruction file. This gives
each file independent conflict detection and restoration.

Stop reporting `commands` and `instructions` as skipped when the APM native
capture handled them. Continue to report those capabilities as unsupported for
ordinary plugin bundles until repoverlay defines a reliable cross-harness
mapping for them.

Tests must cover:

- Claude command installation and removal.
- Copilot prompt installation and removal.
- Instructions with target-specific frontmatter preserved byte-for-byte.
- Existing target files backed up and restored.
- Two profiles that claim the same target.
- Restore after `.repoverlay/` and harness files are removed.

Run:

```bash
cargo test cli::commands::apm::tests
cargo test profile_applicators::
cargo test profile_plan::
```

## Task 4: Apply Copilot hooks

**Files:**

- Modify: `src/cli/mod.rs`
- Modify: `src/cli/commands/apm.rs`
- Modify: `src/profile_applicators/copilot.rs`
- Test: the same modules and `tests/cli.rs`

Add `--allow-hooks` to `repoverlay apm install`. If the package produces hooks
and the flag is absent, stop before writing the durable artifact or target
repository. Tell the user to inspect the package and rerun with
`--allow-hooks`.

When consent is present:

1. Copy `.github/hooks/*.json` and their script files from the native artifact.
2. Preserve executable permissions.
3. Replace absolute paths rooted in the temporary project with paths rooted in
   the durable artifact.
4. Reject absolute paths outside the temporary project.
5. Apply each hook file through managed placement.

Do not execute hook scripts during installation.

Tests must cover missing consent, path rewriting, executable permissions,
conflicts, removal, and restore.

Run:

```bash
cargo test cli::commands::apm::tests
cargo test --test cli apm
```

## Task 5: Apply Claude hooks

**Files:**

- Modify: `src/cli/commands/apm.rs`
- Modify: `src/profile_applicators/claude.rs`
- Modify: `src/profile_applicators/mod.rs`
- Modify: `src/profile_plan.rs` only if existing JSON ownership cannot express
  the required conflict rule
- Test: the same modules

Require the same `--allow-hooks` consent.

Merge the captured object into `.claude/settings.json` under `hooks`. Own one
pointer per event:

```text
/hooks/PreToolUse
/hooks/PostToolUse
/hooks/Stop
```

For the first version, one profile owns the complete array for an event. Reject
another profile that claims the same event. Do not add element-level array
merging in this phase.

Rewrite command paths before the merge:

- Replace temporary-project prefixes with the durable artifact path.
- Keep relative commands relative.
- Reject absolute paths outside the temporary project.

Use the existing JSON backup and restore behavior so user settings return after
profile removal. Reject symlinked settings targets through the existing
preflight checks.

Tests must cover:

- Each supported hook event.
- Existing unrelated settings preserved.
- Existing values under an owned event restored after removal.
- Conflicts between two profiles.
- Temporary path rejection and rewriting.
- Restore from the external profile snapshot.

Run:

```bash
cargo test profile_applicators::claude
cargo test profile_plan::
cargo test cli::commands::apm::tests
```

## Task 6: Complete lifecycle and documentation

**Files:**

- Modify: `README.md`
- Modify: `DEV.md`
- Modify: `ARCHITECTURE.md`
- Modify: generated CLI reference through the repository's existing process
- Modify: `docs/plans/2026-09-10-apm-integration-plan.md`

Document:

- Supported APM primitives for each harness.
- The `--allow-hooks` requirement.
- Managed target paths.
- Conflict behavior.
- Removal and restore behavior.
- The remaining limits for ordinary non-APM plugin bundles.

Update the original APM plan's outcome and remove commands, hooks, and
instructions from its unsupported list.

Run:

```bash
just check
```

Create a changie entry because this changes the published CLI.

## Acceptance criteria

- One APM package can install skills, agents, MCP servers, commands or prompts,
  instructions, and hooks for its selected harness.
- Repoverlay uses APM's target-native output without translating primitive
  formats.
- Hook installation requires explicit consent.
- No stored command references the deleted temporary project.
- A failed install leaves the previous applied profile usable.
- Removal restores pre-existing files and JSON values.
- Restore works without running APM or resolving dependencies again.
- Conflicting profile ownership fails before target files change.
- Unsupported ordinary plugin capabilities still produce explicit warnings.

## Deferred work

- Element-level composition of hook arrays from multiple profiles.
- User-scope APM installation.
- Generic commands, prompts, instructions, or hooks for non-APM plugin bundles.
- Additional APM targets beyond Claude and Copilot.

Add these only after a concrete use case requires them.

## References

- [APM targets matrix](https://microsoft.github.io/apm/reference/targets-matrix/)
- [APM hooks and commands](https://microsoft.github.io/apm/producer/author-primitives/hooks-and-commands/)
- [Existing APM integration plan](2026-09-10-apm-integration-plan.md)
