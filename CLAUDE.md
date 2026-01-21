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

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed architecture documentation including:
- Module structure and responsibilities
- Data flow diagrams for each operation
- State file format
- Git integration details
- Caching strategy

## Testing

Tests are organized in `src/main.rs` under `mod tests`:
- Unit tests: `remove_section`, `state` tests
- Integration tests: `apply`, `remove`, `status`, `create`, `switch` modules
- CLI tests: `cli` module using `assert_cmd`

Tests create temporary git repos using `tempfile::TempDir` and the `create_test_repo()` / `create_test_overlay()` helpers.

## Rust Edition

Uses Rust 2024 edition (`edition = "2024"` in Cargo.toml).
