# Sync - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH
- `git` installed

> **Note:** `sync` only works with overlays applied from an overlay repo source
> (e.g., `org/repo/name` format). Overlays applied directly from local paths
> (`./my-overlay`) are not syncable. TC-01 through TC-03 verify this constraint,
> while TC-04 and TC-05 test error handling.
>
> The `sync` command also requires the target repository to have a git remote
> origin for org/repo detection.

## Setup

```bash
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"

# Create a target git repository (with a fake remote for sync's org/repo detection)
mkdir target-repo && cd target-repo
git init
git remote add origin https://github.com/example/repo.git
git commit --allow-empty -m "init"
cd "$TEST_DIR"

# Create a local overlay
mkdir -p my-overlay
cat > my-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = my-overlay
CCL
echo "original content" > my-overlay/.config

# Initialize overlay as a git repo (sync commits changes back)
cd my-overlay
git init
git add . && git commit -m "init overlay"
cd "$TEST_DIR"

# Apply the overlay to the target repo
repoverlay apply ./my-overlay --target ./target-repo
```

## Test Cases

### TC-01: Sync rejects locally-applied overlay

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay sync my-overlay --target ./target-repo
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating local source overlays cannot be synced
- Non-zero exit code

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo"
# Should show my-overlay still applied (unchanged)
```

### TC-02: Sync --dry-run rejects locally-applied overlay

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay sync my-overlay --target ./target-repo --dry-run
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating local source cannot be synced
- Non-zero exit code

### TC-03: Sync --all with only local overlays applied

> **Known issue:** [#205](https://github.com/tylerbutler/repoverlay/issues/205) —
> `sync --all` does not gracefully skip local-source overlays.

**Steps:**

```bash
cd "$TEST_DIR"

# Create and apply a second local overlay
mkdir -p other-overlay
cat > other-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = other-overlay
CCL
echo "other content" > other-overlay/.other

cd other-overlay
git init
git add . && git commit -m "init other overlay"
cd "$TEST_DIR"

repoverlay apply ./other-overlay --target ./target-repo

repoverlay sync --all --target ./target-repo
echo "Exit code: $?"
```

**Expected Output (after #205 is fixed):**

- Messages indicating each overlay skipped (local sources not syncable)
- Summary like "Synced 0 overlay(s), skipped 2"

**Current Output:**

- `Error: Failed to pull overlay repository` — attempts remote pull instead of skipping
- Non-zero exit code

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo"
# Should show both overlays still applied (unchanged)
```

### TC-04: Sync non-existent overlay

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay sync does-not-exist --target ./target-repo
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating overlay is not applied
- Non-zero exit code

### TC-05: Sync with no overlay name and no --all

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay sync --target ./target-repo
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating overlay name is required or --all must be used
- Non-zero exit code, OR selects the single applied overlay if only one exists

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
