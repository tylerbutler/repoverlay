# Codebase Concerns

**Analysis Date:** 2026-02-27

## Tech Debt

**Absolute vs. Relative Symlinks:**
- Issue: Symlinks created in `src/lib.rs` use absolute paths to source files. On Unix systems, symlinks created with `std::os::unix::fs::symlink(&source_file, &target_file)` use absolute path arguments, making symlinks non-portable across different machines or after directory moves.
- Files: `src/lib.rs` (lines 1231, 1463)
- Impact: Symlinks break when overlay source directory is moved or when repository is accessed from different systems/mounts. Cross-machine portability is compromised.
- Fix approach: Convert to relative symlinks by computing the relative path from target to source before creating the symlink. Consider using `pathdiff` crate or custom relative path computation.

**Windows Symlink Elevation Requirements Not Documented:**
- Issue: Windows symlinks created via `std::os::windows::fs::symlink_file` and `symlink_dir` require Administrator privileges or Developer Mode enabled on Windows 10+. Failure modes not explicitly documented.
- Files: `src/lib.rs` (lines 1238, 1467)
- Impact: Users on Windows without Admin/Developer Mode will get cryptic filesystem errors. Current code provides no guidance or fallback.
- Fix approach: Add explicit Windows symlink permission check with helpful error message. Document requirement in CLI help. Consider automatic fallback to copy mode on Windows when symlink fails.

**Large Monolithic Functions:**
- Issue: `apply_overlay_internal` in `src/lib.rs` is ~350 lines with complex nested conditionals and multiple responsibilities (validation, conflict resolution, file application, state management).
- Files: `src/lib.rs` (lines ~1030-1535)
- Impact: Difficult to test individual code paths, high cognitive complexity, risky to modify. Merge logic embedded deep within apply logic.
- Fix approach: Extract conflict resolution logic, JSON merge logic, and state management into separate functions. Create a smaller main flow function that orchestrates these pieces.

**Potential Path Traversal via Symlink Following:**
- Issue: In `src/lib.rs` line 1288, path traversal validation checks `normalized.starts_with(target)` but does not account for symlink chains. A malicious overlay could create a symlink that points outside the repository, and the validation only checks the logical path, not the resolved path.
- Files: `src/lib.rs` (lines 1280-1318)
- Impact: An overlay with a symlink target outside the repo could create unintended files outside the target directory (low risk in practice since overlays are typically trusted, but defense in depth is missing).
- Fix approach: After symlink creation, canonicalize the final target path and verify it's within the repository. Use `fs::read_link()` and resolve chains.

## Performance Bottlenecks

**Recursive Directory Copy Without Streaming:**
- Issue: `copy_dir_recursive` in `src/overlay_repo.rs` clones files via `fs::copy()` which loads entire file into memory for large files. No streaming or progress indication for large overlays.
- Files: `src/overlay_repo.rs` (referenced from `src/lib.rs` line 43)
- Current capacity: Works fine for typical overlays (<100MB), becomes slow for multi-GB overlays.
- Limit: No streaming means potential memory spikes on systems with limited RAM when copying large files.
- Improvement path: Implement chunked/streaming copy for files >10MB. Add progress callback support.

**Full Git History in Cache Clones:**
- Issue: Cache cloning in `src/cache.rs` and `src/overlay_repo.rs` uses `--depth 1` for shallow clones, but repeated updates still fetch from remote. Each cache update re-establishes git connection.
- Files: `src/cache.rs` (line ~114), `src/overlay_repo.rs` (line ~147)
- Current capacity: Works for <10 sources, becomes network-bound with 20+ sources or on slow connections.
- Limit: Multiple sources with stale caches cause N requests on every update. No connection pooling or batch operations.
- Improvement path: Implement batch cloning for multi-source initialization. Consider persistent git daemon or credential caching. Add cache prewarming command.

**JSON File Merge Without Streaming:**
- Issue: `merge_json_files` in `src/json_merge.rs` loads entire JSON files into memory via `serde_json::from_str()`. For large JSON files (>10MB), this causes memory spikes.
- Files: `src/json_merge.rs` (lines 87-91)
- Current capacity: Works fine for typical config files (<1MB).
- Limit: Breaks with large JSON files (package-lock.json, etc.).
- Improvement path: Implement streaming JSON parser for large files. Consider jq-like approach for selective merging.

## Fragile Areas

