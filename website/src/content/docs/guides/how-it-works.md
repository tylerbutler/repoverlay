---
title: How it works
sidebar:
  order: 6
---

repoverlay uses symlinks, git exclusions, and external state backups. This page explains how they work.

## Symlinks vs copies

By default, repoverlay creates symlinks from the target repository to the overlay source. Changes to the source appear immediately in the target.

Use `--copy` to copy files instead when:
- Your environment does not support symlinks, such as some Docker setups or Windows without developer mode.
- You want independent copies that do not change with the source.
- Your CI environment does not handle symlinks correctly.

## Git exclusion

When you apply files, repoverlay adds their paths to `.git/info/exclude`. This file contains exclusion rules for one repository. Git does not track the exclusion file. As a result:

- New overlay files do not appear in `git status`.
- repoverlay does not change the tracked `.gitignore` file.
- Each overlay has a named section that repoverlay deletes when you remove the overlay.

The exclude entries look like this:

```
# repoverlay:my-overlay start
.envrc
.claude/
# repoverlay:my-overlay end
```

These rules exclude new overlay files without changes to the tracked `.gitignore` file. They do not hide changes to files that git already tracks.

To prevent accidental commits, `repoverlay apply` fails if it cannot update
`.git/info/exclude`. If it has already created files, repoverlay removes them
where possible. It does not save overlay state.

During `repoverlay remove`, repoverlay removes managed files and state where
possible. If exclude cleanup fails, the command returns a non-zero exit code.
It reports that the files were removed but `.git/info/exclude` still needs repair.

## State tracking

repoverlay tracks applied overlays in two locations:

- In-repo state (`.repoverlay/overlays/<name>.ccl`): the primary record of applied overlays, inside the target repository.
- External backup (`~/.local/share/repoverlay/applied/`): a recovery copy of the state, outside the repository.

The external backup lets repoverlay restore overlays after `git clean` or other operations that remove untracked files. See [Restoring after git clean](/guides/restoring/) for details.

repoverlay writes state files in [CCL format](https://ccl.tylerbutler.com/). Each file records the overlay name, source, application timestamp, and file list with link types.

### CCL compatibility and migrations

repoverlay maintains compatibility for these CCL file formats:

- Overlay source config: `repoverlay.ccl`
- Global and per-repository source config
- In-repo state: `.repoverlay/overlays/<name>.ccl`
- External backup state: `~/.local/share/repoverlay/applied/`

Compatibility follows the CLI semantic versioning policy:

- Patch and minor releases may add optional CCL fields. Older files continue to
  load with documented defaults.
- Patch and minor releases do not remove fields, rename fields, or change the
  meaning of existing fields.
- repoverlay must migrate existing CCL files automatically when possible.
  Release notes must document required migrations.
- Only major releases can remove or rename fields, change their meanings, or
  require a manual migration.

repoverlay uses state files to restore, update, and remove overlays. Their
format stays compatible during normal upgrades so backups remain usable.
For scripts, use `repoverlay status --json` instead of direct access to CCL state files.

## Caching

repoverlay caches GitHub repositories locally so each `apply` does not need a new download. It stores caches at `~/.cache/repoverlay/github/<owner>/<repo>/`.

- repoverlay uses shallow clones to reduce disk use.
- `repoverlay update` updates the caches automatically.
- Cache metadata records the commit hash and last update time.
- A change to `--ref` gets the new ref and adds it to the existing cache.

Manage the cache with:

```bash
repoverlay cache list              # List cached repositories
repoverlay cache path              # Show cache location
repoverlay cache remove owner/repo # Remove a specific cached repo
repoverlay cache remove --all      # Remove all cached repos
```

repoverlay clones configured sources separately, to `~/.cache/repoverlay/sources/<name>/`. These are the sources you add with `repoverlay source add`.

The `cache` subcommands operate only on the `github/` directory. Commands such as `browse` and `update` refresh source clones automatically when they resolve overlays from them. You can delete source clones manually. repoverlay clones them again on the next use.

## Fork inheritance

When you work on a fork, repoverlay can use overlays from the upstream (parent) repository automatically.

### Resolution order

When you apply an overlay using a configured source reference (`org/repo/name`), repoverlay checks:

1. A direct match for the `org/repo` of your fork.
2. A match for the upstream `org/repo`, if no direct match exists and you have an `upstream` remote.

### Example

```bash
# Your fork's remotes
git remote -v
# origin    git@github.com:tylerbutler/FluidFramework.git
# upstream  git@github.com:microsoft/FluidFramework.git

# This checks for tylerbutler/FluidFramework/claude-config first,
# then falls back to microsoft/FluidFramework/claude-config
repoverlay apply microsoft/FluidFramework/claude-config
```

### Status display

If repoverlay uses an upstream overlay, `repoverlay status` shows the source:

```
Overlay: claude-config
  Source:  microsoft/FluidFramework/claude-config (via upstream) (overlay repo)
  Commit:  abc123def456
```

### Upstream detection

repoverlay looks for a git remote named `upstream`, the standard name for the parent repository of a fork. It supports HTTPS and SSH remote URLs.
