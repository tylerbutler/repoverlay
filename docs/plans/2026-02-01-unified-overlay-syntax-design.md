# Unified Overlay Syntax Design

Design for simplified overlay path resolution and interactive browsing.

**Date:** 2026-02-01
**Status:** Draft

---

## Overview

This design introduces a unified syntax for referencing overlays across all commands, with GitHub shorthand and interactive browsing support.

---

## Unified Overlay Path Syntax

Anywhere repoverlay accepts an overlay reference, these forms are valid:

| Form | Interpretation |
|------|----------------|
| `username` | GitHub `username/repo-overlays`, browse mode |
| `owner/repo` | GitHub `owner/repo`, browse mode |
| `owner/repo/overlay` | GitHub `owner/repo`, direct apply |
| `https://github.com/...` | Full URL, direct apply |
| `./path` or `/path` | Local filesystem |
| `overlay-name` | Lookup in applied overlays (for commands operating on applied overlays) |

**Convention:** `repo-overlays` is the assumed repository name for single-username form.

---

## Apply Command Behavior

### Part Count Determines Mode

- **1 part** (`username`): Browse `username/repo-overlays`
- **2 parts** (`owner/repo`): Browse that repo
- **3 parts** (`owner/repo/overlay`): Direct apply

### Interactive vs Non-Interactive

| Input | TTY | Behavior |
|-------|-----|----------|
| `owner/repo` | Yes | Interactive picker |
| `owner/repo` | No | Error: "specify overlay name" |
| `owner/repo/overlay` | Any | Direct apply |
| `owner/repo/overlay` (not found) | Any | Error + fuzzy suggestions |

### Non-Interactive Detection

Apply behaves non-interactively when:
- stdin is not a TTY
- `--non-interactive` flag is set
- CI environment detected (`CI=true`, `GITHUB_ACTIONS`, etc.)

### Strict Three-Part Resolution

If input has three parts (`owner/repo/overlay`) but the overlay is not found:
- **Never** fall back to browse mode
- Show error with fuzzy match suggestions
- Example:
  ```
  Error: Overlay 'claud-config' not found in tylerbutler/repo-overlays
  Did you mean: claude-config?
  ```

---

## Commands Operating on Applied Overlays

Commands like `sync`, `remove`, `status --name` accept overlay references that resolve against applied overlays.

### Resolution Order

1. If input is a full path (`owner/repo/overlay`), match exactly
2. If input is a name (`overlay-name`), search applied overlays

### Ambiguity Handling

When multiple applied overlays share the same name:

- **Unambiguous**: Use it directly
- **Ambiguous + TTY**: Interactive picker
- **Ambiguous + non-TTY**: Error with full paths listed

Example error:
```
Error: Multiple overlays named 'claude-config':
  - tylerbutler/repo-overlays/claude-config
  - myorg/team-configs/claude-config
Use full path to specify.
```

---

## Auto-Create Overlay Repository

When `repoverlay create` is run without a configured overlay repository:

```
repoverlay create claude-config
# No overlay repository configured. Create one?
# > Yes, create yourname/repo-overlays on GitHub
#   No, I'll use a local directory
#   No, let me configure it manually
```

Selecting "Yes":
1. Runs `gh repo create yourname/repo-overlays --private`
2. Clones to local overlay repo path
3. Configures as default overlay repository
4. Continues with overlay creation

---

## Implementation Notes

### GitHub API Usage

- Use `gh` CLI for repo operations (already a dependency pattern)
- Cache overlay listings to avoid repeated API calls
- Respect rate limits

### Fuzzy Matching

- Use Levenshtein distance or similar for suggestions
- Threshold: suggest if distance <= 3 or 30% of string length
- Show top 3 matches maximum

### TTY Detection

```rust
use std::io::IsTerminal;

fn is_interactive() -> bool {
    std::io::stdin().is_terminal()
        && std::env::var("CI").is_err()
        && std::env::var("GITHUB_ACTIONS").is_err()
}
```

---

## Examples

```bash
# Browse tylerbutler's default overlay repo
repoverlay apply tylerbutler

# Browse a specific repo
repoverlay apply myorg/shared-configs

# Direct apply
repoverlay apply tylerbutler/repo-overlays/claude-config

# Typo - error with suggestion
repoverlay apply tylerbutler/repo-overlays/claud-config
# Error: Overlay 'claud-config' not found
# Did you mean: claude-config?

# In CI - error instead of prompt
CI=true repoverlay apply tylerbutler
# Error: No overlay specified. Available: claude-config, dev-setup
# Use: repoverlay apply tylerbutler/repo-overlays/<name>

# Sync with ambiguous name
repoverlay sync claude-config
# ? Multiple overlays named 'claude-config':
#   > tylerbutler/repo-overlays/claude-config
#     myorg/team-configs/claude-config

# Sync with explicit path
repoverlay sync tylerbutler/repo-overlays/claude-config
```
