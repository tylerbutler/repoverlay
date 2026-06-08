---
title: Managing Applied Overlays
sidebar:
  order: 3
---

Once overlays are applied, you can check their status, edit them, update them from their source, and remove them.

## Checking status

See what overlays are currently applied:

```bash
repoverlay status
```

Check a specific overlay:

```bash
repoverlay status --name my-overlay
```

Status shows each overlay's name, source, and the files it manages.

### Status JSON schema

Use JSON output for scripts and CI:

```bash
repoverlay status --json
repoverlay status --json --name my-overlay
```

The `status --json` output is a versioned public contract. The top-level
`schema_version` field is currently `1`.

```json
{
  "schema_version": 1,
  "overlays": [
    {
      "name": "my-overlay",
      "applied_at": "2026-03-15T12:34:56Z",
      "source": {
        "type": "local",
        "path": "/Users/me/overlays/my-overlay"
      },
      "files": [
        {
          "source": ".envrc",
          "target": ".envrc",
          "link_type": "symlink",
          "entry_type": "file",
          "status": "ok"
        }
      ]
    }
  ]
}
```

`source.type` is one of `local`, `github`, `library`, or `overlay_repo`.
Source objects include the stable fields relevant to that source type:

- `local`: `path`, optional `source_name`
- `github`: `url`, `owner`, `repo`, `git_ref`, `commit`, optional `subpath`
- `library`: `name`
- `overlay_repo`: `org`, `repo`, `name`, `commit`, optional `resolved_via`,
  optional `source_name`

File entries use string values for `link_type` (`symlink`, `copy`, `merged`),
`entry_type` (`file`, `directory`), and `status` (`ok`, `missing`).

Patch releases may add fields without changing `schema_version`. Removing or
renaming fields, changing field meanings, or changing enum string values requires
a new `schema_version` and a semver-major release.

## Editing an overlay

The `edit` command lets you add or remove files from an applied overlay.

### Add files

```bash
repoverlay edit add my-overlay newfile.txt
repoverlay edit add my-overlay file1.txt file2.txt
```

This copies the files to the overlay source, replaces the originals with symlinks, and updates the overlay state.

### Remove files

```bash
repoverlay edit remove my-overlay oldfile.txt
```

### Interactive re-selection

Re-run the interactive file selector with current files pre-selected:

```bash
repoverlay edit my-overlay
```

### Preview changes

```bash
repoverlay edit add my-overlay new.txt --dry-run
```

## Syncing changes back

If you've modified overlay files in your repo (e.g., edited a symlinked config), the `sync` command copies those changes back to the overlay source:

```bash
repoverlay sync my-overlay
```

Preview what would be synced:

```bash
repoverlay sync my-overlay --dry-run
```

This is useful when you've tweaked a config in one repo and want to propagate the change to all repos using that overlay.

## Updating remote overlays

When overlays come from GitHub, repoverlay can pull the latest changes and re-apply them:

```bash
# Update all GitHub-sourced overlays
repoverlay update

# Update a specific overlay
repoverlay update my-overlay

# Preview changes
repoverlay update --dry-run
```

:::note
Local overlays don't need updating — symlinks already point to the source files, so changes are reflected immediately.
:::

### When to update

- After the overlay source has been updated on GitHub
- When you want to pick up config changes shared by your team
- Periodically, to stay in sync with upstream overlay changes

## Removing overlays

```bash
# Remove a specific overlay
repoverlay remove my-overlay

# Remove all applied overlays
repoverlay remove --all

# Interactive selection
repoverlay remove --interactive

# Preview what would be removed
repoverlay remove my-overlay --dry-run
```

Removing an overlay deletes its symlinks (or copies), cleans up the git exclude entries, and removes the state files. If exclude cleanup fails, managed files and state are still removed where practical, but the command exits non-zero so you can repair `.git/info/exclude`.

## Switching overlays

The `switch` command atomically replaces all existing overlays with a new one:

```bash
repoverlay switch ~/overlays/typescript-ai
repoverlay switch https://github.com/user/ai-configs/tree/main/rust
```

:::caution
Switch removes **all** existing overlays before applying the new one. If you want to keep some overlays and add another, use `repoverlay apply` instead.
:::

This is equivalent to running `repoverlay remove --all` followed by `repoverlay apply`, but as a single atomic operation.

### When to use switch

- Changing between language-specific overlay sets (e.g., Rust vs TypeScript configs)
- Swapping between personal and team overlay configurations
- Resetting to a known overlay state
