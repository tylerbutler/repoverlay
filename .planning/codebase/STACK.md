# Technology Stack

**Analysis Date:** 2026-02-27

## Languages

**Primary:**
- Rust 2024 edition (minimum version 1.90) - Entire codebase and binary distribution

## Runtime

**Environment:**
- Native compiled Rust binary (runs on Linux, macOS, Windows, WSL)
- No runtime dependencies required (standalone executable)

**Package Manager:**
- Cargo (Rust package manager)
- Lockfile: Cargo.lock (committed)

## Frameworks

**Core:**
- clap 4.5.60 - CLI argument parsing with derive macros
- clap_complete 4.5 - Shell completion generation
- clap-markdown 0.1.5 - Markdown help documentation generation

**Serialization:**
- serde 1.0.228 - Serialization framework with derive
- serde_json 1.0 - JSON parsing and serialization

**Error Handling:**
- anyhow 1.0.102 - Flexible error handling
- thiserror 2.0.18 - Derive macros for error types

**Testing:**
- No external test framework (uses Rust's built-in #[test])
- assert_cmd 2 - Command-line application testing
- predicates 3 - Assertions for test output
- tempfile 3 - Temporary directories for test repositories

**Build/Dev:**
- vergen 9 - Version generation from git metadata (build-time)
- cargo-dist 0.30.3 - Distribution and release packaging
- cargo-nextest - Parallel test execution (optional)
- cargo-llvm-cov - Code coverage reporting (optional)
- cargo-watch - File watching for development (optional)

## Key Dependencies

**Critical:**
- walkdir 2.5.0 - Recursive directory traversal for overlay file discovery
- log 0.4 + env_logger 0.11 - Structured logging with environment control
- colored 3.1.1 - Terminal color output for CLI feedback
- directories 6.0.0 - Cross-platform standard directory paths (cache, config)
- dirs 6.0.0 - Home directory discovery

**Git Integration:**
- url 2.5.8 - URL parsing for GitHub repository validation and parsing
- Command::new("git") - Shell invocation to git CLI (no Rust git library)

**User Interaction:**
- dialoguer 0.12.0 - Interactive CLI dialogs for conflict resolution
- crossterm 0.29.0 - Cross-platform terminal control (colors, cursor)
- fuzzy-matcher 0.3 - Fuzzy matching for overlay selection
- similar 2.7.0 - Diff/similarity computation for conflict display

**Infrastructure:**
- sickle 0.1.2 - Configuration file parsing (for .yaml/.toml state files)
- tiny-update-check 1.0 - Check for new versions from crates.io

## Configuration

**Environment:**
- RUST_BACKTRACE=1 - Backtrace printing in tests and development (set in justfile)
- REPOVERLAY_CI_BUILD - Feature flag for CI build detection (set in CI workflows)
- Log level control via env_logger (RUST_LOG environment variable)

**Build:**
- Cargo.toml with three additional profiles:
  - dist: Inherits from release with LTO thin for distribution
  - profiling: Release with debug symbols for performance analysis
  - bloat: Release with panic=unwind for binary size analysis

**Linting:**
- Clippy pedantic and nursery lints enabled with warnings-as-errors (-D warnings)
- Specific pedantic lints allowed: missing_errors_doc, missing_panics_doc, module_name_repetitions, similar_names, too_many_lines, cognitive_complexity, significant_drop_tightening

## Platform Requirements

**Development:**
- Rust 1.90+ toolchain with rustfmt, clippy, llvm-tools-preview
- Git 2.0+ (for git remote/clone operations)
- Standard build tools (gcc/clang for linking)
- Python (via mise for dev tools)
- mise, just, changie, hk tools (configured in mise.toml)

**Production:**
- Linux (ubuntu-22.04+ tested), macOS, Windows (with longpaths enabled in CI)
- Git client installed in PATH

## Release Process

**Distribution:**
- cargo-dist 0.30.3 - Multi-platform binary distribution
- Targets: Linux x86_64/aarch64, macOS universal (x86_64 + aarch64), Windows x86_64
- Packaging formats: .tar.gz (Unix), .zip (Windows), standalone .msi installers

**Version Management:**
- changie - Changelog management with unreleased entries
- Semantic versioning (MAJOR.MINOR.PATCH)
- Git tags trigger automated release builds (tag pattern: [0-9]+.[0-9]+.[0-9]+*)

**Installation Channels:**
- Binary releases on GitHub (github.com/tylerbutler/repoverlay/releases)
- Homebrew formula published to tylerbutler/homebrew-tap
- Cargo/crates.io (as distrib builds)

## CI/CD & Testing

**Continuous Integration:**
- GitHub Actions (ubuntu-22.04 runners primary)
- Jobs: test, lint (clippy), format (rustfmt), documentation checks
- Pull request triggers
- Main branch: binary size tracking with artifact storage (90 days retention)

**Code Coverage:**
- codecov.yml configuration for coverage reporting
- Target: auto with 2% threshold
- Excluded: src/testutil.rs (test-only utilities)

**Security:**
- cargo audit - Dependency vulnerability scanning
- cargo deny - Comprehensive supply chain security
- Dependabot for automated dependency updates

---

*Stack analysis: 2026-02-27*
