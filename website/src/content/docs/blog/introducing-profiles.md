---
title: Introducing profiles
date: 2026-06-11
excerpt: Profiles compose overlays with AI harness capabilities (instructions and plugins) into one loadable unit you can apply to any repo.
authors:
  - tylerbutler
tags:
  - release
  - profiles
---

repoverlay 0.17.0 ships **profiles**, the biggest feature I've added since the
project began. A profile bundles overlays with AI harness capabilities
(instruction files and plugins) into a single named unit you apply to a repo.

## Why I built them

Overlays solve one problem well: they drop config files into a repo without
committing them. But my own setup outgrew plain files. I wanted to say "give me
everything I need to do Rust work with this agent" and have one command place the
overlays, write the agent instructions, and install the skills and MCP servers I
rely on.

A profile captures that intent. An overlay describes files to place. A profile
describes a working environment: a recipe that references overlays as
ingredients and layers harness capabilities on top.

## How they work

You define marketplaces and profiles in your repoverlay CCL config, alongside `sources`:

```ccl
marketplaces =
  =
    name = playground
    url = obra/claude-plugins

profiles =
  rust-dev =
    description = Rust development profile
    overlays =
      = rust-base
      = rust-tools
    instructions =
      =
        content =
          Be concise in all responses.
    plugins =
      = playground/rust-dev
```

Then you apply it to a specific agent harness. Apply it persistently:

```bash
repoverlay profile apply rust-dev --harness copilot
```

Or apply it only for the lifetime of an agent session, with automatic cleanup:

```bash
repoverlay copilot --profile rust-dev
```

Profiles carry capabilities through **plugins**, the same Claude-style plugin
format the Claude Code ecosystem uses. A plugin ships skills, agents, and MCP
servers. When you apply a profile, repoverlay decomposes managed/cacheable
plugins and places their parts using each harness's own paths: skills land under
`.agents/skills/` for Copilot and `.claude/skills/` for Claude, and MCP servers
merge into `.mcp.json`. Delegate or non-cacheable plugins are Claude-delegated
and Copilot-skipped with a warning.

Profile capabilities are applied to the target repo, not installed globally for
the user or machine. New repo-local files are git-excluded when possible, but
profiles can also update existing repo files through managed regions or JSON
merges, and repoverlay keeps cache and recovery snapshots outside the repo.

## Current limitations

Profiles are new. These limits stand today:

- **Two harnesses.** Profiles target GitHub Copilot and Claude Code. No other
  agent is supported yet.
- **Delegate install is Claude-only.** The `delegate` install mode, which records
  a plugin in the harness's own enablement config, currently means something only
  for Claude. Copilot skips delegate plugins with a warning.
- **Repo scope only.** A profile touches just the repo you apply it to. There is
  no user- or machine-global profile install, so capabilities you want everywhere
  need a profile applied per repo.
- **One mode at a time per profile.** A profile already applied persistently
  cannot also run as an ephemeral session, and vice versa. A lock file guards
  against concurrent ephemeral sessions and recovers automatically if a previous
  session was killed.
- **Interrupted cleanup needs a manual step.** If an ephemeral session is
  interrupted and cleanup fails, repoverlay reports the error and leaves enough
  state behind for you to finish with `repoverlay profile remove`.

## Try them

The [profiles guide](/guides/profiles/) covers defining, inspecting, applying, and
removing profiles in full, including marketplaces, managed versus delegate
plugins, and how each harness maps capabilities. Give it a read, and tell me what
you build.
