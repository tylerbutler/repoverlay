# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## v0.11.0 - 2026-03-17


### Fixes


#### Fixed

##### Write git exclude entries to the common git directory for worktrees

Git reads `info/exclude` from the shared `.git/` directory, not the worktree-specific `$GIT_DIR`. Overlay files applied in worktree checkouts would show as untracked because exclude entries were written to the wrong location. Now uses `git rev-parse --git-path` to resolve the correct path.


### Command: `library`


#### Added

##### Add in-repo overlay library (`repoverlay library`)

You can now store overlays directly in your repository at `.repoverlay/library/`, making them shareable with your team via version control. The new `library` subcommand group provides full lifecycle management:

- `library list` — see what's in the library

- `library import <path-or-name>` — add an overlay (accepts filesystem paths or applied overlay names)

- `library export <name> <dest>` — copy an overlay out of the library

- `library remove <name>` — delete from the library

Library overlays integrate throughout the tool:

- `apply my-overlay` resolves from the library first (highest priority)

- `apply my-overlay --from @library` targets the library exclusively

- `create --into library` creates overlays directly in the library

- `browse` includes library overlays alongside configured sources

- `browse` works with library-only repos — no external sources required

The library path defaults to `.repoverlay/library/` but is configurable via `library_path` in your repo's `repoverlay.ccl` config. `.gitignore` is automatically updated to ensure library contents are tracked by git.


### Command: `browse`


#### Added

##### Show last-updated timestamps for applied overlays in browse UI

Applied overlays now display relative timestamps (e.g. '2 days ago') in the browse selection UI instead of a plain 'already applied' label, making it easier to see how recently each overlay was synced.

##### Auto-detect flat folder layout for local sources

Local directory sources no longer require `org/repo/name` nesting. Flat directories are auto-detected: a directory with overlay files is treated as a single overlay, and subdirectories are each treated as separate overlays.


#### Changed

##### Promote browse as the primary overlay workflow

Updated help text, README, and CLI reference to recommend `browse` as the primary entry point for interactive use. The `apply` command remains available for scripting and power users. Improved the error message when no sources are configured.


### Command: `create`


#### Added

##### Support glob patterns in `--include` flag

The `--include` flag on `create` now accepts glob patterns (e.g., `*.md`, `.claude/**`) in addition to exact file paths. Globs are expanded relative to the source repository root using standard glob syntax.


#### Fixed

##### Discover committed config files in source repositories

`create` now discovers tracked config files (e.g., `.envrc`, `.vscode/`, `justfile`, `Cargo.toml`, `Dockerfile`) in addition to AI configs, gitignored, and untracked files. Detection uses an exclusion-based heuristic that filters out source code, documentation, and media rather than trying to allowlist config patterns. Also fixes output path to create overlay at `output/<name>/` subdirectory.


#### Performance

##### Fix UI hang on large repos during overlay creation

Skip transient tool state directories (worktrees/, todos/) during AI config directory walking, reducing discovered files from ~59K to ~22 on repos with large .claude/ directories. Also fix O(n²) ancestor traversal in the selection UI with a pre-built parent lookup map, and add a progress spinner with Ctrl+C support for git clone/pull operations.


### Command: `edit`


#### Fixed

##### Fix `edit add` failing on directories with "Is a directory" error

The `edit add` command now correctly handles directories (e.g., `.claude/commands`) by using recursive directory copy, directory symlinks, and `EntryType::Directory` in overlay state. Rollback logic also properly restores directories on failure.

##### Remove git remote requirement for locally-applied overlays

`edit add` no longer requires a git remote origin when working with overlays applied from local paths. Remote detection is deferred to only when needed for overlay repo auto-commit.

##### Persist edit exclusions across remove/reapply cycles

Files removed via `edit remove` now stay removed when an overlay is removed and reapplied. Exclusions are tracked in overlay state and persisted in the external backup so they survive the full lifecycle.


### Command: `restore`


#### Fixed

##### Restore broken symlinks without requiring --force

`restore` no longer errors when an overlay's state exists but its symlinked files have been deleted. Since restore's purpose is to re-create missing files, it now always forces past the "already applied" check.


### Command: `source`


#### Added

##### Support local directory sources for overlays

Register directories as overlay sources using `source add ./path`. Local sources are stored in a per-repo config file (`.repoverlay/config.ccl`), which is automatically git-excluded. Paths must start with `/`, `./`, or `~` to be recognized as local (otherwise treated as git URL/shorthand). Local sources skip cloning and caching — overlays are read directly from the filesystem. Use `source list` and `source remove` to manage both global git sources and repo-local sources.


