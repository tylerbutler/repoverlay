# Codebase Structure

**Analysis Date:** 2026-02-27

## Directory Layout

```
repoverlay/
├── src/                    # Core application source code (Rust)
│   ├── main.rs            # Binary entry point (minimal)
│   ├── lib.rs             # Library entry point, core operations
│   ├── cli.rs             # CLI command definitions and dispatch
│   ├── state.rs           # Overlay state persistence
│   ├── sources.rs         # Multi-source resolution manager
│   ├── reference.rs       # Input parsing and categorization
│   ├── github.rs          # GitHub URL parsing
│   ├── cache.rs           # GitHub repository caching
│   ├── config.rs          # Configuration file handling (CCL format)
│   ├── overlay_repo.rs    # Shared overlay repository management
│   ├── upstream.rs        # Git fork/upstream detection
│   ├── detection.rs       # File discovery for overlay creation
│   ├── selection.rs       # Interactive terminal UI
│   ├── json_merge.rs      # JSON deep merge utilities
│   ├── fuzzy.rs           # Fuzzy matching for overlay search
│   ├── overlay_name.rs    # Overlay name validation
│   └── testutil.rs        # Test helper functions
├── tests/                 # Integration tests
│   ├── cli.rs            # CLI integration tests (assert_cmd)
│   └── common/mod.rs     # Shared test utilities
├── Cargo.toml            # Package manifest (Rust 2024 edition)
├── Cargo.lock            # Dependency lock file
├── rust-toolchain.toml   # Rust version specification (1.90+)
├── rustfmt.toml          # Code formatting rules
├── ARCHITECTURE.md       # Architecture reference
├── DEV.md                # Development guide
├── docs/                 # Documentation
│   ├── adr/             # Architectural decision records
│   ├── plans/           # Planning documents
│   └── talks/           # Presentation slides
├── website/             # Astro documentation site
│   └── src/
│       └── content/docs/  # User documentation
├── scripts/             # Build and release scripts
└── metrics/             # Performance measurement data
```

## Directory Purposes

