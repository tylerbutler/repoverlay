# Source Management - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH
- `git` installed

## Setup

```bash
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"

# Create a target git repository
mkdir target-repo && cd target-repo
git init
git commit --allow-empty -m "init"
cd "$TEST_DIR"

# Create a local directory to use as a source
mkdir -p local-source/my-overlay
cat > local-source/my-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = my-overlay
CCL
echo "overlay content" > local-source/my-overlay/.config

# Initialize local-source as a git repo (sources typically need to be git repos)
cd local-source
git init
git add . && git commit -m "init overlay source"
cd "$TEST_DIR"
```

## Test Cases

### TC-01: source add with a local path

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay source add "file://$TEST_DIR/local-source" --name local
```

**Expected Output:**

- Success message indicating source added
- Source name "local" configured

**Verify:**

```bash
repoverlay source list
# Should show "local" source pointing to the local-source path
```

### TC-02: source list

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay source list
```

**Expected Output:**

- Displays configured sources
- Shows "local" source added in TC-01

**Verify:**

Visual inspection that the source list includes:
- Source name ("local")
- Source URL or path

### TC-03: source remove

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay source remove local
```

**Expected Output:**

- Success message indicating source removed

**Verify:**

```bash
repoverlay source list
# Should NOT show "local" source anymore (empty list or other sources only)
```

### TC-04: source add duplicate

**Steps:**

```bash
cd "$TEST_DIR"

# Add the source back
repoverlay source add "file://$TEST_DIR/local-source" --name local

# Try adding the same source again
repoverlay source add "file://$TEST_DIR/local-source" --name local
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating source already exists, or updates the existing source
- Behavior should be clear and predictable

**Verify:**

```bash
repoverlay source list
# Should show exactly one "local" source (not duplicated)
```

### TC-05 (Optional, requires network): source add with GitHub URL

**Steps:**

```bash
cd "$TEST_DIR"

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

```bash
rm -rf "$TEST_DIR"
```