**Interactive Selection UI Terminal State Management:**
- Files: `src/selection.rs` (entire module, ~3300 lines)
- Why fragile: Uses raw `crossterm` terminal handling with manual screen clearing, cursor positioning, and mode switching. Edge cases in terminal state recovery (especially on Ctrl+C or exceptions) could leave terminal in unusable state.
- Safe modification: All terminal operations should be wrapped in RAII guard that restores terminal state on drop. Test on multiple terminal emulators (VTE, iTerm2, Windows Terminal, etc.).
- Test coverage: Selection module has unit tests but no terminal interaction tests. Missing: Ctrl+C recovery, window resize during selection, non-interactive terminal scenarios.

**GitHub URL Parsing with Complex Heuristics:**
- Files: `src/github.rs` (lines 43-94)
- Why fragile: Git ref parsing uses 40-char hex length to distinguish commits from branches. Refs matching exactly 40 hex chars (valid branch names theoretically possible, though rare) will be misclassified as commits.
- Safe modification: Add explicit ref type parameter to GitHubSource to remove ambiguity. Document assumption in comments.
- Test coverage: Good unit tests exist, but missing edge case: valid 40-char hex branch names would fail.

**Overlay Repository Directory Traversal Checks:**
- Files: `src/overlay_repo.rs` (lines 23-31)
- Why fragile: `validate_path_component` rejects `.` and `..` as strings but does not account for Unicode normalization. A path like `"."` (regular period) vs `"．"` (full-width period) would bypass validation on case-sensitive filesystems.
- Safe modification: Use filesystem path validation libraries (e.g., `path-normalization` crate). Reject any path containing non-ASCII characters for safety.
- Test coverage: Missing: Unicode in org/repo/overlay names, case sensitivity edge cases on HFS+.

**State File Serialization Format:**
- Files: `src/state.rs` (entire module)
- Why fragile: Uses CCL format for state files (`.repoverlay/meta.ccl`, etc.). Format is custom and not standard (TOML, JSON, YAML). Loss of parsing capability would prevent users from recovering overlays.
- Safe modification: Consider dual-format support (CCL + JSON) or migration path. Document CCL format as implementation detail, not public API.
- Test coverage: State serialization has unit tests but no migration/upgrade path tests for format changes.

**Configuration File Merging Without Validation:**
- Files: `src/json_merge.rs` (lines 26-65)
- Why fragile: Deep merge silently overwrites scalar values. Type mismatches (e.g., `"foo": "string"` → `"foo": 123`) are logged but merge continues. No rollback mechanism if merge produces invalid config.
- Safe modification: Validate merged result against schema (if available). Provide merge preview before applying. Add confirmation step for destructive merges.
- Test coverage: Good merge unit tests but missing: schema validation after merge, invalid JSON recovery, partial merge rollback.

## Security Considerations

**Git Flag Injection in Refs:**
- Risk: Refs starting with `-` could be interpreted as git flags. Mitigation exists in `src/github.rs` line 154 (`if s.starts_with('-')`), but the validation only happens in `GitRef::from_str()`, not in all code paths that accept ref strings.
- Files: `src/github.rs` (lines 153-158), `src/cache.rs` (cache checkout operations)
- Current mitigation: `GitRef::from_str()` rejects flag-like refs. This covers most code paths.
- Recommendations: Add comprehensive validation at cache.rs git command boundaries. Add integration tests that verify flag injection is blocked in all code paths.

**URL Scheme Validation in Overlay Repository:**
- Risk: `OverlayRepoManager::validate_clone_url()` in `src/overlay_repo.rs` (lines 114-135) rejects `file://` URLs to prevent local file access. However, validation is case-insensitive on scheme but the check happens after URL lowercasing, which is correct.
- Files: `src/overlay_repo.rs` (lines 114-135)
- Current mitigation: Allowlist approach (HTTPS, SSH only). Strong defense.
- Recommendations: None - implementation is solid. Consider documenting the security rationale in code comments.

**Cache Directory Permissions:**
- Risk: Cache stored in `~/.cache/repoverlay/` or `~/AppData/Local/repoverlay/` on Windows. No explicit permission checks ensure the cache is not world-readable (may contain credentials in future).
- Files: `src/cache.rs`, `src/overlay_repo.rs`
- Current mitigation: Relies on OS default cache directory permissions.
- Recommendations: After cloning or updating cache, explicitly set directory permissions to 0o700 (Unix) or verify ACLs are restrictive (Windows).

