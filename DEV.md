# Development Guide

## Prerequisites

- **Rust** (stable, 2024 edition) - https://rustup.rs/
- **just** - Task runner - https://github.com/casey/just
- **git** - Required at runtime for GitHub overlay functionality

## Building

```bash
just build      # Debug build
just release    # Release build
```

## Testing

```bash
just test           # Run all tests
just test-verbose   # Run tests with output shown
just test <name>    # Run specific test (via cargo test)
```

## Linting and Formatting

```bash
just lint       # Run clippy
just format     # Format code
just fmt-check  # Check formatting without changes
just check      # Run format check, lint, and tests
```

## Running Locally

```bash
just run apply ./test-overlay
just run status
just run --help
```

Or install locally:

```bash
just install
repoverlay --help
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

```
src/
├── main.rs    # CLI entry point and command handlers
├── state.rs   # State persistence (in-repo and external backup)
├── github.rs  # GitHub URL parsing
└── cache.rs   # GitHub repository cache management
```

See `CLAUDE.md` for detailed architecture documentation.
