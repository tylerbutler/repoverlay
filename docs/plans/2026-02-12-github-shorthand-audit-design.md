# Consistent GitHub Shorthand URL Support (#82)

## Problem

`validate_source_url()` in `config.rs` accepts full git URLs and `owner/repo` shorthand, but rejects bare owner names like `tylerbutler`. These should expand to `owner/repo-overlays` following the existing convention used in `resolve_source()`.

## Approach

Extend `validate_source_url()` in `config.rs` as the single source of truth for URL validation and expansion. Eager expansion: always store full URLs.

### Three-tier validation in `validate_source_url()`

1. Full git URL (`https://...`, `git@...`) -> pass through
2. `owner/repo` shorthand -> expand to `https://github.com/owner/repo`
3. `owner` bare name -> expand to `https://github.com/owner/repo-overlays`

### Shared constant

Move `DEFAULT_OVERLAY_REPO_NAME` ("repo-overlays") from `lib.rs` to `config.rs` so `validate_source_url` can use it without circular dependency. Re-export or reference from `lib.rs`.

## What stays the same

- `SourceReference::parse()` in `reference.rs` is unaffected (handles `apply`/`switch` commands)
- `deserialize_source_url` serde hook already calls `validate_source_url` - gets fix for free
- `source add` handler in `cli.rs` already calls `validate_source_url` - gets fix for free

## Changes

| File | Change |
|------|--------|
| `src/config.rs` | Add `is_bare_owner()`, extend `validate_source_url()`, add `DEFAULT_OVERLAY_REPO_NAME` constant |
| `src/lib.rs` | Use constant from `config.rs` instead of local definition |
| Tests | Add tests for bare owner expansion in `config.rs` |
