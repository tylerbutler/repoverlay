# repoverlay - Project Overview

## Purpose
repoverlay is a CLI tool that overlays config files into git repositories without committing them. It supports both local overlays and GitHub repository overlays.

## Tech Stack
- **Language**: Rust (2024 edition)
- **CLI Framework**: clap with derive feature
- **Serialization**: serde + toml
- **File Walking**: walkdir
- **Terminal Colors**: colored
- **Error Handling**: anyhow + thiserror
- **Date/Time**: chrono
- **URL Parsing**: url
- **Directories**: directories (for platform-specific paths)
- **Testing**: tempfile, assert_cmd, predicates

## Module Structure
- `src/main.rs` - CLI entry point, command handlers, core logic
- `src/state.rs` - State persistence layer (in-repo and external backup)
- `src/github.rs` - GitHub URL parsing
- `src/cache.rs` - GitHub repository caching

## Key Features
1. Apply overlays from local directories or GitHub URLs
2. Symlink or copy files into target repositories
3. Track applied overlays with state files
4. Manage git exclude to prevent accidental commits
5. Restore overlays after git clean
6. Update overlays from remote sources
7. Cache management for GitHub repositories
