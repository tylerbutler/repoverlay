# Phase 2: Test Coverage - Research

**Researched:** 2026-03-01
**Domain:** Rust test coverage hardening, mutation testing, regression test writing
**Confidence:** HIGH

## Summary

Phase 2 builds on a clean Phase 1 baseline: all 15 source modules reviewed, zero correctness bugs found, two user-facing fixes applied (FIX-02 error format, FIX-03 SIGPIPE). The task is to close identified coverage gaps and verify behavioral completeness with mutation testing. The test infrastructure is solid — `assert_cmd` + `predicates` + `tempfile::TempDir` is already in use across 913 unit tests and 93 integration tests.

Path traversal already has unit tests (`path_traversal_tests` module in lib.rs with 4 tests) and one weak integration test. The gaps are: no tests for absolute path mappings, no symlink-chain tests after placement, and the integration test doesn't assert failure — it only checks that success doesn't escape. Cache failure recovery has zero tests for the partial-failure scenario. Terminal recovery (TEST-03) is largely untestable in automation — `prompt_conflict_interactive` reads from stdin, and `selection.rs` requires a PTY; these are manual-only paths. Regression tests for Phase 1 fixes are minimal: the 7 `source_resolver_bugs` tests existed pre-Phase-1, and the actual Phase 1 code changes (FIX-02, FIX-03) have no regression tests yet. `cargo-mutants` is not installed and must be added before TEST-05/TEST-06.

**Primary recommendation:** Install `cargo-mutants` early (TEST-05 planning needs it), write regression tests for Phase 1 fixes (TEST-04), strengthen path traversal tests (TEST-01), add cache failure recovery tests (TEST-02), document terminal recovery as manual-only (TEST-03), then run mutation testing and address survivors.

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| TEST-01 | Coverage gaps closed for path traversal edge cases | Existing `path_traversal_tests` module has 4 unit tests; integration test is weak; symlink-chain bypass untested; absolute path rejection tested in code but not dedicated test |
| TEST-02 | Coverage gaps closed for cache update failure recovery | `cache.rs` tests cover create/list/remove/metadata but NOT partial-failure scenarios in `ensure_cached`; `update_repo` failure leaves stale cached state untested |
| TEST-03 | Coverage gaps closed for terminal recovery (Ctrl+C in interactive selection) | `prompt_conflict_interactive` reads from stdin — partially testable via stdin injection; `selection.rs` terminal raw mode requires PTY; most paths are manual-only |
| TEST-04 | Regression tests added for every bug fixed during review | FIX-02 (error format): no regression test exists; FIX-03 (SIGPIPE): no regression test exists; FIX-04 (7 source_resolver_bugs tests): pre-existing, already covering #142-#148 |
| TEST-05 | Mutation testing run with cargo-mutants to identify untested behaviors | `cargo-mutants` not installed; `just` has no mutants target; must be installed and a baseline run completed |
| TEST-06 | Surviving mutants addressed with additional tests or documented | Depends on TEST-05 results; surviving mutants are categorized as: add test, add assertion, or document as equivalent mutant |
</phase_requirements>

## Standard Stack

### Core (All Already Installed)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| assert_cmd | 2 | CLI integration testing | Already in use; primary integration test tool |
| predicates | 3 | Test output assertions | Already in use; paired with assert_cmd |
| tempfile | 3 | Temporary test directories | Already in use; critical for isolation |

### New Tooling Required

| Tool | Version | Purpose | Installation |
|------|---------|---------|--------------|
| cargo-mutants | 26.2+ | Mutation testing to find untested behaviors | `cargo install cargo-mutants` |

**cargo-mutants is not installed.** It must be installed before TEST-05 and TEST-06 can be addressed. Verify: `cargo mutants --version`.

### Supporting (Available but Not Used)

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| cargo-nextest | 0.9+ | Faster parallel test execution | Already configured as `just test-fast`; use for mutation testing `--test-tool=nextest` |
| cargo-llvm-cov | 0.8.4 | Coverage HTML reports | Already configured; run `just coverage-html` to identify uncovered lines before writing tests |

