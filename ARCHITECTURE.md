# Architecture

repoverlay is a CLI tool that overlays config files into git repositories without committing them. It supports local overlays, GitHub repository overlays, and shared overlay repositories.

## Module Structure

```
src/
├── main.rs              # CLI entry point (minimal - delegates to lib)
├── cli/
│   ├── mod.rs           # CLI command definitions, argument parsing, and dispatch (clap)
│   └── commands/        # Handlers: browse, cache, claude, copilot, create, edit,
│                        #   library, marketplace, move, profile, source, sync
├── lib.rs               # Core library: apply_overlay, composition resolution, glue
├── resolve.rs           # Source resolution (source string → local path)
├── reference.rs         # Input reference parsing
├── status.rs            # show_status / show_status_json
├── remove.rs            # remove_overlay
├── create.rs            # create_overlay and update_overlays
├── update.rs            # CLI version string and self-update check
├── state.rs             # State persistence (in-repo and external backup)
├── github.rs            # GitHub URL parsing
├── cache.rs             # GitHub repository cache management
├── config.rs            # Global and per-repo configuration (CCL format)
├── sources.rs           # Multi-source overlay resolution with priority ordering
├── overlay_repo.rs      # Shared overlay repository integration
├── upstream.rs          # Upstream repository detection for fork inheritance
├── library.rs           # In-repo overlay library management
├── git.rs               # Git subprocess helpers and repository inspection
├── git_exclude.rs       # .git/info/exclude section management
├── json_merge.rs        # JSON deep merge for --merge
├── fuzzy.rs             # Fuzzy matching for overlay name suggestions
├── fs_util.rs           # Filesystem utilities (atomic writes, copies)
├── path_safety.rs       # Path safety validation (traversal protection)
├── overlay_name.rs      # Normalized overlay name newtype
├── detection.rs         # File discovery for overlay creation
├── selection.rs         # Interactive file selection UI
├── widgets/             # Reusable ratatui UI components
│   └── multi_select_tree.rs  # Tree widget with tri-state checkboxes
├── profile.rs           # Profile configuration, merge rules, state metadata
├── plugin.rs            # Plugin reference model and marketplace resolution
├── profile_plan.rs      # Profile apply planning and transactional execution
├── profile_applicators/ # Per-harness placement (claude.rs, copilot.rs)
├── harness_process.rs   # Managed harness process execution (ephemeral mode)
├── snapshots/           # insta snapshot files for unit tests
└── testutil.rs          # Test utilities (create_test_repo, create_test_overlay)

tests/
├── cli.rs          # CLI integration tests using assert_cmd
└── common/mod.rs   # Shared test utilities and fixtures
```

### Module Responsibilities

- **main.rs** - Minimal CLI entry point. Initializes logging and delegates to `lib::run()`.

