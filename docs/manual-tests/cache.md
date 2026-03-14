# Cache - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH

## Setup

No special setup is needed — cache commands operate on the global cache directory.

## Test Cases

### TC-01: Show cache path

**Steps:**

```bash
repoverlay cache path
```

**Expected Output:**

- Prints the absolute path to the cache directory (e.g., `~/.cache/repoverlay`)

**Verify:**

```bash
CACHE_PATH=$(repoverlay cache path)
echo "$CACHE_PATH"
# Should be a valid absolute path
# Path should end with "repoverlay" or similar
```

### TC-02: List cached repositories (empty cache)

**Steps:**

```bash
repoverlay cache list
```

**Expected Output:**

- Message indicating no cached repositories, or an empty list

**Verify:**

```bash
repoverlay cache list
echo "Exit code: $?"
# Exit code should be 0 (not an error to have an empty cache)
```

### TC-03: Remove with --all on empty cache

**Steps:**

```bash
repoverlay cache remove --all --yes
echo "Exit code: $?"
```

**Expected Output:**

- Message indicating cache cleared or no cached repositories to remove
- Exit code 0

### TC-04 (Optional, requires network): Cache populated after GitHub apply

**Steps:**

```bash
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"

mkdir target-repo && cd target-repo
git init
git commit --allow-empty -m "init"
cd "$TEST_DIR"

# Replace with a real public overlay repository URL
repoverlay apply https://github.com/OWNER/REPO --target ./target-repo

repoverlay cache list
```

**Expected Output:**

- Cache list shows the cloned GitHub repository
- Displays owner/repo, ref, commit hash, and fetched timestamp

**Verify:**

```bash
CACHE_PATH=$(repoverlay cache path)
ls "$CACHE_PATH"
# Should contain a directory for the cached repository
```

### TC-05 (Optional, requires network): Remove specific cached repository

**Steps:**

```bash
# Assumes TC-04 has populated the cache with OWNER/REPO
repoverlay cache remove OWNER/REPO
```

**Expected Output:**

- Success message indicating the repository was removed from cache

**Verify:**

```bash
repoverlay cache list
# Should no longer show the removed repository
```

## Cleanup

```bash
# Only if TC-04/TC-05 were run
rm -rf "$TEST_DIR"
```
