# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## v0.5.0 - 2026-02-14

### Added

- support bare owner names in source URL validation ([#89](https://github.com/tylerbutler/repoverlay/pull/89))
- *(cli)* add --force and --skip-conflicts flags for conflict handling ([#36](https://github.com/tylerbutler/repoverlay/pull/36))

### Fixed

- resolve overlay repo config from sources instead of legacy field ([#86](https://github.com/tylerbutler/repoverlay/pull/86))

### Other

- add badges and streamline README ([#87](https://github.com/tylerbutler/repoverlay/pull/87))
- *(deps)* bump taiki-e/install-action from 2.67.18 to 2.67.26 ([#71](https://github.com/tylerbutler/repoverlay/pull/71))
- *(deps)* bump release-plz/action from 0.5.124 to 0.5.126 ([#72](https://github.com/tylerbutler/repoverlay/pull/72))

## v0.4.0 - 2026-02-12

### Added

- validate and expand source URLs at deserialization time ([#83](https://github.com/tylerbutler/repoverlay/pull/83))
- auto-migrate legacy overlay_repo config to sources format ([#80](https://github.com/tylerbutler/repoverlay/pull/80))
- support multi-overlay selection in browse mode apply ([#77](https://github.com/tylerbutler/repoverlay/pull/77))
- add tree navigation with multi-level hierarchy for create UX ([#75](https://github.com/tylerbutler/repoverlay/pull/75))

### Other

- *(deps)* bump tiny-update-check from 0.1.0 to 1.0.0 ([#73](https://github.com/tylerbutler/repoverlay/pull/73))
- gitignore

## v0.3.3 - 2026-02-08

### Other

- configure repo policies

### Security

- bump deps to address vulnerabilities

## v0.3.2 - 2026-02-04

### Fixed

- prevent restore from re-applying explicitly removed overlays ([#67](https://github.com/tylerbutler/repoverlay/pull/67))

### Other

- add Marp presentation slides for repoverlay ([#63](https://github.com/tylerbutler/repoverlay/pull/63))

## v0.3.1 - 2026-02-04

### Fixed

- support git worktrees for exclude file management ([#65](https://github.com/tylerbutler/repoverlay/pull/65))

## v0.3.0 - 2026-02-03

### Added

- *(cli)* add update notifications ([#54](https://github.com/tylerbutler/repoverlay/pull/54))
- *(cli)* add shell completions command ([#51](https://github.com/tylerbutler/repoverlay/pull/51))
- *(ci)* add PR binary size comparison workflow ([#52](https://github.com/tylerbutler/repoverlay/pull/52))
- *(sources)* add unified overlay syntax ([#48](https://github.com/tylerbutler/repoverlay/pull/48))
- *(cli)* improve version string format for local builds ([#46](https://github.com/tylerbutler/repoverlay/pull/46))
- *(cli)* add dry-run flags, help headings, and create-local command ([#45](https://github.com/tylerbutler/repoverlay/pull/45))
- *(sources)* add multi-source overlay sharing ([#44](https://github.com/tylerbutler/repoverlay/pull/44))
- add debug logging and documentation improvements ([#34](https://github.com/tylerbutler/repoverlay/pull/34))

### Fixed

- *(resolve)* handle nested overlay repo structure correctly ([#50](https://github.com/tylerbutler/repoverlay/pull/50))
- *(ci)* checkout PR branch before pushing metrics updates

### Other

- *(deps)* bump dawidd6/action-download-artifact from 8 to 14 ([#57](https://github.com/tylerbutler/repoverlay/pull/57))
- add workflow to close dependabot PRs for generated files
- cargo update
- *(talk)* restructure to apply-first flow with unified syntax ([#47](https://github.com/tylerbutler/repoverlay/pull/47))
- *(deps)* bump the actions group with 5 updates ([#42](https://github.com/tylerbutler/repoverlay/pull/42))
- *(deps)* bump the rust-deps group with 2 updates ([#43](https://github.com/tylerbutler/repoverlay/pull/43))
- update sickle to pick up fixes
- add talk outline and Marp slide deck
- enhance justfile with organized recipes and bloat profile ([#41](https://github.com/tylerbutler/repoverlay/pull/41))
- add reusable actions and improved workflows ([#40](https://github.com/tylerbutler/repoverlay/pull/40))
- add Cargo.toml improvements for lints and profiles ([#38](https://github.com/tylerbutler/repoverlay/pull/38))
- add conventional commit enforcement tooling ([#39](https://github.com/tylerbutler/repoverlay/pull/39))
- add rust toolchain and formatting configuration ([#37](https://github.com/tylerbutler/repoverlay/pull/37))
- add cargo binstall command

## v0.2.1 - 2026-01-28

### Added

- *(overlay)* add directory symlink support ([#31](https://github.com/tylerbutler/repoverlay/pull/31))
- *(cli)* add subcommand to add files to existing overlays ([#30](https://github.com/tylerbutler/repoverlay/pull/30))

## v0.2.0 - 2026-01-26

### Added

- add fork inheritance for overlay resolution ([#24](https://github.com/tylerbutler/repoverlay/pull/24))
- add hk git hooks for lint and format ([#25](https://github.com/tylerbutler/repoverlay/pull/25))

### Other

- remove public library API ([#27](https://github.com/tylerbutler/repoverlay/pull/27))
- add license
- add Claude Code configuration and skills ([#22](https://github.com/tylerbutler/repoverlay/pull/22))

## v0.1.6 - 2026-01-22

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

## v0.1.5 - 2026-01-21

### Other

- simplify state format using sickle's improved serde support ([#11](https://github.com/tylerbutler/repoverlay/pull/11))
- improve documentation structure and clarity ([#10](https://github.com/tylerbutler/repoverlay/pull/10))
- document decision to use git CLI over git library
- extract library crate and reorganize tests ([#8](https://github.com/tylerbutler/repoverlay/pull/8))

## v0.1.4 - 2026-01-15

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

## v0.1.3 - 2026-01-07

### Other

- use PAT for release-plz to trigger release workflow

## v0.1.2 - 2026-01-07

### Other

- fix release-plz config to create tags for cargo-dist
- add automatic tag creation on release PR merge

## v0.1.1 - 2026-01-07

### Other

- add installation methods to README
- add cargo-dist for binary releases and Homebrew distribution

## v0.1.0 - 2026-01-07

### Added

- add GitHub repository overlay support
- add multi-overlay support
- initial repoverlay CLI implementation

### Other

- build binary before running tests
- fix workflow action names and release-plz config
- add README, DEV guide, and Claude Code instructions
- add CI/CD workflows and release automation

