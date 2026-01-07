# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

This project uses `just` as a task runner. Common commands:

```bash
just build       # Build in debug mode (alias: b)
just release     # Build in release mode (alias: r)
just test        # Run all tests (alias: t)
just test-verbose # Run tests with output shown (alias: tv)
just lint        # Run clippy lints (alias: l)
just format      # Format code (aliases: fmt, f)
just fmt-check   # Check formatting without changes (alias: fc)
just check       # Run all checks: format, lint, test (alias: c)
just ci          # Run full CI suite: test, lint, fmt-check
just run <args>  # Run the binary with arguments
```

Run a single test:
```bash
cargo test <test_name>
cargo test apply::applies_single_file  # Run specific test module::test_name
```

## Architecture Overview

repoverlay is a CLI tool that overlays config files into git repositories without committing them. It supports both local overlays and GitHub repository overlays.

### Module Structure

- **`src/main.rs`** - CLI entry point using clap. Contains the core command handlers (`apply_overlay`, `remove_overlay`, `show_status`, `restore_overlays`, `update_overlays`) and git exclude file management.

- **`src/state.rs`** - State persistence layer. Manages overlay state in two locations:
  - In-repo: `.repoverlay/overlays/<name>.toml` - tracks applied overlays
  - External: `~/.local/share/repoverlay/applied/` - backup for recovery after `git clean`

- **`src/github.rs`** - GitHub URL parsing. Handles URL formats like `https://github.com/owner/repo/tree/branch/subpath` and extracts owner, repo, ref, and subpath components.

- **`src/cache.rs`** - GitHub repository caching. Manages cloned repos in `~/.cache/repoverlay/github/owner/repo/`. Supports shallow clones and update checking.

### Key Data Flow

1. **Apply**: Source (local path or GitHub URL) -> `resolve_source()` -> files walked -> symlinks/copies created -> state saved -> git exclude updated
2. **Remove**: Load state -> remove files -> clean empty dirs -> update git exclude -> remove state
3. **Restore**: Load external state backup -> re-apply overlays (for recovery after `git clean`)

### State File Format

Overlay state is stored as TOML with source information (local path or GitHub metadata), applied timestamp, and file entries (source path, target path, link type).

### Git Integration

Overlay files are excluded from git tracking via `.git/info/exclude` using named sections:
```
# repoverlay:<overlay-name> start
.envrc
# repoverlay:<overlay-name> end
```

## Testing

Tests are organized in `src/main.rs` under `mod tests`:
- Unit tests: `remove_section`, `state` tests
- Integration tests: `apply`, `remove`, `status` modules
- CLI tests: `cli` module using `assert_cmd`

Tests create temporary git repos using `tempfile::TempDir` and the `create_test_repo()` / `create_test_overlay()` helpers.

## Rust Edition

Uses Rust 2024 edition (`edition = "2024"` in Cargo.toml).