**Installation:**
```bash
# Required new tool
cargo install cargo-mutants

# Already installed, verify
cargo mutants --version
just test-coverage  # verify lcov pipeline works
```

## Architecture Patterns

### Existing Test Organization

```
tests/
├── cli.rs              (1844 lines — CLI integration tests)
│   ├── Help/Version displays
│   ├── Apply Command Tests
│   ├── Remove Command Tests
│   ├── ... (all commands)
│   ├── Security Tests  (line 911 — path_traversal integration test)
│   └── Workflow Integration Tests
└── common/
    └── mod.rs          (134 lines — TestContext, SourceTestContext, fixtures)

src/
├── lib.rs              (unit tests embedded, ~263 #[test] annotations)
│   ├── mod path_traversal_tests   (line 5715 — 4 existing tests)
│   ├── mod symlink_escape_tests   (line 5822 — 1 unix test)
│   └── ... (many other test modules)
├── cache.rs            (unit tests at line 438 — 14+ tests, none for partial failure)
├── cli.rs              (unit tests at line 2774)
│   └── mod source_resolver_bugs  (line 7603 — 7 regression tests for #142-#148)
└── ... (other modules with inline tests)
```

### Pattern 1: Module-Local Unit Tests (Use for TEST-01, TEST-02, TEST-04)

Add inside existing `#[cfg(test)] mod tests { }` blocks, grouped with inner modules:

```rust
// In src/lib.rs, inside existing #[cfg(test)] mod tests { }
// Append to existing path_traversal_tests module OR add new submodule:

mod path_traversal_edge_cases {
    use super::*;

    #[test]
    fn rejects_absolute_path_in_mapping() {
        // mapping: file.txt = /etc/passwd
        // expect: Err containing "Absolute paths not allowed"
    }

    #[test]
    fn rejects_windows_absolute_path_in_mapping() {
        // mapping: file.txt = C:\Windows\System32\etc
        // expect: Err containing "Absolute paths not allowed"
    }
}
```

### Pattern 2: Integration Test Expansion (Use for TEST-01, TEST-04)

Append at end of `tests/cli.rs` with clear comment separators:

```rust
// ==================== 1.0 Stabilization: Regression Tests (Phase 2) ====================

#[test]
fn error_messages_use_display_format_not_debug() {
    // Trigger a known error (apply to non-git dir)
    // Assert stderr does NOT contain "Os {" or "code:" or "kind:"
    // Assert stderr DOES contain a human-readable message
}

#[test]
fn apply_path_traversal_fails_with_clear_error() {
    // Use mapping with ../escape path
    // Assert failure (not just "if success")
    // Assert stderr contains "Path traversal" or "path traversal"
}
```

### Pattern 3: Cache Failure Simulation (Use for TEST-02)

Since `CacheManager` methods are private and require real git, simulate failure by:
1. Creating a real (temporary) git repo as the "cached" repo
2. Corrupting the metadata file
3. Making the repo non-fetchable by removing its remote
4. Calling `ensure_cached` and verifying graceful error handling

```rust
// In src/cache.rs, inside mod tests { }
#[test]
fn ensure_cached_handles_corrupted_metadata_gracefully() {
    let temp = TempDir::new().unwrap();
    let manager = CacheManager { cache_dir: temp.path().to_path_buf() };

    // Create the "cache" directory structure for a repo
    let repo_path = temp.path().join("github/owner/repo");
    // ... init git repo ...
    // Write corrupted metadata
    fs::write(repo_path.join(".repoverlay-cache-meta.ccl"), "invalid{ccl").unwrap();

    // ensure_cached should either recover or return a clear error
    // NOT panic or leave partial state
}
```

### Pattern 4: Stdin Injection for Interactive Conflict (Use for TEST-03)

`prompt_conflict_interactive` reads from `io::stdin()`. Testing via stdin pipe:

```rust
// In tests/cli.rs
#[test]
fn apply_interactive_conflict_abort_cleans_up() {
    // Set up conflict scenario
    // Pipe "a\n" (abort choice) to stdin via .write_stdin()
    cargo_bin_cmd!("repoverlay")
        .args(["apply", ..., "--conflict", "interactive"])
        .write_stdin("a\n")
        .assert()
        .failure();
    // Verify no partial state left
}
```

