# Architecture

repoverlay is a CLI tool that overlays config files into git repositories without committing them. It supports local overlays, GitHub repository overlays, and shared overlay repositories.

## Module Structure

```
src/
├── main.rs         # CLI entry point (minimal - delegates to lib)
├── cli.rs          # CLI command definitions and argument parsing (clap)
├── lib.rs          # Core library with apply/remove/status/restore/update operations
├── state.rs        # State persistence (in-repo and external backup)
├── github.rs       # GitHub URL parsing and source resolution
├── cache.rs        # GitHub repository cache management
├── config.rs       # Global and per-repo configuration (CCL format)
├── sources.rs      # Multi-source overlay resolution with priority ordering
├── overlay_repo.rs # Shared overlay repository integration
├── upstream.rs     # Upstream repository detection for fork inheritance
├── detection.rs    # File discovery for overlay creation
├── selection.rs    # Interactive file selection UI
└── testutil.rs     # Test utilities (create_test_repo, create_test_overlay)

tests/
├── cli.rs          # CLI integration tests using assert_cmd
└── common/mod.rs   # Shared test utilities and fixtures
```

### Module Responsibilities

- **main.rs** - Minimal CLI entry point. Initializes logging and delegates to `lib::run()`.

- **cli.rs** - CLI command definitions using clap derive macros. Defines all subcommands, arguments, and flags.

- **lib.rs** - Core operations: `apply_overlay`, `remove_overlay`, `show_status`, `restore_overlays`, `update_overlays`, `create_overlay`, `switch_overlay`. Also handles git exclude file management.

- **selection.rs** - Interactive file selection UI. Handles checkbox-style multi-select for overlay creation.

- **state.rs** - State persistence layer. Manages overlay state in two locations:
  - In-repo: `.repoverlay/overlays/<name>.ccl` - tracks applied overlays
  - External: `~/.local/share/repoverlay/applied/` - backup for recovery after `git clean`

- **github.rs** - GitHub URL parsing. Handles URL formats like `https://github.com/owner/repo/tree/branch/subpath` and extracts owner, repo, ref, and subpath components.

- **cache.rs** - GitHub repository caching. Manages cloned repos in `~/.cache/repoverlay/github/owner/repo/`. Supports shallow clones and update checking.

- **config.rs** - Configuration management using CCL format. Handles global config (`~/.config/repoverlay/config.ccl`) and per-overlay config (`repoverlay.ccl`).

- **sources.rs** - Multi-source overlay resolution. Manages a priority-ordered list of overlay sources (configured via `repoverlay source add/remove/move`). Provides `SourceManager` for resolving overlay references across multiple sources with first-match-wins semantics.

- **overlay_repo.rs** - Shared overlay repository support. Allows overlays to be referenced as `org/repo/name` from a centrally managed repository. Supports fallback resolution for fork inheritance.

- **upstream.rs** - Upstream repository detection. Scans git remotes to identify parent repositories (forks), enabling automatic overlay inheritance from upstream.

- **detection.rs** - File discovery for the `create` command. Identifies AI configs, gitignored files, and untracked files that might be candidates for overlay creation.

- **testutil.rs** - Test utilities including `create_test_repo()` and `create_test_overlay()` helpers for setting up temporary git repositories in tests.

## Data Flow

### Apply

```
Source string → resolve_source() → local path
    ↓
Walk files in overlay directory
    ↓
For each file:
    - Check for conflicts with existing overlays
    - Check for conflicts with existing files
    - Create symlink or copy
    ↓
Update .git/info/exclude with overlay section
    ↓
Save state to .repoverlay/overlays/<name>.ccl
    ↓
Save external backup to ~/.local/share/repoverlay/applied/
```

### Remove

```
Load state from .repoverlay/overlays/<name>.ccl
    ↓
For each file entry:
    - Remove file/symlink
    - Clean empty parent directories
    ↓
Remove overlay section from .git/info/exclude
    ↓
Delete state file
    ↓
Remove external backup
```

### Restore

```
Load external state backup from ~/.local/share/repoverlay/applied/
    ↓
For each saved overlay:
    - Re-apply using original source (path or GitHub URL)
```

### Update

```
For each applied GitHub overlay:
    - Check remote for new commits
    - If updates available:
        - Remove old overlay
        - Re-apply with updated cache
```

### Create

```
Discover files in repository (AI configs, gitignored, untracked)
    ↓
Interactive selection or --include flags
    ↓
Copy selected files to output directory
    ↓
Generate repoverlay.ccl config
```

### Switch

```
Remove all existing overlays
    ↓
Apply new overlay (atomic replacement)
```

## State File Format

Overlay state is stored in CCL format (a human-readable configuration language). Example:

```
name = my-overlay
applied_at = 2024-01-15T10:30:00Z
source = local|/path/to/overlay
files =
  source = .envrc
  target = .envrc
  link_type = symlink
```

Source types are encoded as pipe-delimited strings:
- Local: `local|/path/to/overlay`
- GitHub: `github|url|owner|repo|ref|commit|subpath|cached_at`
- Overlay repo: `overlay_repo|org|repo|name|commit`

## Git Integration

Overlay files are excluded from git tracking via `.git/info/exclude` using named sections:

```
# repoverlay:my-overlay start
.envrc
.claude/
# repoverlay:my-overlay end
```

This approach:
- Keeps overlay files out of version control
- Doesn't modify `.gitignore` (which is tracked)
- Allows multiple overlays with distinct sections
- Enables clean removal of individual overlays

## Source Resolution

The `resolve_source()` function determines the overlay source type:

1. **GitHub URL** (`https://github.com/...`) - Downloads to cache, returns cached path
2. **Local path** (`./path` or `/path`) - Returns path directly after validation
3. **Overlay repo reference** (`org/repo/name`) - Resolves from configured shared repository

## Fork Inheritance

When applying overlays from a shared repository to a forked repo, repoverlay automatically inherits overlays from the upstream (parent) repository.

### Resolution Order

1. **Direct match** - Look for `fork-org/fork-repo/overlay-name`
2. **Upstream fallback** - If not found and upstream exists, look for `upstream-org/upstream-repo/overlay-name`

### Upstream Detection

The upstream repository is detected by scanning git remotes:

1. Check for a remote named `upstream` (standard fork convention)
2. Parse the remote URL (supports both HTTPS and SSH formats)
3. Extract org/repo for fallback resolution

Example:
```bash
# Fork setup
git remote -v
# origin    git@github.com:tylerbutler/FluidFramework.git (fetch)
# upstream  git@github.com:microsoft/FluidFramework.git (fetch)

# Apply overlay - will fallback to microsoft/FluidFramework if needed
repoverlay apply microsoft/FluidFramework/claude-config
```

### State Tracking

The `ResolvedVia` enum tracks how an overlay was resolved:
- `Direct` - Exact match in overlay repository
- `Upstream` - Resolved via upstream fallback

This is stored in the overlay state and displayed in `repoverlay status`:
```
Overlay: claude-config
  Source:  microsoft/FluidFramework/claude-config (via upstream) (overlay repo)
  Commit:  abc123def456
```

## Caching Strategy

GitHub repositories are cached in `~/.cache/repoverlay/github/owner/repo/`:

- Uses shallow clones to minimize disk usage
- Caches are updated on `repoverlay update` or when `--ref` changes
- Cache metadata tracks commit hash and last update time
- `repoverlay cache` subcommands manage the cache

## Decisions

See [docs/adr/](docs/adr/) for architectural decision records.