### Command: `switch`


#### Added

##### Add --dry-run support to switch command

The `switch` command now supports `--dry-run` to preview what would change without making modifications, consistent with `apply`, `remove`, `restore`, and `update`.


### Command: `sync`


#### Fixed

##### Skip non-syncable overlays gracefully in --all mode

`sync --all` no longer fails when only locally-applied overlays are present. The overlay repo manager is now lazily initialized, only created when a syncable overlay is encountered. Local and GitHub overlays are skipped with a warning message.


## v0.10.1 - 2026-03-05


### Command: `sync`


#### Fixed

##### Fix `sync <name>` using wrong org/repo for overlays applied via upstream fallback

When syncing a single overlay by name in a fork repository, the sync command detected the org/repo from the git remote (e.g., `alexvy86/FluidFramework`) instead of using the org/repo saved in the overlay state (e.g., `microsoft/FluidFramework`). This caused sync to fail with "does not exist in overlay repo" because the fork's org/repo path doesn't exist in the overlay repo. Now uses the org/repo from the saved state, matching the behavior of `sync --all`.


## v0.10.0 - 2026-03-05


### Command: `apply`


#### Added

##### Auto-detect configured source when applying via GitHub URL

When applying an overlay via a GitHub URL that matches a configured source, resolve it as an overlay repo (editable and syncable) instead of a read-only GitHub source. URLs with an org/repo/name subpath redirect to three-part resolution; bare repo URLs use interactive browse mode with the matched source.


### Command: `sync`


#### Fixed

##### Fix `sync <name>` failing for GitHub-sourced overlays that match a configured source

The single-name sync path was missing the `try_upgrade_github_source()` call that was added to `sync --all`, `edit`, and `edit add` in #171. This caused GitHub-sourced overlays to be rejected as non-syncable instead of being lazily upgraded to editable overlay repo sources.





#### Fixed

##### Improve error display and add SIGPIPE handling

Fixed error output formatting to use Display instead of Debug format. Added SIGPIPE signal handling so piped output to commands like `head` exits cleanly without "Broken pipe" errors.


## v0.9.2 - 2026-03-04


### Command: `edit`


#### Changed

##### Split `edit` into `edit add` and `edit remove` subcommands

The `--add` and `--remove` flags used greedy `num_args = 1..` parsing, which consumed trailing arguments and required the overlay name to appear before any flags. This was confusing and the help text couldn't convey the constraint clearly. The `edit` command now uses proper subcommands:

  repoverlay edit add my-overlay file1.txt file2.txt   repoverlay edit remove my-overlay oldfile.txt   repoverlay edit my-overlay  # interactive file selection   repoverlay edit             # select overlay then edit interactively

Running `edit` with no overlay name now presents an interactive overlay picker (auto-selects when only one overlay is applied). The old `--add`/`--remove`/`--interactive` flags still work but are hidden from help and print a deprecation warning.


## v0.9.1 - 2026-03-04


### Command: `edit`


#### Fixed

##### Fix `edit --add` dropping existing git exclude entries

When adding files to an applied overlay, the git exclude section was rewritten with only the newly added files, silently removing entries for previously managed files. This caused those files to reappear in `git status` after the add operation. Rebuilt the full exclude list from overlay state, matching the pattern already used by `edit --remove`.


#### Changed

##### Clarify that overlay NAME must precede --add/--remove flags

The --add and --remove flags accept multiple values and greedily consume trailing arguments. Running `edit --add file.txt name` fails because `name` is parsed as a second file. Updated help text to document the required argument order.


## v0.9.0 - 2026-03-02


#### Added

##### Always sync cache and overlay repos before operations

All commands that read from the overlay repo or GitHub cache now pull the
latest by default. The `--update` flag is replaced with `--no-update` to
opt out (e.g., for offline use). Affected commands: `apply`, `browse`,
`list`, `switch`, `create`, and `sync`.

**BREAKING:** The `--update` flag has been removed. Use `--no-update` to
skip syncing.


## v0.8.0 - 2026-02-28


### Command: `apply`


#### Added

##### Add interactive conflict resolution mode

Use `--interactive` (`-i`) to be prompted for each file conflict during apply, restore, and update. When a conflict is detected, you can choose to overwrite the file, skip it, view a diff, or abort the operation. The `--force` flag can also be written as `--overwrite`.

