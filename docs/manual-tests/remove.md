# Remove - Manual Test

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

# Create two local overlays
mkdir -p overlay-a
cat > overlay-a/repoverlay.ccl << 'CCL'
overlay =
  name = overlay-a
CCL
echo "alpha content" > overlay-a/.alpha

mkdir -p overlay-b
cat > overlay-b/repoverlay.ccl << 'CCL'
overlay =
  name = overlay-b
CCL
echo "beta content" > overlay-b/.beta

# Apply both overlays
repoverlay apply ./overlay-a --target ./target-repo
repoverlay apply ./overlay-b --target ./target-repo
```

## Test Cases

### TC-01: Remove named overlay

**Steps:**

```bash
cd "$TEST_DIR"
repoverlay remove overlay-a --target ./target-repo
```

**Expected Output:**

- Success message indicating overlay-a removed
- Files from overlay-a cleaned up

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.alpha" 2>&1
# Should report "No such file or directory" — file was removed

repoverlay status --target "$TEST_DIR/target-repo"
# Should show only overlay-b, NOT overlay-a

cat "$TEST_DIR/target-repo/.git/info/exclude"
# Should NOT contain overlay-a files
```

### TC-02: Remove with --all

**Steps:**

```bash
cd "$TEST_DIR"

# Re-apply overlay-a so both are present
repoverlay apply ./overlay-a --target ./target-repo

repoverlay remove --all --target ./target-repo
```

**Expected Output:**

- Success message indicating all overlays removed

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.alpha" 2>&1
# Should report "No such file or directory"

ls "$TEST_DIR/target-repo/.beta" 2>&1
# Should report "No such file or directory"

repoverlay status --target "$TEST_DIR/target-repo" --quiet
echo "Exit code: $?"
# Exit code should be 1 (no overlays applied)
```

### TC-03: Remove with --dry-run

**Steps:**

```bash
cd "$TEST_DIR"

# Re-apply an overlay
repoverlay apply ./overlay-a --target ./target-repo

repoverlay remove overlay-a --target ./target-repo --dry-run
```

**Expected Output:**

- Shows what would be removed without making changes
- Lists files that would be deleted

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.alpha"
# Should still exist — dry-run made no changes

repoverlay status --target "$TEST_DIR/target-repo"
# Should still show overlay-a as applied
```

### TC-04: Remove non-existent overlay

**Steps:**

```bash
cd "$TEST_DIR"
repoverlay remove does-not-exist --target ./target-repo
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating no overlay named "does-not-exist" is applied
- Non-zero exit code

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo"
# Should still show overlay-a (unchanged)
```

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
