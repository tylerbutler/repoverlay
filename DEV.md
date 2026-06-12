# Development Guide

## Prerequisites

- **Rust** 1.91+ (2024 edition) - https://rustup.rs/
- **just** - Task runner - https://github.com/casey/just
- **git** - Required at runtime for GitHub overlay functionality
- **mise** (optional) - `mise install` provisions the pinned dev tools (`cargo-cyclonedx`, `cargo-dist`, etc.) used by the supply-chain recipes - https://mise.jdx.dev/

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

This project uses a release pipeline with clear ownership boundaries: [changie](https://changie.dev/) owns changelog content, [release-plz](https://release-plz.ieni.dev/) owns crates.io publishing and release tags, and [cargo-dist](https://rust-lang.github.io/cargo-dist/) owns binary distribution and GitHub release hosting.

### Ownership Boundaries

- **changie** (`.changes/unreleased/`, `CHANGELOG.md`) owns changelog fragments and changelog aggregation. `.changie.yaml` defines the allowed kinds/components and renders `CHANGELOG.md`.
- **release-plz** (`release-plz.toml`, `.github/workflows/release-plz.yml`) owns crates.io publishing and `v<version>` tag creation after a release PR lands on `main`. It does **not** update `CHANGELOG.md` (`changelog_update = false`) or create GitHub releases (`git_release_enable = false`).
- **cargo-dist** (`dist-workspace.toml`, `.github/workflows/release.yml`) owns binary artifacts, shell/PowerShell/Homebrew installers, GitHub release hosting, the Homebrew publish job, and SLSA build-provenance attestation of the binaries (`github-attestations = true`).
- **SBOM** (`.github/workflows/release-sbom.yml`) owns generating a CycloneDX SBOM, attaching it to the GitHub release, and creating a signed SBOM attestation bound to the released archives.
- **Custom Homebrew tap workflow** (`.github/workflows/publish-homebrew-tap.yml`) publishes the generated formula to `tylerbutler/homebrew-tap` using a GitHub App token instead of a long-lived PAT.

### Release Preflight Checklist

Before merging a release PR, ensure:

1. Local checks pass:
   ```bash
   just check      # Runs format check, lint, and tests
   ```

2. The next changelog renders correctly:
   ```bash
   just changelog-preview    # Runs: changie batch auto --dry-run
   ```

3. The release PR contains the expected release files:
   - `CHANGELOG.md` includes the aggregated changie entries.
   - Release/version files are bumped consistently, including `Cargo.toml` and `Cargo.lock`.
   - Changelog entries are complete, user-facing, and grouped under the right kind/component.

### Release Flow

1. **Prepare changelog fragments** - User-facing changes should include a fragment created with:
   ```bash
   just change    # Runs: changie new
   ```

   Select one of the configured kinds (`Breaking`, `Added`, `Fixed`, `Performance`, `Changed`, `Reverted`, `Dependencies`, `Security`) and an appropriate component (`features`, `fixes`, `misc`, `library`, or a command name: `apply`, `browse`, `cache`, `create`, `edit`, `move`, `remove`, `restore`, `source`, `status`, `switch`, `sync`, `update`).

   No changelog fragment is needed for CI-only/release-plumbing documentation changes that do not affect the published binary.

2. **Create/update the release PR** - On every push to `main`, the changie release workflow (`.github/workflows/changie-release.yml`) checks for unreleased fragments in `.changes/unreleased/`. If found, it batches them into `CHANGELOG.md`, bumps `Cargo.toml`, and creates or updates a release PR. A subsequent step in the workflow then updates `Cargo.lock` to match the new version.

3. **Merge the release PR** - After the preflight checklist passes, merge the release PR to `main`. This triggers:
   - The changie release workflow, which runs again but skips if no unreleased fragments remain.
   - The release-plz workflow (`.github/workflows/release-plz.yml`), which detects the version change in `Cargo.toml`, publishes the crate to crates.io, and creates the `v<version>` git tag configured by `release-plz.toml`.

4. **Publish binaries and installers** - The `v<version>` tag created by release-plz triggers the cargo-dist release workflow (`.github/workflows/release.yml`, via the `**[0-9]+.[0-9]+.[0-9]+*` tag pattern). It builds the configured targets (`aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`), creates the GitHub release, uploads artifacts, attests SLSA build provenance for the binaries, publishes shell/PowerShell installers, and invokes the custom Homebrew publish job (`.github/workflows/publish-homebrew-tap.yml`).

5. **Generate and attest the SBOM** - Publishing the GitHub release fires the `release-sbom.yml` workflow (on `release: published`). It runs `just sbom` to produce a CycloneDX SBOM (`repoverlay.cdx.json`), uploads it as a release asset, and creates a signed SBOM attestation bound to the released archives via `actions/attest`.

### Supply Chain: SBOM and Attestations

Releases ship two kinds of [GitHub Artifact Attestations](https://docs.github.com/actions/security-guides/using-artifact-attestations-to-establish-provenance-for-builds), both signed via Sigstore:

- **Build provenance** for each binary archive, produced by cargo-dist (`github-attestations = true` in `dist-workspace.toml`, regenerated into `release.yml`).
- **SBOM attestation** binding the CycloneDX SBOM to the released archives, produced by `release-sbom.yml`.

The SBOM (`repoverlay.cdx.json`) is also attached to the GitHub release as a downloadable asset.

Local tooling (installed via `mise install`):

- `just sbom` - generate the CycloneDX SBOM locally (uses `cargo-cyclonedx`).
- `just dist-generate` / `just dist-plan` - regenerate and validate `release.yml` after editing `dist-workspace.toml` (uses the pinned `cargo-dist`).
- `just sbom-attest-verify <archive>` - verify attestations for a downloaded release archive:

```bash
gh release download v<version> --pattern 'repoverlay-*.tar.xz'
just sbom-attest-verify repoverlay-x86_64-unknown-linux-gnu.tar.xz
```

### Conventional Commit Types

release-plz follows conventional commits for release semantics:
- `fix:` - Patch version bump
- `feat:` - Minor version bump
- `feat!:` or `BREAKING CHANGE:` - Major version bump

### Required Secrets and Permissions

All release secrets are configured in GitHub Actions:

- **`CARGO_REGISTRY_TOKEN`** - crates.io API token used by release-plz to publish the crate.
- **`RELEASE_APP_ID`** - GitHub App ID used to mint installation tokens for release automation.
- **`RELEASE_APP_PRIVATE_KEY`** - GitHub App private key used with `RELEASE_APP_ID`.
- **`GITHUB_TOKEN`** - Automatically provided by GitHub Actions; cargo-dist uses it for artifact upload and GitHub release creation.

The GitHub App configured by `RELEASE_APP_ID`/`RELEASE_APP_PRIVATE_KEY` must be installed on both repositories it writes to:

- `tylerbutler/repoverlay` with contents/write access for release PR, tag, and release automation.
- `tylerbutler/homebrew-tap` with contents/write access so `.github/workflows/publish-homebrew-tap.yml` can update the Homebrew formula.

## Profiles Feature Status

*Snapshot: 2026-06-10.*

The full profiles + plugins implementation landed on `main` in #347 (squash of the `profiles` branch) and shipped in **v0.15.0**. All profile-related tests pass on `main` (46 tests via `cargo test profile`). The feature is **hidden**: the `profile`, `claude`, and `copilot` commands are declared with `hide = true` in `src/cli/mod.rs`, so they work but do not appear in `--help`. Only `marketplace` is visible.

### CLI surface

| Command | Visibility | Purpose |
| --- | --- | --- |
| `repoverlay profile list/show/apply/status/remove` | hidden | Persistent profile lifecycle (`apply`/`status`/`remove` take `--harness claude\|copilot`) |
| `repoverlay claude --profile X [--profile Y] [-- args]` | hidden | Run Claude with profiles applied for the process lifetime (ephemeral) |
| `repoverlay copilot --profile X [--profile Y] [-- args]` | hidden | Same for GitHub Copilot CLI |
| `repoverlay marketplace …` | visible | Manage the plugin marketplace registry |
| `repoverlay update` | visible | Also re-resolves managed profile plugins (full update only, not single-overlay) |

### What a profile contains and how each part maps to a harness

A profile (`ProfileConfig` in `src/profile.rs`) is defined under the `profiles` key of the CCL config (global `~/.config/repoverlay/config.ccl` or repo-local `.repoverlay/config.ccl`) and has four fields. Per-harness placement is owned by the applicators in `src/profile_applicators/` and the `AgentHarness` enum (single source of truth for harness paths).

| Profile content | Claude mapping | Copilot mapping |
| --- | --- | --- |
| `description` | None — display-only in `profile list`/`show` | Same |
| `overlays` (list of overlay refs) | `ApplyOverlay` through the regular overlay apply machinery; harness-independent | Same |
| `instructions` (each entry exactly one of `source` file path or inline `content`) | Concatenated into a marker-delimited managed region keyed by profile name in `<repo>/CLAUDE.md`, coexisting with user content and other profiles' regions | Same, targeting `<repo>/AGENTS.md` |
| `plugins` → bundle `skills/` | Copied to `<repo>/.claude/skills/<skill>` | Copied to `<repo>/.agents/skills/<skill>` |
| `plugins` → bundle `agents/` | Copied to `<repo>/.claude/agents/<agent>` | Copied to `<repo>/.github/agents/<agent>` |
| `plugins` → bundle `.mcp.json` `mcpServers` | Merged into `<repo>/.mcp.json` under `mcpServers`, `${CLAUDE_PLUGIN_ROOT}` substituted with the cached bundle dir; RFC 6901 per-pointer ownership for conflict detection and clean removal; two plugins providing the same server is an error | Same target and semantics (Copilot CLI also keys servers under `mcpServers`) |
| `plugins` → bundle `hooks/`, `commands/` | `SkipCapability` — not decomposable | `SkipCapability` — unsupported |
| `plugins` with `install = delegate` (or managed plugins whose source cannot be cached/introspected) | `enabledPlugins` (`"name@marketplace": true`) + `extraKnownMarketplaces` merged into `.claude/settings.json` (`scope = project`, default for persistent applies) or `.claude/settings.local.json` (`scope = local`, default for ephemeral); requires the marketplace to be registered with a URL | `SkipCapability` — delegate plugins are Claude-only |

Plugin references (`PluginRef` in `src/plugin.rs`) are either marketplace refs (`marketplace/plugin` shorthand, or a table with optional `ref` pin, `install = managed|delegate`, delegate `scope`) or local paths (starting with `.` or `/`). Bundles use the Claude plugin format (`.claude-plugin/plugin.json`).

Harness identity (`AgentHarness` in `src/profile_applicators/mod.rs`): stable ids `claude`/`copilot`; config homes `~/.claude` and `~/.config/github-copilot` (overridable via `REPOVERLAY_CLAUDE_HOME` / `REPOVERLAY_COPILOT_HOME`); launch programs `claude`/`copilot` (overridable via `REPOVERLAY_*_COMMAND`); removable JSON targets are `.mcp.json` plus, for Claude, `.claude/settings.json` and `.claude/settings.local.json`.

### Lifecycle guarantees

- Everything a profile applies lands inside the target repo's working tree, git-excluded — no user- or machine-global writes.
- Persistent applies are transactional (rollback on failure) and recorded under `<repo>/.repoverlay/profiles/`; `repoverlay restore` rebuilds profiles after `git clean` via external snapshots.
- Ephemeral sessions hold PID lock files with stale-lock recovery (survives `SIGKILL`/power loss), refuse to run over an already-persistent profile, and roll back all placements when the session exits or is interrupted.
- Plugin decomposition is identical for persistent and ephemeral applies; only the delegate settings scope default differs.

### Remaining work

- **Hooks and commands**: plugin bundle `hooks/` and `commands/` are never decomposed for either harness.
- **Un-hide the CLI**: `profile`, `claude`, and `copilot` are still hidden pending public announcement.
- **Unmerged docs on the `profiles` branch**: the website profiles guide (`website/src/content/docs/guides/profiles.md`), homepage profile announcements, and the design specs/plans (`docs/superpowers/specs/2026-06-02-profiles-design.md`, `2026-06-03-plugins-design.md`, plans, and the harness-process refactor docs) exist only on the branch. The branch's source code is otherwise fully merged — `main` is *ahead* of it (the branch lacks the cf4114f cross-platform symlink fix), so only the docs need to be brought over. The unmerged guide also predates Claude instruction placement (it documents instructions as Copilot-only), so it needs updating when brought over.

## Vendored Agent Skills

The repository tracks skills for AI coding agents working on this codebase: `.claude/skills/` (Claude Code) and `.agents/skills/` (other harnesses) hold vendored skill copies, and `skills-lock.json` pins each skill's upstream source and version. These are contributor tooling only — they are not part of the published binary and do not appear in user documentation.

## Project Structure

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed module structure and responsibilities.