##### Prompt to save source on first use

When `apply` resolves a username or owner/repo reference for the first time, it now prompts the user to save it as a configured source for future use. The prompt is skipped if the source is already configured or in non-interactive mode.


### Command: `browse`


#### Added

##### Allow browsing without a configured source

`browse` now accepts an optional source argument (GitHub username, owner/repo, or URL) to fetch and browse overlays without adding a persistent source. Existing behavior using configured sources is unchanged when no argument is provided.


### Command: `create`


#### Added

##### Auto-apply overlay after creation (#144)

The `create` command now automatically applies the overlay to the source
repository after creating it. Files are replaced with symlinks, overlay
state is saved, and git exclude is updated. Both local and overlay-repo
modes are supported. Dry-run mode skips the apply step.


### Command: `edit`


#### Fixed

##### Interactive edit handles non-OverlayRepo sources correctly (#148)

`add_files_to_overlay` is now source-type-aware and includes a rollback
mechanism. Local overlays copy files to the overlay directory; GitHub
overlays are rejected with a clear error. If any operation fails mid-way,
all completed operations are rolled back to prevent partial state.


### Command: `update`


#### Fixed

##### Fix update showing incorrect source type for overlay repo sources (#145)

Running `update` on overlays from an overlay repo incorrectly displayed
messages meant for local sources. Each source type now shows the correct
label and update behavior.





#### Fixed

##### Fix apply and sync failing for local overlay sources (#143)

Overlays from local directory sources were incorrectly handled through the
remote overlay path, causing apply and dry-run to fail. Local sources now
resolve correctly.


#### Changed

##### Remove legacy overlay_repo config support (#79)

The deprecated `overlay_repo` configuration field and automatic migration
from the old format have been removed. Users must use the `sources`
configuration format. The `OverlayRepoConfig` struct is retained for use
by the sources system.

##### Remove deprecated commands: add, publish, list, create-local (#84)

Removed hidden/deprecated command variants that have been replaced:
`add` (use `edit --add`), `publish` (use `create`), `list` (use `browse`),
and `create-local` (use `create --local`).


## v0.7.0 - 2026-02-18


### Command: `apply`


#### Added

##### Show already-applied overlays as disabled in the overlay picker

The interactive overlay picker now displays already-applied overlays as dimmed, non-selectable entries so users can see what's active without leaving the selection UI.


### Command: `browse`


#### Added

##### Add interactive overlay selection and apply to `browse` command

When running in an interactive terminal, `browse` now presents a multi-select picker to choose overlays and applies them directly. Already-applied overlays appear disabled. Non-interactive mode (piped output or `--no-interactive`) continues to list overlays as text. Also adds `--target`, `--no-interactive`, and `--dry-run` flags.


### Command: `remove`


#### Added

##### Replace numbered list with multi-select picker for overlay removal

The `remove` command now uses an interactive multi-select picker with keyboard navigation, search filtering, and select-all toggling, replacing the old numbered-list prompt. Multiple overlays can be removed at once.


### Command: `status`


#### Added

##### Add `--json` flag to `status` command for machine-readable output

Outputs applied overlay state as structured JSON including overlay name, source info, applied timestamp, and per-file status (`ok` or `missing`). Useful for scripting and CI integration (e.g. `repoverlay status --json | jq ...`). Works with `--name` filter to output a single overlay.

##### Add `--quiet` flag to `status` command for exit-code-only checks

Exits with code 0 if overlays are applied, 1 if none. Produces no output. Useful for conditional scripts (e.g. `if repoverlay status -q; then ...`).


### Command: `sync`


#### Added

##### Add --all flag to sync all applied overlays at once

Only syncs overlays sourced from the overlay repo. Overlays from local or GitHub sources are skipped with a warning. Supports --dry-run.


#### Fixed

##### Fix sync --all skipping overlays from the overlay repo GitHub source

Overlays applied via two-part browse mode (e.g., `repoverlay apply owner/repo-overlays`) are stored with a GitHub source type rather than an OverlayRepo source type. The sync command now detects these by checking if the GitHub URL matches a configured overlay source and the subpath contains a valid org/repo/name reference.





#### Added

##### Auto-filter overlays by current repository

When browsing or selecting overlays, the current repository is auto-detected from git remotes (origin and upstream). Matching overlays are shown first; non-matching overlays are labeled "different repo" in the interactive picker. Text listing filters to matching overlays by default. Use `--show-all` to see all overlays.

