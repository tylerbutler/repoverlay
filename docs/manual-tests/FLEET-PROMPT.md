Run the manual test suite for the `repoverlay` CLI tool. The test cases are in
`docs/manual-tests/`. Each `.md` file covers one command or group of commands.

## Step 1: Build

Build the project first:

```
cargo build --release
```

The binary will be at `target/release/repoverlay`.

## Step 2: Dispatch test agents

Dispatch one agent per test file, **all in parallel**. Each agent should:

1. Set up its environment:
   ```bash
   export PATH="<repo-root>/target/release:$PATH"
   export GIT_AUTHOR_NAME="Test" GIT_AUTHOR_EMAIL="test@test.com"
   export GIT_COMMITTER_NAME="Test" GIT_COMMITTER_EMAIL="test@test.com"
   ```

2. Read its assigned test file from `docs/manual-tests/`.

3. Run the **Setup** section once, then execute each **TC-xx** test case in
   order:
   - Run the **Steps** commands exactly as written.
   - Run all **Verify** commands and compare actual output against expected.
   - Record **PASS** or **FAIL** with details for each TC.

4. **Skip** any test case marked `(Optional, requires network)` or
   `(requires network)` unless you have been told network access is available.

5. Run the **Cleanup** section at the end.

6. Return a summary table:
   ```
   | TC-ID | Name | PASS/FAIL | Details (if failed) |
   ```

Here are the test files to dispatch (one agent each):

| Agent | File | Description |
|-------|------|-------------|
| 1 | `docs/manual-tests/apply.md` | Apply overlays (symlink, copy, dry-run, force, merge) |
| 2 | `docs/manual-tests/create.md` | Create overlays from repos |
| 3 | `docs/manual-tests/remove.md` | Remove applied overlays |
| 4 | `docs/manual-tests/restore.md` | Restore overlays after deletion |
| 5 | `docs/manual-tests/status.md` | Status display and filtering |
| 6 | `docs/manual-tests/switch-browse.md` | Switch overlays and browse sources |
| 7 | `docs/manual-tests/update.md` | Update overlays from sources |
| 8 | `docs/manual-tests/edit.md` | Edit add/remove files in overlays |
| 9 | `docs/manual-tests/sync.md` | Sync changes back to overlay source |
| 10 | `docs/manual-tests/source-management.md` | Manage overlay sources |
| 11 | `docs/manual-tests/cache.md` | Cache management |
| 12 | `docs/manual-tests/completions.md` | Shell completion generation |
| 13 | `docs/manual-tests/library.md` | In-repo library import/export/list/remove workflows |
| 14 | `docs/manual-tests/source-resolution.md` | Source priority, `--from`, and upstream fallback |

## Step 3: Collect results and file issues

After all agents complete:

1. Compile a combined results table across all test suites.
2. For each **FAIL**, determine if it is:
   - **A product bug** — unexpected behavior or crash.
   - **A test case bug** — the test expectations don't match intended behavior.
   - **A known issue** — already documented in the test file with an issue link.
3. File a GitHub issue for each **new product bug** found, including:
   - The test case ID and file.
   - Exact reproduction steps (from the test case).
   - Expected vs. actual output.
   - Exit codes.
4. Print a final summary:
   ```
   ## Results
   - Total: XX test cases
   - Passed: XX
   - Failed: XX
   - Skipped: XX (network-dependent)

   ## Issues Filed
   | # | Title | Label |
   ```
