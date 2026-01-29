# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Requirements

- **Rust edition**: 2024
- **Minimum Rust version**: 1.88
- **Linting**: Clippy with `pedantic` and `nursery` lints enabled

## Build and Development Commands

This project uses `just` as a task runner.

### Core Commands

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

### Additional Commands

```bash
just clean          # Clean build artifacts
just install        # Install binary locally (alias: i)
just watch-test     # Watch mode for tests (alias: wt)
just watch-lint     # Watch mode for clippy (alias: wl)
just test-coverage  # Run tests with coverage (alias: tc)
just coverage-html  # Generate HTML coverage report
just coverage-report # Open coverage report in browser
just audit          # Run security audit with cargo-audit and cargo-deny (alias: a)
just docs           # Build documentation (alias: d)
```

### Running Individual Tests

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

Tests are organized across multiple locations:

- **`tests/cli.rs`** - CLI integration tests using `assert_cmd`, verifying binary behavior
- **`tests/common/mod.rs`** - Shared test utilities and fixtures
- **`src/testutil.rs`** - Test helper module with `create_test_repo()` / `create_test_overlay()` functions
- **Unit tests** - Embedded within individual modules (`lib.rs`, `state.rs`, etc.)

Tests create temporary git repos using `tempfile::TempDir`. Some tests require serial execution due to environment variable handling (coverage runs use `--test-threads=1`).