NOTE: `selection.rs` terminal raw mode (crossterm) CANNOT be tested in automated tests — it requires a PTY. These paths are documented as manual-only.

### Anti-Patterns to Avoid

- **Weak path traversal test:** The existing `apply_rejects_path_traversal_attempt` in `tests/cli.rs` (line 916) uses `if result.status.success() { ... }` — it accepts both success and failure. New tests MUST assert `.failure()` unambiguously.
- **Global state in cache tests:** Always use `CacheManager { cache_dir: temp.path().to_path_buf() }` with a fresh `TempDir`, never the default `CacheManager::new()` which uses `~/.cache/repoverlay/`.
- **Testing terminal raw mode:** Do NOT attempt to test `selection.rs` crossterm rendering in automated tests. It requires a PTY and will fail in CI.
- **Mutants on full codebase without scoping:** cargo-mutants on all 15 modules will run for hours. Scope to highest-risk modules first: `lib.rs`, `cache.rs`, `sources.rs`, `state.rs`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Mutation testing | Custom "delete code and check tests" scripts | cargo-mutants | Handles mutation set intelligently, integrates with nextest, emits GH annotations |
| Coverage reports | Manual line counting | `just coverage-html` (already configured) | LLVM-based, accurate branch coverage |
| Stdin injection for assert_cmd | Custom pipe setup | `.write_stdin()` on assert_cmd's `Command` | Built-in; handles process stdin correctly |

**Key insight:** The test infrastructure is mature. The work in Phase 2 is writing tests, not building test infrastructure.

## Common Pitfalls

### Pitfall 1: Path Traversal Test Passes When It Should Fail

**What goes wrong:** The existing `apply_rejects_path_traversal_attempt` in `tests/cli.rs` accepts EITHER failure OR success-without-escape. If the behavior changes to allow the traversal silently, the test still passes.

**Why it happens:** Test was written defensively because the behavior wasn't guaranteed to fail.

**How to avoid:** New path traversal tests MUST use `.assert().failure()` unconditionally. The code at lib.rs line 1289 already `bail!`s on path traversal. A test that allows success is not a regression test.

**Warning signs:** Test contains `if result.status.success() { ... }` pattern.

### Pitfall 2: cargo-mutants Timeout on Large Files

**What goes wrong:** lib.rs is ~7200 lines and cli.rs is ~7965 lines. Running cargo-mutants on the entire codebase without scoping will take hours and may time out.

**Why it happens:** Each mutant requires a full `cargo test` run. With 900+ unit tests and 90+ integration tests, each test run takes 30-60 seconds.

**How to avoid:**
- Scope mutation testing to specific files: `cargo mutants --file src/lib.rs`
- Use nextest for faster runs: `cargo mutants --test-tool=nextest`
- Set a timeout: `cargo mutants --timeout 60`
- Start with the highest-risk, smallest modules: `cache.rs`, `sources.rs`, `state.rs`

**Warning signs:** Mutation run started on full codebase without `--file` scope.

### Pitfall 3: Cache Failure Tests Require Real Git

**What goes wrong:** `cache.rs` methods that interact with git (`clone_repo`, `update_repo`, `checkout_ref`) spawn real `git` commands. You cannot simulate "git fetch failed" without controlling the remote.

**Why it happens:** No mocking infrastructure exists in this codebase (intentional — real filesystem operations are preferred).

**How to avoid:**
- Test `save_meta` failure by making the cache directory read-only temporarily
- Test `load_meta` with corrupted files
- Test `ensure_cached` with a non-existent repo path that triggers the clone path — then verify the error propagates correctly
- For `update_repo` failure: create a local git repo (not a remote), break its `origin` remote, call methods that fetch

**Warning signs:** Test requires network access without `#[ignore]`.

### Pitfall 4: TEST-03 Terminal Recovery Is Mostly Manual-Only

**What goes wrong:** Attempting to automate Ctrl+C in `selection.rs` creates fragile tests that require PTY manipulation.

**Why it happens:** `crossterm` requires raw terminal mode which isn't available in CI pipes.