**Unvalidated Config File Inclusion:**
- Risk: Config files (`repoverlay.ccl`) are parsed from user-provided overlays without schema validation. Malicious config could cause unexpected behavior.
- Files: `src/config.rs` (config parsing)
- Current mitigation: Config is only used for merge behavior (merge-only flag), impact is limited.
- Recommendations: Add JSON schema validation for config files. Document trusted overlay sources guidance.

## Scaling Limits

**Cache Directory Size Growth:**
- Current capacity: Unlimited cache growth for multiple GitHub repos (each repo cloned fully, even with `--depth 1`).
- Limit: Filesystem space exhaustion possible with many large repos. No cache eviction policy.
- Scaling path: Implement LRU cache eviction. Add `cache prune` command with size limits. Monitor cache directory size.

**Interactive Selection with Large File Lists:**
- Current capacity: Selection UI tested with small lists (<100 files), may become slow with >1000 files in an overlay.
- Limit: Full list rendering on every screen update becomes O(n) for large n.
- Scaling path: Implement virtual scrolling or pagination in selection UI. Add search/filter to reduce visible set.

**External State Backup Storage:**
- Current capacity: External state stored per overlay in `~/.local/share/repoverlay/states/`. No cleanup policy.
- Limit: Could accumulate hundreds of MB with many old overlays applied.
- Scaling path: Implement automatic cleanup of external states older than N days. Add `state cleanup` command.

## Test Coverage Gaps

**Symlink Creation on Windows:**
- What's not tested: Windows symlink elevation failure scenarios. Current tests don't verify behavior when elevation is missing.
- Files: `src/lib.rs` (symlink creation paths), `tests/cli.rs`
- Risk: Users on Windows without Admin will encounter untested error paths.
- Priority: High - affects primary Windows user experience.

**Path Traversal Attack Vectors:**
- What's not tested: Overlays with symlinks pointing outside target repo. Overlays with `../../../etc/passwd` type mappings in config.
- Files: `src/lib.rs` (path validation logic)
- Risk: Could allow unintended file placement (low severity in practice due to overlay trust model, but defense in depth is missing).
- Priority: Medium - thorough validation exists but edge cases untested.

**Interactive Conflict Resolution - Ctrl+C Recovery:**
- What's not tested: Terminal state after user interruption during conflict prompt.
- Files: `src/lib.rs` (lines 126-150: prompt_conflict_interactive)
- Risk: Terminal left in raw mode, corrupting user's shell session.
- Priority: High - affects interactive mode user experience.

**JSON Merge with Edge Cases:**
- What's not tested: Very large JSON files (>100MB), circular references in objects, null value merging behavior consistency.
- Files: `src/json_merge.rs`
- Risk: Merge produces invalid JSON or crashes on large files.
- Priority: Medium - good unit tests exist, but edge cases missing.

**Multi-Source Overlay Resolution with Upstream Fallback:**
- What's not tested: Complex upstream fallback scenarios (fork of fork), conflicting overlays in different sources with same name.
- Files: `src/sources.rs`, `src/upstream.rs`
- Risk: Subtle bugs in resolution priority logic under complex repo structures.
- Priority: Medium - unit tests cover basic cases but complex scenarios need integration tests.

**Cache Update Failure Recovery:**
- What's not tested: What happens when cache update fails halfway (network error during git pull). Does `.repoverlay-overlay-repo-meta.ccl` get updated? Can recovery continue?
- Files: `src/cache.rs`, `src/overlay_repo.rs`
- Risk: Corrupted cache metadata could prevent future updates.
- Priority: Medium - error handling exists but consistency under partial failure untested.

## Dependencies at Risk

**outdated command-line dependencies:**
- `clap` (parsing): Heavy dependency, but well-maintained and widely used.
- `serde_json` (JSON): Standard library, low risk.
- `dialoguer` (interactive prompts): Smaller maintained crate, some platform-specific code.
- `crossterm` (terminal UI): Complex terminal handling, cross-platform. Risk: terminal-specific bugs on lesser-tested terminals (old Windows Console, embedded terminals).
- Risk level: Low to Medium. All popular, maintained crates.
- Migration plan: For crossterm, consider ratatui or termwiz for more robust terminal handling if issues emerge.

**CCL parsing library (sickle):**
- Risk: Custom CCL format is non-standard. Loss of sickle crate or breaking changes would require state file migration.
- Impact: State files in `.repoverlay/` become unreadable if sickle breaks.
- Migration plan: Add dual-format support (CCL + JSON). Document state file format. Consider forking sickle if it becomes unmaintained.

---

*Concerns audit: 2026-02-27*
