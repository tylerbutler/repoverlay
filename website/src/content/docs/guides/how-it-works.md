---
title: How It Works
sidebar:
  order: 6
---

Symlinks, git exclusions, and external backups do the work. This page covers each mechanism.

## Symlinks vs copies

By default, repoverlay creates **symlinks** from the target repo to the overlay source, so changes to the source appear immediately in the target.

Use the `--copy` flag to copy files instead, which is useful when:
- Symlinks aren't supported (e.g., some Docker setups or Windows without developer mode)
- You want independent copies that won't change when the source is modified
- Your CI environment doesn't handle symlinks well

## Git exclusion

When files are applied, repoverlay adds them to `.git/info/exclude` — a per-repo gitignore file that isn't tracked by git itself. This means:

- Overlay files don't show up in `git status`
- No changes to the tracked `.gitignore` file
- Each overlay gets its own named section for clean removal

The exclude entries look like this:

```
# repoverlay:my-overlay start
.envrc
.claude/
# repoverlay:my-overlay end
```

This approach keeps overlay files completely invisible to git without modifying any tracked files.

Because this exclusion is core to preventing accidental commits, `repoverlay apply`
fails if `.git/info/exclude` cannot be updated. If files were already created,
repoverlay rolls them back where practical and does not save overlay state.

During `repoverlay remove`, managed files and state are still removed where
practical. If exclude cleanup fails, the command exits non-zero and reports that
the files were removed but `.git/info/exclude` still needs attention.

## State tracking

repoverlay tracks applied overlays in two locations:

- **In-repo state** (`.repoverlay/overlays/<name>.ccl`) — the primary record of what's applied, stored inside the target repository
- **External backup** (`~/.local/share/repoverlay/applied/`) — a recovery copy stored outside the repository

The external backup exists so that overlays can be restored after `git clean` or other operations that remove untracked files. See [Restoring After Git Clean](/guides/restoring/) for details.

State files are written in [CCL format](https://ccl.tylerbutler.com/) and track the overlay name, source, applied timestamp, and list of files with their link types.

### CCL compatibility and migrations

repoverlay treats these CCL files as user-facing data contracts:

- Overlay source config: `repoverlay.ccl`
- Global and per-repository source config
- In-repo state: `.repoverlay/overlays/<name>.ccl`
- External backup state: `~/.local/share/repoverlay/applied/`

Compatibility follows the CLI's semver policy:

- Patch and minor releases may add optional CCL fields. Older files continue to
  load with documented defaults.
- Patch and minor releases do not remove fields, rename fields, or change the
  meaning of existing fields.
- Any required migration for existing CCL files must be automatic when possible
  and documented in the release notes.
- Removing or renaming fields, changing meanings, or requiring a manual migration
  is reserved for semver-major releases.

State files are implementation records used for restore/update/remove, but their
format is still stable enough for backups to survive normal upgrades. Prefer
`repoverlay status --json` for scripting instead of reading state CCL directly.

## Caching

GitHub repositories are cached locally to avoid re-downloading on every `apply`. Caches are stored at `~/.cache/repoverlay/github/<owner>/<repo>/`.

- Repos are **shallow cloned** to minimize disk usage
- Caches are updated automatically during `repoverlay update`
- Cache metadata tracks the commit hash and last update time
- Changing `--ref` fetches the new ref into the existing cache

Manage the cache with:

```bash
repoverlay cache list              # List cached repositories
repoverlay cache path              # Show cache location
repoverlay cache remove owner/repo # Remove a specific cached repo
repoverlay cache remove --all      # Remove all cached repos
```

Configured sources (added with `repoverlay source add`) are cloned separately, to `~/.cache/repoverlay/sources/<name>/`. The `cache` subcommands operate only on the `github/` directory; source clones are refreshed automatically by commands that resolve from them (such as `browse` and `update`) and can be deleted manually — they are re-cloned on next use.

## Fork inheritance

When you work on a **fork** of a repository, repoverlay can automatically inherit overlays from the **upstream** (parent) repository.

### Resolution order

When you apply an overlay using a configured source reference (`org/repo/name`), repoverlay checks:

1. **Direct match** — an overlay matching your fork's `org/repo`
2. **Upstream fallback** — if no direct match exists and an `upstream` remote is configured, an overlay matching the upstream's `org/repo`

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

When an overlay is resolved via upstream fallback, `repoverlay status` shows how it was resolved:

```
Overlay: claude-config
  Source:  microsoft/FluidFramework/claude-config (via upstream) (overlay repo)
  Commit:  abc123def456
```

### Upstream detection

repoverlay detects the upstream repository by scanning git remotes for one named `upstream` — the standard convention for forks. Both HTTPS and SSH remote URLs are supported.