##### Add documentation site at repoverlay.tylerbutler.com

Full docs site with installation instructions, quick start guide, concept explanations (overlay repos, sources, fork inheritance, configuration), usage guides, and CLI reference.


#### Changed

##### Rationalize command vocabulary and deprecate old commands before 1.0

Several commands have been renamed or consolidated for consistency. The old names still work but are hidden from help output and print a deprecation warning. They will be removed in 1.0.

**`list` → `browse`** — Browse available overlays from the overlay repository. Flags (`--filter`, `--update`) are unchanged.

**`create-local` → `create --output <path>`** — Local overlay creation is now a mode of the `create` command. Pass `--output` to write to a local directory instead of the overlay repository; the overlay name becomes optional in this mode.

**`cache clear` → `cache remove --all`** — Cache removal is unified under `cache remove`. Use `--all` to clear everything, or pass a specific `owner/repo` to remove a single cached repository.

**`publish` → `create`** — Overlay publishing is now handled by `create`, which auto-detects the target repository from the git remote or accepts an explicit `org/repo/name` path.

**`add` → `edit --add`** — Adding files to an existing overlay is now a flag on the `edit` command. Use `--add` (repeatable) to include new files and `--remove` to drop files in a single invocation.


#### Security

##### Reject symlinks that escape the overlay source directory during copy

Malicious overlay repositories could include symlinks pointing outside their directory tree (e.g., to /etc/passwd). The copy operation now checks each entry with symlink_metadata and rejects any symlink whose target resolves outside the source root. Also adds a recursion depth limit (64) to prevent stack overflow from circular symlinks.

##### Validate overlay repository URL scheme before cloning

Only https://, ssh://, and git@ URLs are now accepted for overlay repository sources. This prevents file:// and other local schemes from being used to read files from the host filesystem. URLs starting with `-` are also rejected to prevent git flag injection.

##### Validate overlay path components against directory traversal

The org, repo, and overlay name components used to construct filesystem paths are now validated to reject `..`, `/`, `\`, and leading `.` characters, preventing path traversal attacks via crafted overlay references.


## v0.6.0 - 2026-02-16


### Command: `apply`


#### Added

- Deep merge `.json` files during overlay application

  Resolve `.json` file conflicts automatically instead of failing. When `--merge` is enabled (via CLI flag or `REPOVERLAY_MERGE` env var), JSON files are recursively deep merged — objects merge key-by-key with overlay values winning, arrays and scalars use overlay values directly, and type mismatches are logged with full dotted key paths. Merged files are tracked with a new `Merged` link type in overlay state for correct cleanup on remove/update.

#### Fixed

- Reload overlay targets after removing a conflicting overlay during re-apply

  When `--force` removed an existing overlay to make room for a re-apply, the in-memory target set was stale, causing subsequent conflict checks in the same batch to see phantom conflicts. The target set is now reloaded after each forced removal.

### Command: `edit`


#### Added

- Add `edit` command for modifying existing overlays

  New `edit` command with `--add`, `--remove`, and `--interactive` flags for modifying applied overlays. `--add` adds files to an overlay, `--remove` removes specific files without removing the whole overlay, and `--interactive` re-runs the file selection UI with currently-applied files pre-selected. The existing `add` command is deprecated in favor of `edit --add`.

#### Changed

- `edit` defaults to interactive file selection when no flags given

  Running `repoverlay edit <name>` without `--add`, `--remove`, or `--interactive` in an interactive terminal now launches interactive file re-selection automatically. Non-interactive environments are unaffected.

### Command: `remove`


#### Changed

- `remove` defaults to interactive selection when no arguments given

  Running `repoverlay remove` in an interactive terminal now launches the interactive overlay selection instead of printing a usage error. Non-interactive environments (CI, piped stdin, TERM=dumb) are unaffected.




#### Security

- Reject git refs starting with `-` to prevent flag injection

  Source URLs with refs like `--upload-pack=evil` could be passed through to git commands as flags. `GitRef::from_str` now rejects any ref beginning with `-` and `with_ref_override` propagates the error instead of panicking via `.unwrap()`.
- Validate overlay mapping destinations against path traversal

  Overlay configs with mappings like `secret.txt = ../etc/passwd` could write files outside the target repository. Mapping destinations are now canonicalized and verified to stay within the target directory before any files are copied. Symlinks in overlay sources are also skipped to prevent symlink-based escapes.

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

