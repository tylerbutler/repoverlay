---
title: Migrating to 1.0
sidebar:
  order: 5
---

repoverlay 1.0 removes the hidden deprecated command forms that existed during the
pre-1.0 transition. The current commands below are the only supported syntax going
forward.

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

`create-local` has been replaced by local output mode on `create`:

```bash
repoverlay create --output ./my-overlay
```

`create --output` writes overlay files to the output directory and auto-applies the
overlay unless you pass `--dry-run`. See [Creating & Sharing Overlays](/guides/creating/)
for the full workflow.

### Editing applied overlays

The `edit` subcommands are explicit:

```bash
repoverlay edit add my-overlay .envrc
repoverlay edit remove my-overlay old-file.txt
repoverlay edit my-overlay
```

Run `repoverlay edit <name>` with no `add` or `remove` subcommand to re-open the
interactive selector.

### Browsing overlays

Use `browse` for interactive overlay selection:

```bash
repoverlay browse
repoverlay browse owner/repo
repoverlay browse ./path/to/overlays
```

## Source URL syntax changes

repoverlay 1.0 rejects unsupported source URL schemes. These forms are no longer
accepted as overlay sources:

- `file://...`
- `ftp://...`
- `http://...`
- arbitrary `scheme://...` URLs

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

Before 1.0, repoverlay kept some deprecated CLI surfaces hidden for compatibility.
Those hidden compatibility aliases are removed in 1.0 instead of being retained as
long-term aliases.

Retained compatibility is limited to the documented current command surface and
supported source syntaxes listed above. Scripts should be updated to those forms
before upgrading to 1.0.
