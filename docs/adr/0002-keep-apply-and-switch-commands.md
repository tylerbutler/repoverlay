# ADR 0002: Keep Both `apply` and `switch` Commands

## Status

Accepted (2026-01-31)

## Context

repoverlay has two commands for applying overlays:

1. **`apply`** - Applies an overlay to a repository additively (can stack multiple overlays)
2. **`switch`** - Removes all existing overlays before applying a new one (exclusive mode)

During CLI simplification discussions, we considered whether `switch` should be removed since it can be expressed as `remove --all && apply`. The concern was that maintaining two commands with overlapping functionality increases maintenance burden and may confuse users.

## Decision

**Keep both `apply` and `switch` commands.**

## Rationale

### Different Mental Models

The commands serve distinct user mental models:

- **`apply`** (additive): "I want to add this overlay to what I have"
- **`switch`** (exclusive): "I want to use this overlay instead of what I have"

While `switch` can be expressed as `remove --all && apply`, forcing users to think in two steps adds cognitive overhead for a common use case.

### Convenience Matters

Many users work with mutually exclusive overlay profiles (dev, test, staging, prod). For them, `switch` is the primary operation and having a single command is more ergonomic:

```bash
# Switching profiles throughout the day
repoverlay switch dev-overlay
# ... later ...
repoverlay switch staging-overlay
# vs.
repoverlay remove --all && repoverlay apply staging-overlay
```

### Low Maintenance Burden

`switch` is a thin wrapper that calls:
1. `remove_overlay(target, None, true)` (remove all)
2. `apply_overlay(...)`

The implementation is ~20 lines and shares all core logic with `apply` and `remove`. There's no divergent code paths to maintain.

### No User Confusion

The commands have clear, distinct purposes:
- `apply` = add/stack
- `switch` = replace

Users intuitively understand the difference, similar to `git checkout` vs `git switch` (though in our case, both names remain useful).

## Consequences

### Positive

- Users can express both additive and exclusive operations naturally
- No breaking change for existing users
- Common workflow (profile switching) remains a single command
- CLI is not artificially constrained

### Negative

- Two commands instead of one for overlay application
- Slightly larger `--help` output
- New users must learn when to use each (though this is intuitive)

## Alternatives Considered

### Remove `switch`, use `apply --replace`

- Rejected because: The flag-based approach makes the exclusive case verbose (`apply --replace`) while the common additive case uses the bare command. This inverts the expected usage frequency for many users.

### Remove `switch`, recommend `remove --all && apply`

- Rejected because: Forcing two commands for a common operation is poor UX. Users would likely create shell aliases, indicating a missing feature.

### Add `apply --exclusive` flag

- Rejected because: This is essentially the same as keeping `switch` but with worse discoverability and ergonomics.

## References

- [CLI Simplification Plan](../plans/2026-01-30-cli-simplification-plan.md)
- Related: User feedback indicating profile switching is a primary use case