- **cli/** - CLI command definitions using clap derive macros. `mod.rs` defines
  top-level commands, arguments, flags, and dispatch; `commands/` contains
  command-specific handlers.

- **lib.rs** - Core apply machinery: `apply_overlay`, overlay composition resolution
  (`extends`/`includes`), and shared helpers. Status, remove, create, and update
  operations live in their own modules (`status.rs`, `remove.rs`, `create.rs`).

- **resolve.rs / reference.rs** - Turn user input into something applyable:
  `reference.rs` parses the input string (path, URL, `org/repo/name`, bare name),
  `resolve.rs` resolves it to a local overlay directory, consulting the library,
  configured sources, and the GitHub cache.

- **selection.rs** - Interactive file selection UI. Handles checkbox-style multi-select for overlay creation.

- **widgets/** - Reusable ratatui UI components.
  - **multi_select_tree.rs** - `MultiSelectTree` stateful widget: renders a tree with tri-state checkboxes (checked/unchecked/partial) based on descendant selection state.

- **state.rs** - State persistence layer. Manages overlay state in two locations:
  - In-repo: `.repoverlay/overlays/<name>.ccl` - tracks applied overlays
  - External: `~/.local/share/repoverlay/applied/` - backup for recovery after `git clean`

- **github.rs** - GitHub URL parsing. Handles URL formats like `https://github.com/owner/repo/tree/branch/subpath` and extracts owner, repo, ref, and subpath components.

- **cache.rs** - GitHub repository caching. Manages cloned repos in `~/.cache/repoverlay/github/owner/repo/`. Supports shallow clones and update checking.

- **config.rs** - Configuration management using CCL format. Handles global config (`~/.config/repoverlay/config.ccl`), per-repo config (`.repoverlay/config.ccl`), and per-overlay config (`repoverlay.ccl`).

- **sources.rs** - Multi-source overlay resolution. Manages a priority-ordered list of overlay sources (configured via `repoverlay source add/remove/list`). Provides `SourceManager` for resolving overlay references across multiple sources with first-match-wins semantics. Configured git sources are cloned to `~/.cache/repoverlay/sources/<name>/`.

- **overlay_repo.rs** - Shared overlay repository support. Allows overlays to be referenced as `org/repo/name` from a centrally managed repository. Supports fallback resolution for fork inheritance.

- **upstream.rs** - Upstream repository detection. Scans git remotes to identify parent repositories (forks), enabling automatic overlay inheritance from upstream.

- **library.rs** - In-repo overlay library management. Handles the `.repoverlay/library/` directory for storing shareable overlays within a repository. Provides path resolution (configurable via per-repo config), overlay listing, import/export/remove operations, and gitignore detection. Library overlays are auto-discovered and resolved with highest priority.

- **git.rs / git_exclude.rs** - Git subprocess helpers (with Ctrl+C handling) and
  management of named overlay sections in `.git/info/exclude`.

- **json_merge.rs / fuzzy.rs / fs_util.rs / path_safety.rs / overlay_name.rs** -
  Support modules: JSON deep merge for `--merge`, fuzzy name suggestions, atomic
  filesystem operations, path traversal protection, and the normalized overlay
  name newtype.

- **detection.rs** - File discovery for the `create` command. Identifies AI configs, gitignored files, and untracked files that might be candidates for overlay creation.

- **profile.rs / plugin.rs / profile_plan.rs / profile_applicators/ / harness_process.rs** -
  The profile subsystem. `profile.rs` defines `ProfileConfig` (description, overlays,
  instructions, plugins) and profile state; `plugin.rs` models plugin references and
  resolves them from marketplaces or local paths; `profile_plan.rs` plans and executes
  a profile apply transactionally; `profile_applicators/` owns per-harness placement
  (Claude vs Copilot paths); `harness_process.rs` runs a harness process with
  ephemeral profiles applied for its lifetime. See DEV.md "Profiles Feature Status"
  for the capability matrix.

- **testutil.rs** - Test utilities including `create_test_repo()` and `create_test_overlay()` helpers for setting up temporary git repositories in tests.

## Data Flow

### Apply

```
Source string → resolve_source() → local path
    ↓
Load repoverlay.ccl config
    ↓
If extends/includes present:
    Recursively resolve composition → merged file list
Else:
    Walk files in overlay directory → file list
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

### Profile apply

```
Look up profile in config (repo-local .repoverlay/config.ccl over global config)
    ↓
Resolve plugins (PluginRef → marketplace clone or local bundle dir)
    ↓
Build ProfilePlan for the chosen harness (claude | copilot):
    - overlays → regular overlay apply machinery
    - instructions → managed region in CLAUDE.md / AGENTS.md
    - plugin skills/agents → harness-specific directories
    - plugin MCP servers → merged into .mcp.json
    - delegate plugins (Claude only) → .claude/settings[.local].json
    ↓
Execute plan transactionally (rollback on failure)
    ↓
Record state under .repoverlay/profiles/ + external snapshot
```

Ephemeral mode (`repoverlay claude|copilot --profile X`) runs the same plan, holds a
PID lock while the harness process runs, and rolls back all placements when it exits.
See DEV.md "Profiles Feature Status" for the full per-harness capability matrix.

## State File Format

Overlay state is stored in CCL format (a human-readable configuration language). Example:

```
name = my-overlay
applied_at = 2024-01-15T10:30:00Z
source =
  type = Local
  path = /path/to/overlay
files =
  =
    source = .envrc
    target = .envrc
    link_type = symlink
```

The source is an internally tagged enum (`OverlaySource` in `src/state.rs`, `#[serde(tag = "type")]`): a nested block whose `type` field selects the variant, with the variant's fields alongside it. The variants and their fields:

- `Local`: `path`, optional `source_name`
- `Library`: `name` (in-repo `.repoverlay/library/` overlay)
- `GitHub`: `url`, `owner`, `repo`, `git_ref`, `commit`, optional `subpath`, `cached_at`
- `OverlayRepo`: `org`, `repo`, `name`, `commit`, optional `resolved_via`, optional `source_name`

(`repoverlay status --json` exposes the same shape with lowercase `type` values; that JSON schema is the stable contract for scripting.)

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
3. **Library overlay** (bare name) - Checks `.repoverlay/library/` first (highest priority)
4. **Configured source reference** (`org/repo/name`) - Resolves from configured sources in priority order

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

# Apply overlay - falls back to microsoft/FluidFramework if needed
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

## Overlay Composition

Overlays can inherit files from other library overlays via `extends` and `includes` in `repoverlay.ccl`.

### extends

Full inheritance from a single parent overlay. The parent's files, mappings, and directories are inherited. Child files win on conflict.

```
extends =
  overlay = parent-name
```

Multi-level chains are supported (child extends parent extends grandparent). Cycle detection prevents infinite recursion.

### includes

Cherry-pick specific files from other overlays without inheriting everything.

```
includes =
  =
    overlay = tools
    files =
      = .editorconfig
      = scripts/lint.sh
```

Multiple includes are allowed. Included overlays are recursively resolved (they may themselves use extends/includes).

### Precedence

When the same target path appears in multiple sources, the highest-precedence version wins:

1. **Child's own files** (highest)
2. **extends** parent files
3. **includes** files (in listed order, later overrides earlier)

### Scope

Referenced overlays must be library overlays (`.repoverlay/library/`). Other source types (GitHub, local path) are not supported for composition references.

## Caching Strategy

GitHub repositories are cached in `~/.cache/repoverlay/github/owner/repo/`:

- Uses shallow clones to minimize disk usage
- Caches are updated on `repoverlay update` or when `--ref` changes
- Cache metadata tracks commit hash and last update time
- `repoverlay cache` subcommands manage the cache

Configured sources are cloned separately to `~/.cache/repoverlay/sources/<name>/` and refreshed when commands resolve from them. The `cache` subcommands only operate on the `github/` directory.

## Decisions

See [docs/adr/](docs/adr/) for architectural decision records.
