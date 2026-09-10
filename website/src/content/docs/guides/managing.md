---
title: Managing applied overlays
sidebar:
  order: 3
---

After you apply overlays, you can check their status, edit them, update them from their source, and remove them.

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

The `status --json` output has a versioned public format. The top-level
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

Patch releases can add fields without a change to `schema_version`.
A release must use a new `schema_version` and a new major version if it:

- Removes or renames fields.
- Changes the meaning of a field.
- Changes an enum string value.

## Editing an overlay

The `edit` command lets you add or remove files from an applied overlay.

### Add files

```bash
repoverlay edit add my-overlay newfile.txt
repoverlay edit add my-overlay file1.txt file2.txt
```

This command copies the files to the overlay source. It replaces the original files with symlinks and updates the overlay state.

### Remove files

```bash
repoverlay edit remove my-overlay oldfile.txt
```

### Interactive re-selection

Open the file selection menu again. The current files are already selected:

```bash
repoverlay edit my-overlay
```

### Preview changes

```bash
repoverlay edit add my-overlay new.txt --dry-run
```

## Syncing changes back

If you change overlay files in your repository, use `sync` to copy the changes back to the overlay source:

```bash
repoverlay sync my-overlay
```

Preview what would be synced:

```bash
repoverlay sync my-overlay --dry-run
```

Use this command to share a configuration change with other repositories that use the overlay.

## Updating remote overlays

For overlays from GitHub, repoverlay can get the latest changes and apply them again:

```bash
# Update all GitHub-sourced overlays
repoverlay update

# Update a specific overlay
repoverlay update my-overlay

# Preview changes
repoverlay update --dry-run
```

:::note
Local overlays that use symlinks do not need updates. The symlinks point to the source files, so changes appear immediately.
:::

### When to update

- After an update to the overlay source on GitHub.
- When you want configuration changes from your team.
- At regular intervals, to get upstream overlay changes.

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

When you remove an overlay, repoverlay deletes its symlinks or copies, git exclude entries, and state files. If exclude cleanup fails, repoverlay still removes managed files and state where possible. The command returns a non-zero exit code so you can repair `.git/info/exclude`.

## Switching overlays

The `switch` command atomically replaces all existing overlays with a new one:

```bash
repoverlay switch ~/overlays/typescript-ai
repoverlay switch https://github.com/user/ai-configs/tree/main/rust
```

:::caution
The `switch` command removes **all** existing overlays before it applies the new one. To keep existing overlays and add another, use `repoverlay apply` instead.
:::

This is the same as `repoverlay remove --all` followed by `repoverlay apply`, but in one atomic operation.

### When to use switch

- To change between language-specific overlay sets, such as Rust and TypeScript configuration.
- To change between personal and team overlay configuration.
- To return to a known overlay state.
