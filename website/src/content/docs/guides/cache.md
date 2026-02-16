---
title: Managing the Cache
sidebar:
  order: 7
---

<!-- TODO: Explain cache location per-platform and size management -->

GitHub repositories are cached locally to avoid re-downloading on every apply. repoverlay provides commands to manage this cache.

## Viewing cached repositories

```bash
repoverlay cache list
```

## Cache location

To see where the cache is stored:

```bash
repoverlay cache path
```

The default location is `~/.cache/repoverlay/github/owner/repo/`.

## Clearing the cache

Remove all cached repositories:

```bash
repoverlay cache clear
```

Remove a specific cached repository:

```bash
repoverlay cache remove owner/repo
```

## How caching works

- GitHub repos are **shallow cloned** to minimize disk usage
- Caches are updated automatically during `repoverlay update`
- Cache metadata tracks the commit hash and last update time
- Changing `--ref` fetches the new ref into the existing cache
