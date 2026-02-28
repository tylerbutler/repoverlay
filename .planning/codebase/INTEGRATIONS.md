# External Integrations

**Analysis Date:** 2026-02-27

## APIs & External Services

**GitHub Repository Access:**
- Service: GitHub (HTTPS only for cloning)
- What it's used for: Cloning overlay repositories, parsing GitHub URLs for source resolution
- SDK/Client: Native `git clone` invocation via Command::new("git")
- Auth: Git SSH key or HTTPS credentials (handled by git client, not repoverlay)
- Implementation: `src/github.rs` - GitHubSource struct for URL parsing and validation
- Features:
  - URL parsing: Supports github.com/owner/repo, with branch/tag/commit refs
  - Support for repository subpaths: github.com/owner/repo/tree/ref/path/to/overlay
  - Commit SHA detection (40 hex characters) vs branch/tag names
  - Cache key generation for local repo storage
  - Security: Rejects git refs starting with `-` (flag injection prevention)

**Upstream Repository Detection:**
- Service: GitHub (via git remotes)
- What it's used for: Automatic overlay filtering for forked repositories
- Implementation: `src/upstream.rs` - detect_upstream() and detect_repo_identity()
- Mechanism:
  - Reads git remote "upstream" URL
  - Reads git remote "origin" URL
  - Parses both for org/repo pairs
  - Case-insensitive matching for overlay targeting

**Version Check Service:**
- Service: crates.io API (via tiny-update-check library)
- What it's used for: Notifying users when new versions are available
- Caching: 24-hour local cache
- Implementation: `src/cli.rs` - check_for_updates() function
- Output: Terminal message with link to GitHub releases page

## Data Storage

**Databases:**
- None - repoverlay is stateless regarding external databases

**File Storage:**
- Local filesystem only
- Cache location: XDG_CACHE_HOME (via directories crate) or ~/.cache/repoverlay
- State location: .repoverlay/ directory in target repository
- Files stored:
  - `.repoverlay/meta.json` - Overlay metadata and track record
  - `.repoverlay/overlays/*.json` - Per-overlay state and file entries
  - `.repoverlay/config` - Repository configuration

**Caching:**
- Git repositories cloned to: `~/.cache/repoverlay/` (owner__repo__ref format)
- Cache manager: `src/cache.rs` - CacheManager struct
- Cleanup: Cache entries retain full history; no automatic eviction
- Lookups: Cached repos reused across invocations via identity detection

## Git Integration

**Git Operations:**
- Clone: `git clone --branch <ref>` for fetching overlay sources
- Remote inspection: `git remote get-url <name>` for upstream/origin detection
- No git library dependency (shell invocation via Command)
- Ref resolution: Automatic (git handles branches, tags, commits)

**Symlink vs Copy Behavior:**
- Default on Unix: Symlinks (overlay/target file relationships preserved)
- Default on Windows: Copy (no symlink permission issues)
- Override: `--force-copy` flag for explicit copy mode
- Relative symlinks: src/overlay_repo.rs - symlink_recursive() with path canonicalization

## Configuration & State

**State Files:**
- Format: JSON (serde_json)
- Location: `.repoverlay/` directory in target repository
- Root state: `.repoverlay/meta.json` - GlobalMeta struct
  - Sections: managed overlays (MANAGED_SECTION_NAME = "repoverlay")
  - Markers: exclude_marker_start/end for manual override protection
- Per-overlay state: `.repoverlay/overlays/{name}.json` - OverlayState struct
  - Tracks: file entries, entry types (link/copy), link types (relative/absolute)
  - JSON merge tracking for merged files

**Configuration Format:**
- CCL (Colon Config Language) - Custom text format (not TOML/YAML)
- File: `.repoverlay/config` in target repository
- Parsing: sickle crate (0.1.2 with serde feature)

## Authentication & Identity

**Auth Provider:**
- None - repoverlay delegates to git client for HTTPS/SSH authentication
- User provides: Git credentials via standard git config

**Repository Identity:**
- Determined from git remotes (origin, upstream)
- Used for auto-filtering overlays to matching repositories
- Case-insensitive matching
- No external identity service used

## Monitoring & Observability

**Error Tracking:**
- None - No external service integration

**Logs:**
- Output: stderr via env_logger
- Configuration: RUST_LOG environment variable
- Levels: trace, debug, info, warn, error
- No external log aggregation

**Debugging:**
- RUST_BACKTRACE=1 for detailed panic traces
- Verbose logging available via RUST_LOG=repoverlay=debug or =trace
- No telemetry collection

## CI/CD & Deployment

**Hosting:**
- GitHub (repository and releases)
- Homebrew tap: tylerbutler/homebrew-tap (formula auto-generation)

**CI Pipeline:**
- GitHub Actions (github.com/tylerbutler/repoverlay/.github/workflows)
- Workflows:
  - ci.yml: Test, lint, format, documentation checks (on push to main, PRs)
  - release.yml: Distribution build and GitHub release creation (on version tags)
  - coverage.yml: Code coverage reporting (on push to main)
  - audit.yml: Security audits (on push to main)
  - pr.yml: Pull request checks
  - dependabot.yml: Automated dependency updates

**Distribution:**
- cargo-dist - Multi-platform binary building and packaging
- Targets: Linux (x86_64, aarch64), macOS (universal), Windows (x86_64)
- Artifact types: .tar.gz, .zip, .msi installers, Homebrew formula
- Publishing: Direct to GitHub Releases, Homebrew tap via git push

**Version Control:**
- Automation: changie for changelog batch processing
- Release branches: None (direct tag-based releases)
- Prerelease handling: Version suffix detection (e.g., v1.0.0-beta.1)

## Webhooks & Callbacks

**Incoming:**
- None - repoverlay is a CLI tool with no server component

**Outgoing:**
- GitHub API: Release creation via gh CLI (GITHUB_TOKEN in CI)
- Homebrew tap: git push to tylerbutler/homebrew-tap repository

## Environment Configuration

**Required env vars:**
- None - repoverlay has no mandatory environment variables

**Optional env vars:**
- RUST_LOG - Control logging verbosity (default: off)
- RUST_BACKTRACE - Enable panic backtraces (1 for short, full for long)
- REPOVERLAY_CI_BUILD - Set by CI to use simpler version string
- XDG_CACHE_HOME - Override default cache directory (standard XDG)
- XDG_CONFIG_HOME - Override default config directory (standard XDG)
- HOME - Used for ~/ expansion
- GIT_* - Standard git environment variables (honored by git client)

**Secrets location:**
- Git credentials: Managed by git (SSH keys, HTTPS credentials)
- No secrets stored by repoverlay itself
- Homebrew publishing: HOMEBREW_TAP_TOKEN secret (GitHub Actions, tylerbutler/homebrew-tap repo)

## External Command Invocations

**System Commands:**
- `git clone` - Fetch overlay repositories
- `git remote get-url` - Discover upstream/origin
- `git checkout` - Switch to specific refs/branches

---

*Integration audit: 2026-02-27*
