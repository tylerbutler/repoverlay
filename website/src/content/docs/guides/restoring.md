---
title: Restoring after git clean
sidebar:
  order: 4
---

If `git clean` or another operation deletes your overlay files, repoverlay can restore them from their sources. It uses the state backup outside the repository to identify those files. Keep the overlay sources available.

## Restoring overlays

```bash
repoverlay restore
```

This command applies all previously applied overlays again. It uses the information in `~/.local/share/repoverlay/applied/`.

## Preview before restoring

```bash
repoverlay restore --dry-run
```

## How backups work

:::tip
repoverlay stores state backups outside the git repository. These backups survive `git clean`, branch switches, and other operations that remove untracked files. repoverlay creates them automatically.
:::

Each time you apply an overlay, repoverlay saves a copy of its state outside the git repository.

The external backup stores:
- The overlay name and source
- The list of files and their link types
- Enough information to re-apply the overlay from the original source

For details about state tracking and external backups, see [How it works](/guides/how-it-works/#state-tracking).

## When to use restore

- After you run `git clean -fd` or `git clean -fdx`
- After a branch checkout removes the `.repoverlay/` directory
- After any operation that deletes untracked files from the repository
