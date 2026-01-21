# ADR 0001: Use Git CLI Instead of Git Library

## Status

Accepted (2026-01-21)

## Context

repoverlay needs to perform various git operations:

- Clone repositories (with shallow depth=1)
- Fetch and checkout specific commits
- Get HEAD commit SHA
- Get remote URL
- List ignored/untracked files
- Stage, commit, and push changes (for overlay_repo publishing)

We evaluated whether to replace the current `Command::new("git")` approach with a Rust git library to gain:

1. **Robustness** - Type-safe APIs instead of parsing CLI output
2. **Cross-platform consistency** - No dependency on git CLI being installed
3. **Performance** - Avoid process spawning overhead
4. **Fewer external dependencies** - Self-contained binary

Two libraries were considered:

- **git2-rs** (libgit2 bindings)
- **gitoxide** (pure Rust implementation)

## Decision

**Keep the current git CLI approach.**

## Consequences

### Positive

- Binary size remains ~2.2 MB (vs ~5-6 MB with gitoxide)
- No additional compile-time dependencies
- Full git feature coverage (push, shallow clone, etc.)
- Simpler codebase without library abstraction layer

### Negative

- Requires git CLI to be installed on user's system
- Output parsing is fragile (though well-tested)
- Process spawning has some overhead
- Potential for platform-specific CLI behavior differences

## Alternatives Considered

### git2-rs (libgit2)

- **Rejected because:** Does not support shallow clones (`--depth`), which is critical for fast GitHub overlay fetching. [Issue #875](https://github.com/rust-lang/git2-rs/issues/875) was closed as "not planned."
- Binary size impact: ~1.5 MB addition

### gitoxide (gix)

- **Rejected because:**
  - Binary size increase of ~3 MB (2.2 MB → 5.3 MB, roughly 2.5x)
  - Push is not implemented ([status as of Jan 2025](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md))
  - Staging API is low-level (`dangerously_push_entry()` instead of `add_path()`)
  - Pull requires manual implementation (fetch + fast-forward merge)

#### gitoxide Feature Coverage (as evaluated)

| Operation | Support | Notes |
|-----------|---------|-------|
| Clone (shallow) | Full | `prepare_clone().with_shallow(Shallow::DepthAtRemote(1))` |
| Fetch | Full | Supported |
| Get HEAD | Full | `repo.head_id()` |
| Get remote URL | Full | `repo.find_remote()` |
| Status (ignored/untracked) | Full | `repo.status()` |
| Stage files | Partial | Only low-level `dangerously_push_entry()` |
| Commit | Partial | Works, but hooks not supported |
| Push | None | Not implemented |

## Future Considerations

This decision should be revisited if:

- gitoxide adds push support and improves staging ergonomics
- Binary size becomes less important (e.g., server-only deployment)
- Git CLI output parsing causes actual bugs in production
- Cross-platform compatibility issues arise with git CLI

## References

- [gitoxide GitHub](https://github.com/GitoxideLabs/gitoxide)
- [gitoxide crate status](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md)
- [git2-rs shallow clone issue](https://github.com/rust-lang/git2-rs/issues/875)
- [gix docs](https://docs.rs/gix/latest/gix/)
