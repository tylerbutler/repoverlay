---
title: Introducing profiles
date: 2026-06-27
excerpt: Profiles combine overlays, agent instructions, and plugins into one named configuration. Apply a profile to any repository.
authors:
  - tylerbutler
tags:
  - release
  - profiles
---

repoverlay 0.17.0 introduces **profiles**, the biggest feature I have added since the
project began. A profile combines overlays, agent instruction files, and plugins
into one named configuration that you apply to a repository.

## Why I built them

Overlays solve one problem well: they add configuration files to a repository without
commits. But my own setup needed more than files. I wanted to say "give me
everything I need to do Rust work with this agent" and have one command place the
overlays, write the agent instructions, and install the skills and MCP servers I
rely on.

An overlay defines files to add. A profile defines a working environment.
It refers to overlays and adds agent capabilities through instructions and plugins.

## How they work

Define marketplaces and profiles in your repoverlay CCL configuration, beside `sources`:

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

Apply the profile to a specific harness, the application that runs the agent.
To keep the profile in place until you remove it, use persistent mode:

```bash
repoverlay profile apply rust-dev --harness copilot
```

To apply it for one agent session with automatic cleanup, use ephemeral mode:

```bash
repoverlay copilot --profile rust-dev
```

Profiles use **plugins** in the Claude Code plugin format. A plugin supplies
skills, agents, and MCP servers. When you apply a profile, repoverlay extracts
managed plugins that it can cache. It puts each part in the path for the selected
harness.

Skills use `.agents/skills/` for Copilot and `.claude/skills/` for Claude.
repoverlay merges MCP servers into `.mcp.json`. Claude loads delegate plugins
and plugins that repoverlay cannot cache through its own settings.
Copilot skips these plugins with a warning.

repoverlay applies profile capabilities to the target repository, not globally
for the user or machine. It excludes new files from git when possible.
Profiles can also update existing files through managed regions or JSON merges.
repoverlay keeps cache and recovery snapshots outside the repository.

## Current limitations

Profiles have these limits in 0.17.0:

- Profiles support GitHub Copilot and Claude Code. They do not support other
  harnesses yet.
- Only Claude supports `delegate` install mode. This mode enables a plugin in the
  harness configuration. Copilot skips delegate plugins with a warning.
- A profile changes only the repository where you apply it. It does not install
  capabilities globally for the user or machine. Apply the profile to each
  repository where you need it.
- **One mode at a time per profile.** A profile already applied persistently
  cannot also run as an ephemeral session. An active ephemeral profile cannot
  also be applied persistently. A lock file prevents concurrent ephemeral
  sessions. repoverlay recovers the lock automatically if a previous session was killed.
- **Interrupted cleanup needs a manual step.** If an ephemeral session is
  interrupted and cleanup fails, repoverlay reports the error. It keeps enough
  state for you to finish with `repoverlay profile remove`.

## Try them

The [profiles guide](/guides/profiles/) explains how to define, inspect, apply, and
remove profiles. It covers marketplaces, managed and delegate plugins, and the
capability paths for each harness. Read the guide, and tell me what you build.