**How to avoid:**
- `prompt_conflict_interactive` (in lib.rs) CAN be partially tested via stdin injection with `write_stdin("a\n")` for the abort path
- `selection.rs` terminal raw mode recovery on Ctrl+C is MANUAL-ONLY — document this explicitly in the plan
- The CONCERNS.md already notes `selection.rs` as untestable; accept this and document

**Warning signs:** Test tries to import crossterm or send SIGINT programmatically.

### Pitfall 5: Mutation Testing Surviving "Equivalent Mutants"

**What goes wrong:** cargo-mutants reports surviving mutants that are actually equivalent — they change the code but produce the same behavior.

**Why it happens:** For example, replacing `>= 0` with `> 0` in a context where the value is always positive produces a mutant that all tests pass but the mutation is semantically equivalent.

**How to avoid:**
- Document equivalent mutants explicitly: create `mutants.out/equivalent-mutants.txt`
- Only add tests for mutants that represent real behavioral gaps
- Distinguish: "tests don't cover this path" vs "this mutation produces identical output"

**Warning signs:** Adding tests for every surviving mutant regardless of whether they're equivalent.

## Code Examples

Verified patterns from existing codebase:

### Path Traversal Test Pattern (from src/lib.rs line 5715)
```rust
// Source: src/lib.rs mod path_traversal_tests
fn try_apply(overlay: &Path, target: &Path) -> Result<()> {
    let resolved = ResolvedSource {
        path: overlay.to_path_buf(),
        source_info: OverlaySource::local(overlay.to_path_buf()),
    };
    let canonical = target.canonicalize().unwrap();
    apply_resolved_overlay(
        &resolved,
        &canonical,
        true,  // save_state
        None,
        ConflictStrategy::default(),
        false, // force
    )
}

#[test]
fn rejects_escape_at_root() {
    let repo = create_test_repo();
    let overlay = TempDir::new().unwrap();
    make_overlay_with_config(
        overlay.path(),
        &[("secret.txt", "payload")],
        "mappings =\n  secret.txt = ../etc/passwd\n",
    );
    let result = try_apply(overlay.path(), repo.path());
    assert!(result.is_err(), "should reject ../etc/passwd mapping");
    assert!(result.unwrap_err().to_string().contains("Path traversal"));
}
```

### Cache Manager Test Pattern (from src/cache.rs line 438)
```rust
// Source: src/cache.rs mod tests
// Use local CacheManager with TempDir to avoid polluting real cache
fn make_test_manager() -> (CacheManager, TempDir) {
    let temp = TempDir::new().unwrap();
    let manager = CacheManager { cache_dir: temp.path().to_path_buf() };
    (manager, temp)  // Return TempDir to keep it alive
}
```

### Regression Test for Error Format (FIX-02 pattern)
```rust
// In tests/cli.rs — pattern for asserting Display format errors
#[test]
fn error_messages_use_display_not_debug_format() {
    // Apply to a non-git directory triggers a known error
    let temp = tempfile::TempDir::new().unwrap();
    let overlay = TestContext::new().with_overlay(&envrc_overlay());

    cargo_bin_cmd!("repoverlay")
        .args(["apply", overlay.overlay_source()])
        .args(["--target", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a git repository"))
        // Debug format would contain: Os { code: N, kind: ..., message: "..." }
        .stderr(predicate::str::contains("Os {").not())
        .stderr(predicate::str::contains("kind: ").not());
}
```

