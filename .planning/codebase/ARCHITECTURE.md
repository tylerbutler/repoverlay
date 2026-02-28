# Architecture

**Analysis Date:** 2026-02-27

## Pattern Overview

**Overall:** Modular monolithic CLI application with layered separation of concerns

**Key Characteristics:**
- Single Rust binary (`repoverlay`) with internal module organization
- Clear separation between CLI layer, application logic, and support modules
- Multi-source overlay resolution with priority-based selection
- State persistence with dual-location backup (in-repo + external)
- Git integration via direct git commands and .git/info/exclude management
- Support for three overlay source types: local filesystem, GitHub, and shared overlay repositories

## Layers

**CLI Layer:**
- Purpose: Parse command-line arguments, dispatch to application logic
- Location: `src/cli.rs`
- Contains: Clap derive structures (`Cli`, `Commands` subcommands), argument validation
- Depends on: Application logic functions, state types
- Used by: `src/main.rs` entry point

**Application Logic Layer:**
- Purpose: Implement core operations (apply, remove, status, restore, update, create, switch, browse)
- Location: `src/lib.rs` (primary)
- Contains: Functions like `apply_overlay()`, `remove_overlay()`, `show_status()`, `restore_overlays()`, `update_overlays()`, `create_overlay()`, `switch_overlay()`, conflict handling strategies
- Depends on: State, sources, GitHub, config, detection, selection modules
- Used by: CLI layer dispatches to these functions

**Source Resolution Layer:**
- Purpose: Resolve overlay references to actual locations
- Location: `src/reference.rs`, `src/sources.rs`, `src/overlay_repo.rs`
- Contains: `SourceReference` enum (parses user input), `SourceManager` (multi-source priority ordering), `OverlayRepoManager` (shared repository access)
- Depends on: GitHub module, config, upstream detection
- Used by: Application logic

**State Management Layer:**
- Purpose: Persist and restore overlay state
- Location: `src/state.rs`
- Contains: `OverlayState` (applied overlay record), `OverlaySource` enum (GitHub/Local/OverlayRepo), `SourceResolver`, persistence functions
- Depends on: CCL format handling (sickle crate)
- Used by: Application logic for reading/writing `.repoverlay/` and `~/.local/share/repoverlay/`

**GitHub Integration Layer:**
- Purpose: Parse GitHub URLs and manage repository caching
- Location: `src/github.rs`, `src/cache.rs`
- Contains: `GitHubSource` (URL parsing), `GitRef` enum, `CacheManager` (shallow clones), git command execution
- Depends on: Process execution, file system
- Used by: Source resolution, application logic

**Configuration Layer:**
- Purpose: Load and manage global and per-repo configuration
- Location: `src/config.rs`
- Contains: `RepoverlayConfig` (global config), `Source` (overlay source definitions), CCL parsing/serialization
- Depends on: CCL format (sickle crate)
- Used by: Source management, application logic

**Support Modules:**
- `src/detection.rs` - File discovery for overlay creation (AI configs, gitignored, untracked)
- `src/selection.rs` - Interactive terminal UI for file selection (crossterm-based)
- `src/upstream.rs` - Git fork detection and upstream repository identification
- `src/json_merge.rs` - Deep JSON merge utilities for `--merge` flag
- `src/fuzzy.rs` - Fuzzy matching for overlay name selection
- `src/overlay_name.rs` - Overlay name validation and normalization

## Data Flow

### Apply Overlay

```
CLI: apply <source> --target <repo>
  ↓
parse_source() via SourceReference → resolve_source() (reference.rs, sources.rs, overlay_repo.rs)
  ↓
Determine source type (GitHub, Local, OverlayRepo) and resolve to filesystem path
  ↓
apply_overlay() in lib.rs:
  - Walk files in overlay directory
  - Check for conflicts (existing overlays, existing files, cross-overlay conflicts)
  - Apply conflict strategy (Fail, Force, SkipConflicts, Interactive)
  - For each file:
    - Create directory structure
    - Create symlink (Unix) or copy (Windows by default)
    - If JSON + --merge: perform deep merge via json_merge.rs
  ↓
update_git_exclude() → Add section markers to .git/info/exclude
  ↓
save_overlay_state() → Write to .repoverlay/overlays/<name>.ccl (CCL format)
  ↓
save_external_state() → Write to ~/.local/share/repoverlay/applied/<hash>.ccl (backup)
```

### Remove Overlay

```
CLI: remove [<name>] --target <repo>
  ↓
load_overlay_state() from .repoverlay/overlays/<name>.ccl
  ↓
remove_overlay() in lib.rs:
  - For each FileEntry in state:
    - Delete symlink or file
    - Clean empty parent directories
  ↓
update_git_exclude() → Remove section markers from .git/info/exclude
  ↓
Delete .repoverlay/overlays/<name>.ccl
  ↓
remove_external_state() → Delete ~/.local/share/repoverlay/applied/<hash>.ccl
```

### Multi-Source Resolution

