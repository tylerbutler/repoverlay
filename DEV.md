# Development Guide

## Prerequisites

- **Rust** 1.90+ (2024 edition) - https://rustup.rs/
- **just** - Task runner - https://github.com/casey/just
- **git** - Required at runtime for GitHub overlay functionality

## Building

```bash
just build      # Debug build (alias: b)
just release    # Release build (alias: r)
```

## Testing

```bash
just test           # Run all tests (alias: t)
just test-verbose   # Run tests with output shown (alias: tv)
just test <name>    # Run specific test (via cargo test)
```

Run a single test directly:
```bash
cargo test <test_name>
cargo test apply::applies_single_file  # Run specific test module::test_name
```

### Test Organization

- **`tests/cli.rs`** - CLI integration tests using `assert_cmd`
- **`tests/common/mod.rs`** - Shared test utilities and fixtures
- **`src/testutil.rs`** - Test helper module with `create_test_repo()` / `create_test_overlay()`
- **Unit tests** - Embedded within individual modules (`lib.rs`, `state.rs`, etc.)

Tests create temporary git repos using `tempfile::TempDir`. Some tests require serial execution due to environment variable handling (coverage runs use `--test-threads=1`).

## Linting and Formatting

Clippy is configured with `pedantic` and `nursery` lints enabled.

```bash
just lint       # Run clippy (alias: l)
just format     # Format code (aliases: fmt, f)
just fmt-check  # Check formatting without changes (alias: fc)
just check      # Run format check, lint, and tests (alias: c)
```

## Running Locally

```bash
just run apply ./test-overlay
just run status
just run --help
```

Or install locally:

```bash
just install    # alias: i
repoverlay --help
```

## Additional Commands

```bash
just clean          # Clean build artifacts
just watch-test     # Watch mode for tests (alias: wt)
just watch-lint     # Watch mode for clippy (alias: wl)
just test-coverage  # Run tests with coverage (alias: tc)
just coverage-html  # Generate HTML coverage report
just coverage-report # Open coverage report in browser
just audit          # Run security audit with cargo-audit and cargo-deny (alias: a)
just docs           # Build documentation (alias: d)
```

## CI

The CI workflow runs on pull requests and pushes to main:

```bash
just ci   # Runs: test, lint, fmt-check
```

## Release Process

Releases are automated via [release-plz](https://release-plz.ieni.dev/):

1. **Automatic PR**: When commits are pushed to `main`, release-plz creates/updates a release PR with version bumps and changelog updates.

2. **Merge to release**: Merging the release PR creates a git tag (`v<version>`).

3. **Publish**: The tag triggers the release workflow which:
   - Publishes to crates.io
   - Creates a GitHub release with auto-generated notes

### Manual Version Bumps

release-plz determines version bumps from conventional commit messages:
- `fix:` - Patch version bump
- `feat:` - Minor version bump
- `feat!:` or `BREAKING CHANGE:` - Major version bump

### Required Secrets

- `CARGO_REGISTRY_TOKEN` - For publishing to crates.io

## Project Structure

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed module structure and responsibilities.