**src/**
- Purpose: Rust source code for the binary and library
- Contains: Module files implementing all functionality
- Key files: `lib.rs` (entry point), `cli.rs` (command routing), `state.rs` (persistence)

**tests/**
- Purpose: Integration tests using `assert_cmd` for CLI testing
- Contains: CLI behavior tests that run the compiled binary
- Key files: `cli.rs` (test cases), `common/mod.rs` (test fixtures)

**docs/adr/**
- Purpose: Architectural decision records documenting why design choices were made
- Contains: `.md` files explaining decisions, trade-offs, alternatives

**docs/plans/**
- Purpose: Planning and roadmap documents
- Contains: Feature plans, milestone definitions

**website/src/content/docs/**
- Purpose: User-facing documentation
- Contains: Guide documents, command reference, examples

**scripts/**
- Purpose: Automation for building, testing, releasing
- Contains: Shell scripts for CI/CD operations

## Key File Locations

**Entry Points:**
- `src/main.rs` - Binary entry point. Initializes logging, calls `lib::run()`
- `src/lib.rs` - Library entry point. Exports `pub fn run()` called from main
- `src/cli.rs` - CLI command parsing and dispatching. Implements `pub fn run()` that handles all commands

**Configuration:**
- `Cargo.toml` - Package metadata, dependencies, profiles, lint settings
- `rust-toolchain.toml` - Specifies Rust version 1.90+
- `rustfmt.toml` - Code formatting rules for `cargo fmt`

**Core Logic:**
- `src/lib.rs` - Primary application logic: `apply_overlay()`, `remove_overlay()`, `show_status()`, `restore_overlays()`, `update_overlays()`, `create_overlay()`, `switch_overlay()`, `browse_overlays()`, git exclude management
- `src/state.rs` - State models and persistence: `OverlayState`, `OverlaySource` enum, save/load functions
- `src/sources.rs` - Multi-source resolution: `SourceManager`, priority-based overlay lookup

**Source Resolution:**
- `src/reference.rs` - Parse user input into `SourceReference` enum (GitHub, local, three-part, etc.)
- `src/github.rs` - Parse GitHub URLs into `GitHubSource`, handle git refs
- `src/cache.rs` - Cache GitHub repos, execute git commands
- `src/overlay_repo.rs` - Manage shared overlay repositories, list available overlays

**Configuration & Detection:**
- `src/config.rs` - Load/serialize config in CCL format, `RepoverlayConfig` type
- `src/detection.rs` - Discover files for `create` command: AI configs, gitignored, untracked
- `src/upstream.rs` - Detect upstream repository from git remotes

**User Interaction:**
- `src/selection.rs` - Terminal UI for interactive file selection (crossterm-based)
- `src/fuzzy.rs` - Fuzzy matching for overlay name filtering

**Utilities:**
- `src/json_merge.rs` - Deep JSON merge for `--merge` flag
- `src/overlay_name.rs` - Overlay name validation and normalization
- `src/testutil.rs` - Test helpers: `create_test_repo()`, `create_test_overlay()`

**Testing:**
- `tests/cli.rs` - Integration tests covering help, version, apply, remove, status, etc.
- `tests/common/mod.rs` - `TestContext`, `SourceTestContext` test fixtures

## Naming Conventions

**Files:**
- Module files: snake_case (e.g., `github.rs`, `overlay_repo.rs`)
- Source files with one main struct/enum use struct name: `CacheManager` → `cache.rs`
- Test files match module: tests in subdirectory by convention

**Modules:**
- Public modules: `pub mod <name>;` in parent file
- Private modules: `mod <name>;` (not exported)
- Internal imports: `use crate::module_name;`

**Functions:**
- Public: snake_case, documented with `///` comments
- Private: snake_case with `pub(crate)` when needed by other modules
- Test functions: `#[test] fn test_<description>()`

**Types:**
- Structs: PascalCase (e.g., `OverlayState`, `CacheManager`, `GitHubSource`)
- Enums: PascalCase variants (e.g., `SourceReference::ThreePart`)
- Result/Option: No postfix, use `anyhow::Result<T>` pattern
- Error types: Use `anyhow::` macros for ad-hoc errors, custom Error enums rare

**Constants:**
- Module constants: SCREAMING_SNAKE_CASE (e.g., `STATE_DIR`, `MANAGED_SECTION_NAME`, `MAX_VISIBLE_ITEMS`)
- Const functions: allowed, used in constructors (e.g., `OverlaySource::local()`)

**Variables:**
- Local: snake_case, descriptive (e.g., `overlay_name`, `target_repo`, `resolved_path`)
- Loop variables: concise where clear (e.g., `entry`, `source`, `file`)

## Where to Add New Code

**New Feature/Command:**
1. Add subcommand struct to `Commands` enum in `src/cli.rs`
2. Implement handler in `src/cli.rs::run_<command>()`
3. Add core logic to `src/lib.rs` as new public/private functions
4. Add tests to `tests/cli.rs` for CLI behavior
5. Add integration tests if needed using `TestContext` from `tests/common/mod.rs`

**New Overlay Source Type:**
1. Add variant to `OverlaySource` enum in `src/state.rs`
2. Implement constructor methods: `pub fn <type_name>(...) -> Self`
3. Add display/serialization logic to impl blocks
4. Add resolution logic to `reference.rs` or `sources.rs` (depends on reference format)
5. Update `SourceManager::resolve()` if multi-source support needed

**New Validation/Utility:**
1. Keep module-specific: if it validates file names → `overlay_name.rs`, if it merges JSON → `json_merge.rs`
2. Create new module only if functionality spans multiple use cases
3. Test utilities go in `src/testutil.rs` or `tests/common/mod.rs`
4. Utility functions exposed via `pub fn` for cross-module usage

**New CLI Option:**
1. Add field to relevant subcommand struct in `src/cli.rs`
2. Use `#[arg(...)]` attributes for clap configuration
3. Pass through to handler function
4. Update help text via `#[doc]` and clap attributes

**New Integration:**
1. Dependencies added to `Cargo.toml` with version pinning
2. Use feature flags when dependencies are optional
3. Wrap external library in dedicated module (e.g., `cache.rs` wraps git, `selection.rs` wraps crossterm)
4. Export minimal public interface from wrapper module

## Special Directories

**`.repoverlay/`:**
- Purpose: In-repository state storage
- Generated: Yes (created on first overlay apply)
- Committed: No (added to `.git/info/exclude` or `.gitignore`)
- Structure: `.repoverlay/overlays/<name>.ccl` files for each applied overlay
- Structure: `.repoverlay/meta.ccl` for repository metadata

**`~/.local/share/repoverlay/applied/`:**
- Purpose: External backup of applied overlay state (for recovery after `git clean`)
- Generated: Yes (created on first overlay apply)
- Committed: No (external to repository)
- Structure: One `.ccl` file per applied overlay, named by hash

**`~/.cache/repoverlay/github/`:**
- Purpose: Cache of cloned GitHub repositories
- Generated: Yes (on first use of GitHub source)
- Committed: No (cache directory)
- Structure: `owner/repo/` subdirectories with shallow clones

**`~/.cache/repoverlay/sources/`:**
- Purpose: Cache of multi-source overlay repositories
- Generated: Yes (when multi-source config is used)
- Committed: No (cache directory)
- Structure: `<source-name>/` subdirectories with clones

**`~/.config/repoverlay/`:**
- Purpose: Global configuration
- Generated: Manual or via `repoverlay source add` command
- Committed: No (user-specific configuration)
- Structure: `config.ccl` file with sources list

---

*Structure analysis: 2026-02-27*
