---
title: Migrating to 1.0
sidebar:
  order: 5
---

repoverlay 1.0 removes the hidden command forms that earlier versions kept for
compatibility. Use the supported commands below.

## CLI syntax changes

| Removed syntax | Use instead |
| --- | --- |
| `repoverlay create-local ...` | `repoverlay create --output <path> ...` |
| `repoverlay list` | `repoverlay browse` |
| `repoverlay edit --add <name> <files...>` | `repoverlay edit add <name> <files...>` |
| `repoverlay edit --remove <name> <files...>` | `repoverlay edit remove <name> <files...>` |
| `repoverlay edit --interactive <name>` | `repoverlay edit <name>` |
| `repoverlay cache clear` | `repoverlay cache remove --all` |

### Creating local overlays

Use `create` with `--output` instead of `create-local`:

```bash
repoverlay create --output ./my-overlay
```

`create --output` writes overlay files to the output directory and applies the
overlay automatically. Use `--dry-run` to preview these actions without changes.
See [Creating and sharing overlays](/guides/creating/) for the full procedure.

### Editing applied overlays

Use `edit add` to add files and `edit remove` to remove them. Use `edit` alone to open the selection menu:

```bash
repoverlay edit add my-overlay .envrc
repoverlay edit remove my-overlay old-file.txt
repoverlay edit my-overlay
```

Run `repoverlay edit <name>` without `add` or `remove` to open the selection
menu again.

### Browsing overlays

Use `browse` for interactive overlay selection:

```bash
repoverlay browse
repoverlay browse owner/repo
repoverlay browse ./path/to/overlays
```

## Source URL syntax changes

repoverlay 1.0 rejects these unsupported source URL schemes:

- `file://...`
- `ftp://...`
- `http://...`
- Other unsupported `scheme://...` URLs

Use one of the supported source forms instead:

| If you used | Use instead |
| --- | --- |
| `file:///home/me/overlays` | `/home/me/overlays` |
| `file://./overlays` | `./overlays` |
| `file://~/overlays` | `~/overlays` |
| `http://github.com/owner/repo` | `https://github.com/owner/repo` |

Supported source forms are:

- Local paths: `./path`, `/absolute/path`, `~/path`
- Git URLs: `https://...`, `ssh://...`, `git@host:owner/repo.git`
- GitHub shorthand and bare owner references, such as `owner/repo` or `owner`

## Compatibility policy for 1.0

Before 1.0, repoverlay kept some hidden command aliases for compatibility.
Version 1.0 removes them.

Version 1.0 supports only the documented commands and source syntax listed above.
Update your scripts to use these forms before you upgrade to 1.0.