```
SourceReference::parse(input) → Structured enum (GitHub URL, Local, ThreePart, TwoPart, OnePart)
  ↓
SourceManager::resolve() → Check each source in priority order:
  1. Try to find overlay in first source
  2. If not found, try second source (first-match-wins)
  ↓
For OverlayRepo sources, attempt upstream fallback:
  - If direct lookup fails and upstream exists:
    - Try upstream-org/upstream-repo/overlay-name
    - Track resolution method (Direct vs Upstream)
  ↓
Return ResolvedOverlay {path, source, resolved_via, commit}
```

### Restore Overlays

```
load_external_states() from ~/.local/share/repoverlay/applied/
  ↓
For each external state backup:
  - Validate source reference is still resolvable
  - Apply using original source string (may fail if source moved)
  ↓
Report restore results (successes and failures)
```

### Update Overlays

```
list_applied_overlays() from .repoverlay/overlays/
  ↓
For each applied GitHub overlay:
  - Check remote HEAD commit via git ls-remote
  - Compare with cached commit
  - If newer commit exists:
    - Remove old overlay
    - Re-apply with --update flag (forces cache refresh)
```

### Create Overlay

```
detect_files() in detection.rs:
  - Find AI config files (claude, copilot, jetbrains, etc.)
  - Find gitignored files
  - Find untracked files
  ↓
Interactive selection via selection.rs if no --include flags:
  - Categorized view of detected files
  - Search, toggle, bulk select
  ↓
Copy selected files to output directory
  ↓
generate_repoverlay_ccl() → Create repoverlay.ccl metadata file
```

## Key Abstractions

**SourceReference:**
- Purpose: Parse and categorize user input strings
- Examples: GitHub URLs, three-part references (`org/repo/overlay`), local paths
- Pattern: Enum-based pattern matching with validation logic

**SourceManager:**
- Purpose: Coordinate resolution across multiple overlay sources
- Pattern: Iterator over managed sources with first-match-wins semantics
- Usage: Multi-source configurations check sources in priority order

**ConflictStrategy:**
- Purpose: Control behavior when applying overlays encounters conflicts
- Variants: `Fail` (default), `Force` (overwrite), `SkipConflicts`, `Interactive`
- Pattern: Enum-based behavior dispatch in `apply_overlay()` function

**OverlaySource:**
- Purpose: Represent the origin of an applied overlay
- Variants: `Local { path }`, `GitHub { owner, repo, git_ref, commit, ... }`, `OverlayRepo { org, repo, name, ... }`
- Pattern: Enum with tagged structs for variant-specific data

**ResolvedVia:**
- Purpose: Track how an overlay repository reference was resolved
- Variants: `Direct` (exact match), `Upstream` (fallback to fork upstream)
- Usage: Stored in state for transparency in `repoverlay status` output

**SelectableItem / SelectionResult:**
- Purpose: Terminal UI for interactive file selection
- Pattern: State machine with keyboard input handling via crossterm
- Usage: `create` command interactive mode

## Entry Points

**Main Entry (`main.rs`):**
- Location: `src/main.rs`
- Triggers: Binary execution
- Responsibilities: Initialize logger, call `lib::run()`

**CLI Run (`cli.rs::run()`):**
- Location: `src/cli.rs`
- Triggers: Called from `lib::run()`
- Responsibilities: Parse CLI args via clap, dispatch to command handlers

**Command Handlers:**
- Location: `src/cli.rs` (commands module)
- Each command (apply, remove, status, etc.) maps to a handler function
- Handlers construct arguments and call application logic from `lib.rs`

## Error Handling

**Strategy:** Layered error propagation with context

**Patterns:**
- Use `anyhow::Result<T>` for error propagation with `.context()` for error messages
- Use `anyhow::bail!()` for immediate error termination with custom messages
- Use `thiserror` crate for custom error types (not currently used extensively)
- CLI layer catches errors from logic functions and prints to stderr
- Exit code 1 on any error, 0 on success

**Common Error Cases:**
- Source not found or inaccessible
- Conflict detection during overlay application
- Invalid configuration files (CCL parsing)
- Git command failures
- File system access errors

## Cross-Cutting Concerns

**Logging:**
- Framework: `env_logger` + `log` crate
- Pattern: Log macros `debug!()`, `trace()` scattered throughout for visibility
- Controlled via `RUST_LOG` environment variable

**Validation:**
- Pattern: Validate early in CLI layer, propagate errors upward
- Examples: Git repo validation (`validate_git_repo()`), overlay name normalization (`normalize_overlay_name()`)
- Some validation happens in parsing layers (e.g., path component validation in `overlay_repo.rs`)

**Git Integration:**
- Pattern: Execute git commands via `Command::new("git")`
- Managed in `cache.rs` (git commands) and `lib.rs` (git exclude file management)
- All overlays tracked via `.git/info/exclude` with named section markers

**File System Operations:**
- Pattern: Use `fs::`, `walkdir::WalkDir`, `std::path` modules
- Symlink creation on Unix, copy fallback on Windows (controlled by platform detection and `--copy` flag)
- Careful handling of empty directory cleanup and error recovery

---

*Architecture analysis: 2026-02-27*
