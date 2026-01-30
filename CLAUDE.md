# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Quick Reference

- **Development guide**: [DEV.md](DEV.md) - build commands, testing, CI, release process
- **Architecture**: [ARCHITECTURE.md](ARCHITECTURE.md) - module structure, data flows, design decisions

## Key Commands

```bash
just check      # Run all checks: format, lint, test
just test       # Run all tests
just lint       # Run clippy lints
```

## Project Context

- **Rust 2024 edition**, minimum version 1.90
- **Clippy pedantic + nursery** lints enabled
- **CCL format** used for configuration and state files (not TOML)
- Tests use `tempfile::TempDir` for temporary git repos
