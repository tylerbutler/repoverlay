---
title: What is repoverlay?
---

repoverlay is a command-line tool that adds configuration files to git repositories without commits. It creates symlinks or copies from overlay sources. It adds new paths to `.git/info/exclude` so git does not track them.

## OK, but why?

You may need configuration files that you should not commit to a repository:

- AI assistant configuration (`.claude/`, `CLAUDE.md`, `.cursor/`): personal preferences for each developer.
- Editor settings (`.vscode/settings.json`, `.idea/`): settings for different editors.
- Environment files (`.envrc`, `.env.local`): paths and secrets for a specific machine.
- Development tools (`.prettierrc`, `biome.json`): standards you use in multiple repositories.

You can copy these files manually and add them to `.gitignore`. But you must then update each copy in each repository. A `git clean` command can also delete these files.

## How repoverlay helps

An overlay source is a local directory or a GitHub repository. Define a source, then apply an overlay to any repository with one command. repoverlay:

- Creates symlinks or copies in the target repository.
- Excludes new paths from git through `.git/info/exclude`, not `.gitignore`.
- Records applied files in state files so you can remove, restore, or update them.
- Saves state backups outside the repository so you can restore files from their sources after `git clean`.

## Key features

- Apply overlays from local directories, GitHub URLs, or shared overlay repositories.
- Use overlays from an upstream repository when your fork has no matching overlay.
- Get remote changes with `repoverlay update`. Restore files after `git clean` with `repoverlay restore`.
- Put your configuration files in overlays and share them through GitHub.
- Use `repoverlay.ccl` to rename files, create directory symlinks as a unit, and control other overlay settings.
- Use profiles to combine overlays with AI agent instructions, skills, and MCP servers. Apply a profile to a specific harness until removal or for one agent session. See the [profiles guide](/guides/profiles/).
