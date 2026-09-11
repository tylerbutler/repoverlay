# APM integration plan

Status: Phase 1 validated. Native support is not currently required.

Date: 2026-09-10

## Goal

Let developers use [Microsoft Agent Package Manager (APM)](https://github.com/microsoft/apm)
packages through repoverlay without requiring the target repository to adopt APM
or commit agent configuration.

Primary use case: keep personal or team agent dependencies in a separate configuration
repository, then apply them to work and open-source repositories through repoverlay
profiles. Support both persistent application and temporary agent sessions.

## Responsibility boundary

| Concern | Owner |
|---------|-------|
| Dependency resolution, lockfiles, package integrity, and compilation | APM |
| File placement, Git exclusion, applied state, removal, and recovery in the target repository | repoverlay |
| Package policy and trust decisions | Preserve applicable APM controls and explicit user consent |

Use APM as an external tool. Do not reimplement its resolver or introduce a second
dependency lockfile format. Keep repoverlay configuration in CCL; reference APM's
manifest and lockfile rather than translate them into CCL.

Do not let both tools deploy or reconcile the same files in the target repository.
Do not add a generic post-apply hook that runs `apm install` there: its outputs would
escape repoverlay's ownership records, and writes through symlinks could change
shared overlay sources.

## Phase 1 findings

Validated with APM 0.29.1. `apm install --frozen` followed by `apm pack --format claude-plugin --output ./build` produces a standalone plugin bundle with `plugin.json`, `skills/`, `agents/`, `.mcp.json`, `hooks/`, and an embedded `apm.lock.yaml`. The current repoverlay plugin adapter already handles skills, agents, and MCP servers and reports `hooks` and `commands` as unsupported capabilities. APM omitted an unaudited dependency's hooks/MCP content from the packed bundle, so APM policy and attestation controls remain authoritative. No native APM invocation is needed for the supported workflow.

## Starting point

The current repoverlay profile interface supports overlays, instructions, and
Claude-style plugins from local paths or marketplaces. Managed plugin support
includes skills and MCP servers. Profiles can remain applied or run for the
duration of a Claude or Copilot session.

APM documents standalone plugin export through `apm pack`. Use that format as the
first integration point. Do not assume that repoverlay supports all APM exports,
including agents, commands, hooks, or instructions.

Recheck both tools' behavior when implementation starts. The commands, export
layout, and supported capabilities can change.

## Phase 1: Prove the plugin workflow

1. Select a small APM project with a skill and an MCP declaration. Include or add
   fixtures for unsupported components so the compatibility limits are visible.
2. Resolve dependencies and export a standalone Claude-style plugin with APM in
   a separate working directory. Record the APM version and exact commands.
3. Reference the exported bundle from an existing repoverlay profile through
   its local plugin path support. Keep ordinary configuration files in overlays.
4. Exercise persistent profile application and temporary Claude and Copilot
   sessions. Identify differences between harnesses.
5. Document the supported export subset and report unsupported components before
   application. Do not silently discard them.
6. Add only the compatibility fixes needed for this workflow. Do not add a new
   profile schema unless the existing plugin interface cannot express it.

Expected flow:

```text
APM manifest and lockfile
  -> APM install and plugin export outside the target repository
  -> local plugin bundle
  -> repoverlay profile with plugin and ordinary overlays
  -> repo-local agent configuration
```

Completion criteria:

- A developer can apply a supported APM-built plugin with an existing profile.
- The target repository does not need an APM manifest or committed agent files.
- Existing project files and local edits remain protected.
- Removal and temporary-session cleanup affect only profile-owned content.
- Documentation identifies unsupported exports and any harness-specific limits.

## Phase 2: Add native profile support only if needed

Proceed if Phase 1 shows a concrete need for repoverlay to invoke APM, manage build
artifacts, or connect dependency updates to profile updates. Otherwise, keep the
documented plugin workflow as the integration.

Possible configuration, illustrative only:

```ccl
profiles =
  rust-dev =
    overlays =
      = my-overlays/rust
    apm =
      source = ./agent-packages/rust
```

Start with a local APM project reference. Define path resolution relative to the
profile's configuration file. Defer remote source syntax unless a required use
case cannot use existing source mechanisms.

Implementation tasks:

1. Define the supported APM versions, manifest and lockfile requirements, and
   export targets. Fail with an actionable error when APM is missing or incompatible.
2. Run APM in an isolated working directory, with the policy context and trust
   decisions that apply to the source and target repositories. Block the operation
   if the adapter cannot preserve those controls.
3. Materialize supported output as a repoverlay-managed artifact. Reuse the plugin
   adapter where possible. Keep symlink targets alive for the full applied lifetime;
   do not link an applied profile to a temporary build directory.
4. Apply output through existing file, exclusion, conflict, and state mechanisms.
   Do not copy an entire scratch directory into the repository.
5. Track enough provenance to identify the source, lockfile, APM version, target,
   and applied artifact. Reuse existing cache and state mechanisms where they fit.
6. Define lifecycle behavior for profile apply, status, update, remove, restore,
   and temporary harness sessions. Keep restore distinct from dependency update;
   do not resolve newer dependencies as a fallback during recovery.
7. Define composition rules for multiple profiles and existing repository APM
   configuration. Reject ambiguous ownership before writing files.
8. Surface resolution, policy, compilation, deployment, and cleanup failures.
   Preserve the previous usable application if a replacement fails.

## Validation for implementation

Use the repository's existing Rust test infrastructure for adapter and lifecycle
coverage. Use a controlled APM fixture or executable stub for deterministic tests;
record a separate compatibility run against the supported APM release.

Cover these cases:

- Skills and MCP configuration for each supported harness.
- Existing tracked files, shared configuration files, and user edits.
- Unsupported plugin components and conflicting profile output.
- Persistent apply, repeated apply, update, removal, and recovery after `git clean`.
- Temporary-session cleanup after normal exit, agent failure, and interruption.
- Missing APM, incompatible versions, failed resolution, and partial build output.
- Lockfile reproducibility and source or exported-path traversal attempts.
- Trust prompts, policy denial, and attempts to escape the isolated output area.
- Existing project APM configuration without overwrite or ownership takeover.

## Optional distribution path: prebuilt overlays

A publisher can resolve and build APM output in CI, then distribute the supported
files as an overlay. Consumers would only need repoverlay.

Evaluate this after Phase 1 if consumers need to avoid a local APM dependency.
Record source and lockfile provenance with the artifact and publish separate
harness outputs where required. Consumers receive prebuilt output rather than
local dependency resolution.

## Decisions to resolve before Phase 2

- Which export format preserves the required capabilities for Claude and Copilot?
- Does APM expose sufficient output ownership data, or must the adapter restrict
  imports to a known plugin layout?
- How will an isolated build preserve target-repository policy and trust context?
- Where will durable artifacts live, and how will active profiles protect them
  from cache cleanup?
- Which operation may change the APM lockfile, and how will it report that change?
- Can native APM output coexist with existing repoverlay-managed instruction and
  MCP regions without overlapping ownership?

## References

- [APM repository and overview](https://github.com/microsoft/apm)
- [APM pack reference](https://microsoft.github.io/apm/reference/cli/pack/)
- [APM governance guide](https://microsoft.github.io/apm/enterprise/governance-guide/)
- [repoverlay profiles](../../README.md#profiles-experimental)

This plan records the integration discussion. It does not make the illustrative
configuration or proposed lifecycle behavior part of the public interface.
