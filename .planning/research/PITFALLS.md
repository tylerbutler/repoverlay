# Pitfalls Research

**Domain:** Rust CLI tool stabilization and 1.0 release (repoverlay)
**Researched:** 2026-02-27
**Confidence:** HIGH

## Critical Pitfalls

### Pitfall 1: Debug Error Display Leaking to Users

**What goes wrong:**
The `main.rs` entry point uses `{e:?}` (Debug formatting) to display errors: `eprintln!("Error: {e:?}")`. This prints the full anyhow error chain with `Debug` formatting, which includes internal type names, raw OS error codes, and implementation details that are meaningless to end users. After 1.0, users expect polished, actionable error messages -- not debug dumps like `Error: No such file or directory (os error 2)` without context about which file or operation failed.

**Why it happens:**
During development, `{:?}` is convenient because it shows the full error chain for debugging. Developers never switch to user-facing formatting because the debug output is "good enough" during development and the distinction between `{e}` (Display) and `{e:?}` (Debug) is easy to overlook. The anyhow `.context()` chain is specifically designed to produce readable output via Display, but that only works if you actually use `{e}` or `{e:#}` (alternate Display with "Caused by:" chain).

**How to avoid:**
- Switch `main.rs` to use `{e:#}` for user-facing error display (anyhow's alternate Display shows the full context chain readably)
- Reserve `{e:?}` for `RUST_LOG=debug` or `--verbose` flag output
- Audit every `.context()` and `.with_context()` call to ensure messages are written for the user, not the developer (e.g., "Failed to apply overlay 'foo'" not "apply_overlay failed")
- Add an integration test that triggers a common error and asserts the output does not contain Rust type names or raw debug formatting

**Warning signs:**
- Error messages contain `Os { code: 2, kind: NotFound, message: "..." }` instead of plain English
- Users file issues saying "I got an error but I don't understand what it means"
- Error messages expose internal function names or module paths

**Phase to address:**
Error handling audit phase -- before any release candidate testing

---

### Pitfall 2: No SIGPIPE Handling Causes Broken Pipe Panics

**What goes wrong:**
Rust's standard library masks SIGPIPE by default. When repoverlay output is piped to a program that exits early (e.g., `repoverlay status | head -1`), the write to stdout fails with a "broken pipe" error. Instead of silently exiting (the Unix convention), repoverlay prints an error message and exits with code 1, which breaks shell pipelines and confuses users who expect standard Unix pipe behavior.

**Why it happens:**
This is a Rust-specific footgun that does not exist in C/Go/Python CLIs. The Rust runtime installs `SIG_IGN` for SIGPIPE at startup, which means broken pipe becomes an `io::Error` rather than a signal-based termination. Most Rust CLI developers are unaware of this because it only manifests when users pipe output, which is rare during development but common in production scripts.

**How to avoid:**
- Add `#[cfg(unix)] unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL); }` at the start of `main()`, or
- In the error handler in `main.rs`, detect `ErrorKind::BrokenPipe` and exit with code 0 silently
- Test with `repoverlay status | head -1` to verify clean exit
- The second approach (detect and swallow) is safer and does not require `unsafe`

**Warning signs:**
- `repoverlay status | head` prints "Error: Broken pipe" to stderr
- Shell scripts using repoverlay in pipelines fail unexpectedly
- Exit code is 1 when piped, 0 when run directly (for the same command)

**Phase to address:**
Code review / correctness fixes phase -- this is a one-line fix but must be caught before 1.0

---

### Pitfall 3: Windows Symlink Failures Without Actionable Guidance

**What goes wrong:**
On Windows, creating symlinks requires either Administrator privileges or Developer Mode enabled (Windows 10+). When neither is available, `std::os::windows::fs::symlink_file` fails with OS error 1314 ("A required privilege is not held by the client"). The current code propagates this as a raw OS error with no explanation of what the user needs to do to fix it. After 1.0, Windows users will encounter this as the first error they see when trying to use the tool.

**Why it happens:**
Symlink permissions are a Windows-specific concept with no Unix equivalent. Developers testing on Unix never encounter it, and CI often runs with elevated privileges. The platform-specific `#[cfg]` blocks for symlink creation were written for correctness but not for user experience. Additionally, the file-vs-directory symlink distinction on Windows (which does not exist on Unix) means even when privileges are available, the wrong symlink type can be created silently.

**How to avoid:**
- Before creating the first symlink on Windows, run a permission probe (create a test symlink in a temp dir, check for error 1314)
- On failure, print a clear message: "Symlink creation requires Developer Mode or Administrator privileges. Enable Developer Mode in Settings > Update & Security > For Developers, or use --copy to use file copies instead."
- Consider making `--copy` the default on Windows and symlinks opt-in via `--symlink`
- Add a Windows CI job that tests without elevation to verify the error path

**Warning signs:**
- Windows-only bug reports about cryptic OS errors during `apply`
- No Windows CI job in the test matrix (currently CI only runs on `ubuntu-latest`)
- Users working around symlink issues by always passing `--copy`

**Phase to address:**
Cross-platform testing and bug fix phase

---

### Pitfall 4: State File Format Lock-in Without Migration Path

**What goes wrong:**
State files (`.repoverlay/overlays/<name>.ccl`, `~/.local/share/repoverlay/applied/<hash>.ccl`) use the CCL format via the `sickle` crate and include a `GlobalMeta { version: 1 }` marker but no actual migration logic. If the state format needs to change after 1.0 (new fields, restructured data, renamed keys), there is no mechanism to read old-format files and upgrade them. Users who upgrade from 1.0 to 1.1 will find their existing overlays unreadable if the format changes, requiring manual cleanup of `.repoverlay/` directories across all their repositories.

**Why it happens:**
Version fields get added "for future use" but the migration code is never written because there is only one version. The CCL format (via `sickle`) is non-standard, which means there are no ecosystem tools for format migration. The `#[serde(default)]` annotation on `files: Vec<FileEntry>` provides minimal forward compatibility but cannot handle structural changes or renamed fields. The external state backup in `~/.local/share/repoverlay/` compounds the problem because users may have dozens of state files spread across different repos.

**How to avoid:**
- Before 1.0, freeze the state file schema and document it as part of the stability contract
- Implement a version-checking reader that rejects files with `version > 1` with a clear "please upgrade repoverlay" message
- Plan for a `repoverlay migrate` command (can be a no-op in 1.0 but the plumbing should exist)
- Write a test that serializes state with the current version, then attempts to deserialize it with the current reader -- this becomes a regression test for format compatibility
- Add `#[serde(deny_unknown_fields)]` on critical structs so that extra fields from future versions cause explicit errors rather than silent data loss

**Warning signs:**
- State structs use `#[serde(default)]` extensively without documenting why
- No tests verify round-trip serialization stability
- The `version` field in `GlobalMeta` is never actually checked during deserialization
- Adding a new field to `OverlayState` silently breaks old state files

**Phase to address:**
State format audit and test coverage phase -- must be completed before the 1.0 tag

---

### Pitfall 5: CI Tests Only on Linux While Shipping to Three Platforms

**What goes wrong:**
The CI pipeline (`ci.yml`) runs tests exclusively on `ubuntu-latest`, but `cargo-dist` builds and ships binaries for five targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`, and `x86_64-pc-windows-msvc`. Platform-specific bugs in symlink creation, path handling (forward slash vs backslash, UNC paths), git exclude file line endings (LF vs CRLF), and cache directory resolution (`dirs` crate behavior differences) ship to users untested. After 1.0, platform bugs become semver-breaking fixes.

**Why it happens:**
Cross-platform CI is expensive (macOS runners cost 10x Linux) and slow. During rapid development, the friction of maintaining a multi-platform matrix is not worth the cost. But a 1.0 release with binary distribution creates an implicit contract that the tool works on all distributed platforms.

**How to avoid:**
- Add at minimum a `windows-latest` and `macos-latest` job to the CI test matrix
- Gate symlink-specific tests on Windows with `#[cfg(windows)]` tests that specifically exercise the Windows code paths
- Use `cfg_attr` to mark Windows-specific tests as `#[ignore]` on non-Windows, so they run only in the platform-appropriate CI job
- Test path handling with mixed separators (e.g., `foo/bar` vs `foo\bar`)
- Verify `.git/info/exclude` line ending handling on Windows (Git on Windows may use CRLF)

**Warning signs:**
- All `#[cfg(windows)]` code paths have zero test coverage
- CI matrix has only one OS
- Platform-specific bug reports arrive immediately after a release

**Phase to address:**
CI and cross-platform testing phase -- must be in place before release candidate

---

### Pitfall 6: Releasing 1.0 With a Non-Standard Configuration Format Dependency

**What goes wrong:**
Repoverlay depends on `sickle` (version 0.1.2) for CCL format parsing of both configuration files (`repoverlay.ccl`) and state files. The `sickle` crate is at version 0.1.x, indicating it has not reached its own stability commitment. If `sickle` introduces breaking changes, stops being maintained, or has parsing bugs, repoverlay's core functionality (reading/writing state) breaks. Users cannot work around this because the state file format is an implementation detail they cannot control.

**Why it happens:**
CCL was chosen as the configuration format for domain-specific reasons, and `sickle` is the only parser. During development, tight coupling to an unstable dependency is fine. But a 1.0 release with a 0.1.x dependency for critical-path functionality creates a fragile foundation.

**How to avoid:**
- Pin `sickle` to an exact version (`=0.1.2`) in `Cargo.toml` to prevent accidental upgrades
- Write comprehensive round-trip tests for every state struct: serialize, deserialize, verify equality
- Consider vendoring the `sickle` crate or forking it if it has limited maintenance
- Document the CCL format for state files so users could manually repair files if needed
- Evaluate whether state files could use a standard format (JSON, TOML) as a backup or migration target

**Warning signs:**
- `sickle` has infrequent commits or no recent releases
- `cargo update` bumps `sickle` and breaks state file parsing
- No round-trip serialization tests for CCL state files
- Users cannot inspect or manually edit state files

**Phase to address:**
Dependency audit phase -- evaluate before 1.0 release

---

## Technical Debt Patterns

Shortcuts that seem reasonable but create long-term problems.

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| Using `{e:?}` for error display | Shows full debug chain during dev | Exposes internals to users, unprofessional error output | Never in release builds |
| Single exit code (always 1) | Simple error handling | Scripts cannot distinguish "overlay not found" from "permission denied" from "network error" | Pre-1.0 only; 1.0 should have at least 2-3 distinct codes |
| No state file migration logic | Less code to maintain | Format changes after 1.0 require breaking changes or complex retrofitting | Acceptable if schema is frozen and documented |
| Absolute symlinks | Simpler implementation (no relative path computation) | Symlinks break when source directory is moved, not portable across machines | Never for a tool managing shared configurations |
| `anyhow` for all errors without `thiserror` categories | Faster development, less boilerplate | Cannot programmatically distinguish error types for retry logic or specific exit codes | Pre-1.0 only if you plan distinct exit codes |
| Monolithic `apply_overlay_internal` (~350 lines) | Keeps all apply logic in one place | Untestable sub-paths, high cognitive load, merge bugs hide in nested conditionals | Only acceptable if covered by extensive integration tests |

## Integration Gotchas

Common mistakes when connecting to external services.

| Integration | Common Mistake | Correct Approach |
|-------------|----------------|------------------|
| GitHub API / git ls-remote | Not handling rate limiting (unauthenticated requests limited to 60/hour) | Detect 403/rate limit responses, provide clear message suggesting auth token configuration |
| Git subprocess execution | Assuming `git` is on PATH and is a recent version | Check for git availability at startup, verify minimum version for features used (e.g., `--depth 1` requires git 1.9+) |
| Cache directory (`dirs` crate) | Assuming cache dir always exists and is writable | Create cache directory with explicit permissions, handle `None` return from `dirs::cache_dir()` on unusual systems |
| Git `.git/info/exclude` | Assuming the file exists and has specific format | Create the file and parent directories if missing, handle both LF and CRLF line endings, handle concurrent modifications |
| Homebrew tap publishing | Publishing formula before binaries are uploaded | Ensure cargo-dist release workflow completes before Homebrew formula references the binary URLs |

## Performance Traps

Patterns that work at small scale but fail as usage grows.

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| Loading all state files to check status | `repoverlay status` is slow | Index applied overlays in a summary file or use lazy loading | >50 applied overlays across multiple repos |
| Full directory walk for every apply | Noticeable delay on large overlays | Cache directory listings during a single command execution | Overlays with >1000 files |
| N sequential git ls-remote calls during update | Update command takes minutes on slow connections | Batch remote checks or parallelize with `tokio`/`rayon` | >10 GitHub-sourced overlays |
| Interactive selection rendering entire list | UI becomes sluggish | Virtual scrolling or pagination | >500 files in detection results |

## Security Mistakes

Domain-specific security issues for a tool that creates symlinks and executes git commands.

| Mistake | Risk | Prevention |
|---------|------|------------|
| Path traversal via overlay contents | Overlay with `../../.ssh/authorized_keys` mapping could write outside repo | Already partially mitigated via `starts_with(target)` check, but must also canonicalize after symlink creation to catch symlink-chain escapes |
| Git flag injection via crafted refs | A ref like `--upload-pack=malicious` could inject git flags | `GitRef::from_str()` rejects `-`-prefixed refs, but verify all code paths that pass refs to git commands go through this validation |
| Cache directory permissions | `~/.cache/repoverlay/` may contain cloned repos with sensitive content | Set `0o700` permissions on cache directory after creation on Unix |
| Overlay repo SSRF via redirect | A clone URL could redirect to an internal service | The allowlist approach (HTTPS/SSH only, no `file://`) is solid; document this as a security property |

## UX Pitfalls

Common user experience mistakes in CLI stabilization.

| Pitfall | User Impact | Better Approach |
|---------|-------------|-----------------|
| No `--dry-run` on destructive operations | Users cannot preview what `apply --force` will overwrite | Add `--dry-run` to `apply`, `remove`, and `switch` commands |
| Silent success with no output | Users unsure if command did anything | Print a summary line on success: "Applied overlay 'foo' (3 files)" |
| Cryptic overlay name normalization | User types `My Overlay!` and it becomes `my-overlay-` silently | Show the normalized name and confirm if it differs significantly from input |
| Version update check on every run | Slows down every invocation, annoying in scripts | Only check periodically (once per day), respect `NO_UPDATE_CHECK` env var, never check when stdout is not a TTY |
| No shell completion documentation | Users don't know completions are available | Print a hint on first run or in `--help` output |

## "Looks Done But Isn't" Checklist

Things that appear complete but are missing critical pieces.

- [ ] **Error messages:** Often missing `.context()` on file I/O operations -- verify every `fs::read`, `fs::write`, `fs::symlink` has context about which file and why
- [ ] **Cross-platform symlinks:** Often missing the file-vs-directory distinction on Windows -- verify `symlink_file` vs `symlink_dir` is called correctly based on entry type
- [ ] **State persistence:** Often missing atomic write (write to temp then rename) -- verify a crash mid-write does not corrupt the state file
- [ ] **Git exclude management:** Often missing concurrent access handling -- verify two simultaneous `repoverlay apply` calls do not corrupt `.git/info/exclude`
- [ ] **Cache cleanup:** Often missing eviction policy -- verify cache does not grow unbounded over months of use
- [ ] **Release binaries:** Often missing the `strip = true` and `lto = true` settings -- already configured in `Cargo.toml`, verify binary sizes are reasonable
- [ ] **Changelog:** Often missing entries for bug fixes discovered during stabilization -- verify changie has entries for every fix
- [ ] **crates.io metadata:** Often missing `categories`, `keywords`, `repository`, `homepage` -- already configured in `Cargo.toml`, verify they render correctly on crates.io

## Recovery Strategies

When pitfalls occur despite prevention, how to recover.

| Pitfall | Recovery Cost | Recovery Steps |
|---------|---------------|----------------|
| State file format incompatibility after upgrade | MEDIUM | Add `repoverlay migrate` command that reads old format and writes new format; provide manual instructions for users who cannot upgrade immediately |
| Broken symlinks after source directory move | LOW | `repoverlay remove <name>` + `repoverlay apply <new-source>` cycle; document in FAQ |
| Corrupted `.git/info/exclude` | LOW | `repoverlay remove --all` cleans up sections; worst case, user manually edits the file |
| Windows symlink permission failure mid-apply | MEDIUM | `repoverlay remove <name>` to clean partial state; add rollback logic that undoes partial applies on failure |
| Broken pipe error in scripts | LOW | Fix SIGPIPE handling; no user data at risk |
| crates.io publish with bug | HIGH | crates.io publishes are permanent; must release a patch version immediately; cannot yank and re-publish same version |

## Pitfall-to-Phase Mapping

How roadmap phases should address these pitfalls.

| Pitfall | Prevention Phase | Verification |
|---------|------------------|--------------|
| Debug error display | Error handling audit | Integration test asserts no `Os {` patterns in stderr output |
| No SIGPIPE handling | Code review / correctness fixes | Test `repoverlay status \| head -1` exits with code 0 |
| Windows symlink failures | Cross-platform testing | Windows CI job with non-elevated test runner |
| State file format lock-in | State format audit | Round-trip serialization tests for all state structs |
| Linux-only CI | CI infrastructure | Test matrix includes ubuntu, macos, windows |
| sickle dependency risk | Dependency audit | Pin exact version, add serialization round-trip tests |
| Monolithic apply function | Code review | Unit tests for extracted sub-functions (conflict resolution, merge logic) |
| Absolute symlinks | Code review / correctness fixes | Test that created symlinks are relative, not absolute |
| Single exit code | Error handling audit | Different exit codes for "not found" vs "permission error" vs "conflict" |
| No dry-run on destructive ops | UX review | `--dry-run` flag exists and produces accurate preview output |
| Missing .context() on I/O | Error handling audit | Grep for bare `?` on `fs::` calls without `.context()` |
| No atomic state writes | Correctness review | Kill test (interrupt during write) does not leave corrupt state file |

## Sources

- [Cargo Book: SemVer Compatibility](https://doc.rust-lang.org/cargo/reference/semver.html) - semver rules for Rust crates
- [Effective Rust - Item 21: Semantic Versioning](https://effective-rust.com/semver.html) - pitfalls around 1.0 commitment
- [FOSDEM 2024: SemVer in Rust: Tooling, Breakage, and Edge Cases](https://predr.ag/blog/semver-in-rust-tooling-breakage-and-edge-cases/) - 1 in 6 top crates violates semver
- [Rust CLI Book: Exit Codes](https://rust-cli.github.io/book/in-depth/exit-code.html) - exit code conventions
- [Rust CLI Book: Testing](https://rust-cli.github.io/book/tutorial/testing.html) - integration test patterns
- [Rust CLI Book: Error Handling](https://rust-cli.github.io/book/tutorial/errors.html) - anyhow context patterns
- [rust-lang/rust #38921](https://github.com/rust-lang/rust/pull/38921) - Windows unprivileged symlink support
- [rust-cli/team #10](https://github.com/rust-cli/team/issues/10) - Cross-platform filesystem abstractions
- [rust-lang/cargo #5664](https://github.com/rust-lang/cargo/issues/5664) - symlink issues with cargo package on Windows
- [Cargo Book: Publishing](https://doc.rust-lang.org/cargo/reference/publishing.html) - crates.io publish is permanent
- [cargo-semver-checks](https://crates.io/crates/cargo-semver-checks) - automated semver violation detection
- Codebase analysis: `src/main.rs`, `src/state.rs`, `src/lib.rs`, `.github/workflows/ci.yml`, `Cargo.toml`, `dist-workspace.toml`
- Project planning: `.planning/PROJECT.md`, `.planning/codebase/CONCERNS.md`, `.planning/codebase/ARCHITECTURE.md`

---
*Pitfalls research for: Rust CLI 1.0 stabilization (repoverlay)*
*Researched: 2026-02-27*
