# Source Management - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH
- `git` installed

## Setup

Run in a new shell. Local sources must be directories inside the current Git
repository. Use a filesystem path, not a `file://` URL.

```bash
TEST_DIR=$(mktemp -d)
export XDG_CONFIG_HOME="$TEST_DIR/config"
export XDG_DATA_HOME="$TEST_DIR/data"
export XDG_CACHE_HOME="$TEST_DIR/cache"
export REPOVERLAY_CACHE_DIR="$TEST_DIR/cache"
export REPOVERLAY_NO_UPDATE_CHECK=1
cd "$TEST_DIR"

# Create a target git repository
mkdir target-repo && cd target-repo
git init
git commit --allow-empty -m "init"

# Keep the local source inside target-repo
mkdir -p local-source/my-overlay
cat > local-source/my-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = my-overlay
CCL
echo "overlay content" > local-source/my-overlay/.config

printf 'Disposable test directory: %s\n' "$TEST_DIR"
```

## Test Cases

### TC-01: source add with an absolute local path

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay source add "$TEST_DIR/target-repo/local-source" --name local
```

**Expected Output:**

- Success message indicating source added
- Source name "local" configured in `target-repo/.repoverlay/config.ccl`
- Repository-relative path `local-source`

**Verify:**

```bash
repoverlay source list
# Should show "local (path: local-source)" under Repository sources
```

### TC-02: source list

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay source list
```

**Expected Output:**

- Displays configured sources
- Shows "local" source added in TC-01

**Verify:**

Visual inspection that the source list includes:
- Source name ("local")
- Repository-relative path (`local-source`)

### TC-03: source remove

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay source remove local
```

**Expected Output:**

- Success message indicating source removed

**Verify:**

```bash
repoverlay source list
# Should report "No overlay sources configured."
```

### TC-04: source add duplicate local path

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

# Add the source back
repoverlay source add "$TEST_DIR/target-repo/local-source" --name local

# Try adding the same source again
repoverlay source add "$TEST_DIR/target-repo/local-source" --name local
echo "Exit code: $?"
```

**Expected Output:**

- First add succeeds
- Second add reports `Source 'local' already exists` and exits with code 1

**Verify:**

```bash
repoverlay source list
# Should show exactly one "local" source (not duplicated)
```

### TC-05 (Optional, requires network): source add with GitHub URL

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

# Replace with a real GitHub URL
repoverlay source add https://github.com/OWNER/REPO --name github-source
```

**Expected Output:**

- Success message indicating GitHub source added

**Verify:**

```bash
repoverlay source list
# Should show "github-source" pointing to the GitHub URL
```

## Cleanup

Exit the test shell to restore your environment. Remove only the disposable
directory printed during Setup after you record the results.
