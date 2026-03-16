# Copilot Instructions

## Build, Test, and Lint

```bash
just check      # Run all checks: format, lint, test
just test       # Run all tests (alias: t)
just lint       # Run clippy lints (alias: l)
just format     # Format code (alias: f)
```

Run a single test:

```bash
cargo test <test_name>
cargo test apply::applies_single_file   # module::test_name pattern
just test <name>                        # passes args to cargo test
```

## Architecture

repoverlay is a CLI tool that overlays config files into git repositories without committing them. Files are symlinked (or copied) from overlay sources and excluded via `.git/info/exclude`.

- **Single public entry point**: `lib.rs` exposes only `run()`. All modules are `pub(crate)`.
- **CLI layer** (`cli.rs`): clap derive macros define subcommands. Delegates to core operations in `lib.rs`.
- **State layer** (`state.rs`): Dual-write persistence — in-repo (`.repoverlay/overlays/<name>.ccl`) and external backup (`~/.local/share/repoverlay/applied/`) for recovery after `git clean`.
- **Source resolution** (`sources.rs`, `overlay_repo.rs`): Priority-ordered multi-source resolution with first-match-wins semantics. Supports local paths, GitHub URLs, and shared overlay repo references (`org/repo/name`).
- **Fork inheritance** (`upstream.rs`): Overlays from upstream repositories are automatically inherited by forks via remote detection.
- **Caching** (`cache.rs`): GitHub repos cached in `~/.cache/repoverlay/github/owner/repo/` using shallow clones.

## Key Conventions

- **Rust 2024 edition**, minimum version 1.91.
- **Clippy pedantic + nursery** lints are enabled (see `[lints.clippy]` in `Cargo.toml` for allowed exceptions).
- **CCL format** (via `sickle` crate) is used for all configuration and state files — not TOML or JSON.
- **`pub(crate)` visibility**: Used explicitly on all internal APIs for clarity, even in private modules (`redundant_pub_crate` lint is allowed).
- **Error handling**: `anyhow::Result` for application errors, `thiserror` for typed errors.
- **Security**: Path components (org, repo, overlay names) are validated against traversal attacks. Repository URLs must use `https://`, `ssh://`, or `git@` schemes. Use `--` separator before positional args in git commands to prevent flag injection.
- **Tests**: Use `tempfile::TempDir` for temporary git repos. Test helpers in `src/testutil.rs` (`create_test_repo()`, `create_test_overlay()`). CLI integration tests in `tests/cli.rs` use `assert_cmd`.
- **Changelogs**: Created via `changie new` (or `just change`). Entries go in `.changes/unreleased/`.
- **Conventional commits**: Used for automated version bumps (`fix:` = patch, `feat:` = minor, `feat!:` = major).
- **rustfmt**: 100 char max width, 4-space tabs, 2024 edition.

## Version Control

This repository optionally uses **[jj (Jujutsu)](https://jj-vcs.github.io/jj/)** for version control, colocated with git. Check which VCS is active by looking for a `.jj/` directory at the repo root. If present, use `jj` commands; otherwise fall back to standard `git`.

When using jj:

- `jj st` — status
- `jj log` — history
- `jj diff` — working copy changes
- `jj new` — create a new change
- `jj bookmark create <name> -r @` — create a bookmark (branch) at the current change
- `jj bookmark set <name> -r @` — move a bookmark to the current change
- `jj git push --bookmark <name>` — push a bookmark to the remote

Key differences from git:
- **No staging area** — all working copy changes are automatically part of the current change.
- **Bookmarks** are jj's equivalent of git branches.
- **`jj describe`** sets the commit message for the current change.
- **`jj squash`** folds the current change into its parent.
