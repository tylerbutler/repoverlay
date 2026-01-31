# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.2](https://github.com/tylerbutler/repoverlay/compare/v0.2.1...v0.2.2) - 2026-01-31

### Added

- *(cli)* add dry-run flags, help headings, and create-local command ([#45](https://github.com/tylerbutler/repoverlay/pull/45))
- *(sources)* add multi-source overlay sharing ([#44](https://github.com/tylerbutler/repoverlay/pull/44))
- add debug logging and documentation improvements ([#34](https://github.com/tylerbutler/repoverlay/pull/34))

### Other

- *(deps)* bump the rust-deps group with 2 updates ([#43](https://github.com/tylerbutler/repoverlay/pull/43))
- update sickle to pick up fixes
- add talk outline and Marp slide deck
- enhance justfile with organized recipes and bloat profile ([#41](https://github.com/tylerbutler/repoverlay/pull/41))
- add reusable actions and improved workflows ([#40](https://github.com/tylerbutler/repoverlay/pull/40))
- add Cargo.toml improvements for lints and profiles ([#38](https://github.com/tylerbutler/repoverlay/pull/38))
- add conventional commit enforcement tooling ([#39](https://github.com/tylerbutler/repoverlay/pull/39))
- add rust toolchain and formatting configuration ([#37](https://github.com/tylerbutler/repoverlay/pull/37))
- add cargo binstall command

## [0.2.1](https://github.com/tylerbutler/repoverlay/compare/v0.2.0...v0.2.1) - 2026-01-28

### Added

- *(overlay)* add directory symlink support ([#31](https://github.com/tylerbutler/repoverlay/pull/31))
- *(cli)* add subcommand to add files to existing overlays ([#30](https://github.com/tylerbutler/repoverlay/pull/30))

## [0.2.0](https://github.com/tylerbutler/repoverlay/compare/v0.1.6...v0.2.0) - 2026-01-26

### Added

- add fork inheritance for overlay resolution ([#24](https://github.com/tylerbutler/repoverlay/pull/24))
- add hk git hooks for lint and format ([#25](https://github.com/tylerbutler/repoverlay/pull/25))

### Other

- remove public library API ([#27](https://github.com/tylerbutler/repoverlay/pull/27))
- add license
- add Claude Code configuration and skills ([#22](https://github.com/tylerbutler/repoverlay/pull/22))

## [0.1.6](https://github.com/tylerbutler/repoverlay/compare/v0.1.5...v0.1.6) - 2026-01-22

### Added

- simplify overlay publishing workflow ([#16](https://github.com/tylerbutler/repoverlay/pull/16))
- *(create)* add interactive file selection UI with category filters ([#17](https://github.com/tylerbutler/repoverlay/pull/17))
- use ~/.config for config and default create to overlay repo ([#12](https://github.com/tylerbutler/repoverlay/pull/12))

### Fixed

- improve terminal interactivity detection
- use output_dir for create command default path ([#15](https://github.com/tylerbutler/repoverlay/pull/15))

### Other

- improve code coverage for overlay_repo and selection modules ([#21](https://github.com/tylerbutler/repoverlay/pull/21))
- improve code coverage for cache, lib, and main modules ([#20](https://github.com/tylerbutler/repoverlay/pull/20))
- *(deps)* upgrade dependencies ([#19](https://github.com/tylerbutler/repoverlay/pull/19))

## [0.1.5](https://github.com/tylerbutler/repoverlay/compare/v0.1.4...v0.1.5) - 2026-01-21

### Other

- simplify state format using sickle's improved serde support ([#11](https://github.com/tylerbutler/repoverlay/pull/11))
- improve documentation structure and clarity ([#10](https://github.com/tylerbutler/repoverlay/pull/10))
- document decision to use git CLI over git library
- extract library crate and reorganize tests ([#8](https://github.com/tylerbutler/repoverlay/pull/8))

## [0.1.4](https://github.com/tylerbutler/repoverlay/compare/v0.1.3...v0.1.4) - 2026-01-15

### Added

- add overlay repository management with CCL config format
- add interactive mode for overlay creation
- add smart discovery for overlay creation
- add create and switch commands

### Fixed

- coverage workflow builds binary before running tests
- resolve clippy warnings and coverage workflow issues

### Other

- improve test coverage for cache, config, github, and overlay_repo modules
- add code coverage, security audit, and documentation checks
- extract helper functions to reduce code duplication

## [0.1.3](https://github.com/tylerbutler/repoverlay/compare/v0.1.2...v0.1.3) - 2026-01-07

### Other

- use PAT for release-plz to trigger release workflow

## [0.1.2](https://github.com/tylerbutler/repoverlay/compare/v0.1.1...v0.1.2) - 2026-01-07

### Other

- fix release-plz config to create tags for cargo-dist
- add automatic tag creation on release PR merge

## [0.1.1](https://github.com/tylerbutler/repoverlay/compare/v0.1.0...v0.1.1) - 2026-01-07

### Other

- add installation methods to README
- add cargo-dist for binary releases and Homebrew distribution

## [0.1.0](https://github.com/tylerbutler/repoverlay/releases/tag/v0.1.0) - 2026-01-07

### Added

- add GitHub repository overlay support
- add multi-overlay support
- initial repoverlay CLI implementation

### Other

- build binary before running tests
- fix workflow action names and release-plz config
- add README, DEV guide, and Claude Code instructions
- add CI/CD workflows and release automation