### Interactive Conflict Stdin Injection Pattern
```rust
// In tests/cli.rs — using assert_cmd write_stdin
#[test]
fn apply_interactive_conflict_abort_removes_partial_state() {
    let ctx = TestContext::new().with_overlay(&envrc_overlay());
    // Pre-create conflicting file
    ctx.create_repo_file(".envrc", "existing content");

    cargo_bin_cmd!("repoverlay")
        .args(["apply", ctx.overlay_source()])
        .args(["--target", ctx.repo_path().to_str().unwrap()])
        .args(["--conflict", "interactive"])
        .write_stdin("a\n")  // 'a' = abort
        .assert()
        .failure();

    // Verify no partial state exists
    // (state file should not have been created for aborted apply)
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `{e:?}` debug format | `{e:#}` display format | Phase 1 (FIX-02) | User-readable errors; regression test needed to lock this in |
| No SIGPIPE handling | `libc::signal(SIGPIPE, SIG_DFL)` at main() | Phase 1 (FIX-03) | Clean pipe exit; hard to regression-test in integration tests |
| No mutation testing | cargo-mutants for behavioral gap detection | Phase 2 (new) | Finds untested behaviors that coverage metrics miss |

## Open Questions

1. **cargo-mutants scope and timing**
   - What we know: lib.rs (~7200 lines) and cli.rs (~7965 lines) will take hours to mutate fully; cache.rs (~700 lines) is much faster
   - What's unclear: How long a full mutation run takes on this codebase; whether CI will run mutation tests or just local
   - Recommendation: Start with `cargo mutants --file src/cache.rs --file src/sources.rs --file src/state.rs` (highest-value, manageable size) before attempting lib.rs

2. **TEST-03 automation scope**
   - What we know: `prompt_conflict_interactive` in lib.rs reads from `io::stdin()` and can be reached via integration tests with `--conflict interactive`; `selection.rs` terminal rendering requires PTY
   - What's unclear: Whether assert_cmd's `write_stdin` properly triggers the conflict prompt path in an integration test (depends on whether the conflict scenario can be set up reliably)
   - Recommendation: Attempt stdin injection for `prompt_conflict_interactive`; document `selection.rs` raw mode as manual-only; this is sufficient for TEST-03

3. **Phase 1 SIGPIPE regression test feasibility**
   - What we know: FIX-03 restores SIGPIPE default; testing requires piping output to a command that exits early (e.g., `head -0`)
   - What's unclear: assert_cmd doesn't support piped process composition; testing SIGPIPE behavior in Rust tests is non-trivial
   - Recommendation: Write a shell-level smoke test or document this as manual-only; the code change is trivially correct and well-understood

4. **Mutation testing on lib.rs**
   - What we know: lib.rs is 7200+ lines with 263 #[test] annotations and a full integration test suite; it will take a very long time to mutate
   - What's unclear: Whether the surviving mutant count will be manageable or require days of work
   - Recommendation: Run mutation testing on lib.rs last, after completing smaller modules. Use `--timeout 30` to skip slow-running mutants. Cap effort at 2-3 rounds of addressing survivors.

## Sources

### Primary (HIGH confidence)
- Direct codebase inspection: `src/lib.rs` lines 5715-5860 (existing path traversal tests)
- Direct codebase inspection: `src/cache.rs` lines 95-280, 438-760 (ensure_cached logic + existing tests)
- Direct codebase inspection: `src/lib.rs` lines 126-156 (prompt_conflict_interactive)
- Direct codebase inspection: `tests/cli.rs` lines 911-962 (existing path traversal integration test)
- Direct codebase inspection: `src/cli.rs` lines 7603-7960 (source_resolver_bugs regression tests)
- `.planning/codebase/CONCERNS.md` — coverage gaps documented 2026-02-27
- `.planning/codebase/TESTING.md` — test patterns documented 2026-02-27
- `.planning/phases/01-code-review-and-bug-fixes/01-01-SUMMARY.md` — Phase 1 findings
- `.planning/phases/01-code-review-and-bug-fixes/01-02-SUMMARY.md` — Phase 1 fixes (FIX-02, FIX-03)
- `.planning/research/STACK.md` — cargo-mutants recommendation (HIGH confidence, 2026-02-27)
- `.planning/config.json` — `workflow.nyquist_validation` not present (skip Validation Architecture section)

### Secondary (MEDIUM confidence)
- cargo-mutants docs: https://mutants.rs/ — v26.2.0, `--test-tool=nextest`, `--file` scoping, `--timeout` flag

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — all tools already in use except cargo-mutants which is well-documented
- Architecture: HIGH — based on direct codebase inspection, not inference
- Pitfalls: HIGH — based on actual code patterns found in codebase, not theoretical
- Mutation testing pitfalls: MEDIUM — specific timeout/scope numbers are estimates pending actual run

**Research date:** 2026-03-01
**Valid until:** 2026-04-01 (stable stack, Rust ecosystem moves slowly)
