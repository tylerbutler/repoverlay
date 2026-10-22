# Update - Manual Test

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

# Create a local overlay
mkdir -p my-overlay
cat > my-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = my-overlay
CCL
echo "version 1" > my-overlay/.config

# Apply the overlay
repoverlay apply ./my-overlay --target ./target-repo
```

## Test Cases

### TC-01: Local symlinks reflect source edits without update

**Steps:**

```bash
cd "$TEST_DIR"

# Modify the overlay source file to simulate an update
echo "version 2" > my-overlay/.config

cat "$TEST_DIR/target-repo/.config"
# Should already contain "version 2", before update runs

# Run update
repoverlay update --target ./target-repo
```

**Expected Output:**

- Reports `my-overlay is a local overlay (not updatable)`
- Reports `All overlays are up to date.` and exits with code 0

**Verify:**

```bash
cat "$TEST_DIR/target-repo/.config"
# Should still contain "version 2" through the existing symlink

repoverlay status --target "$TEST_DIR/target-repo"
# Should show my-overlay as applied
```

### TC-02: Update --dry-run skips local overlays

**Steps:**

```bash
cd "$TEST_DIR"

# Modify source again
echo "version 3" > my-overlay/.config

cp "$TEST_DIR/target-repo/.repoverlay/overlays/my-overlay.ccl" "$TEST_DIR/state-before-dry-run.ccl"
repoverlay update --target ./target-repo --dry-run
```

**Expected Output:**

- Reports `my-overlay is a local overlay (not updatable)`
- Reports `All overlays are up to date.` and exits with code 0
- Does not replace files or change overlay state; local overlays have no remote update to preview

**Verify:**

```bash
cat "$TEST_DIR/target-repo/.config"
# Should contain "version 3" because the source edit is visible through the symlink

cmp "$TEST_DIR/state-before-dry-run.ccl" "$TEST_DIR/target-repo/.repoverlay/overlays/my-overlay.ccl"
# Should succeed; dry-run did not change overlay state

repoverlay status --target "$TEST_DIR/target-repo"
# Should show my-overlay still applied (dry-run made no state changes)
```

### TC-03: Update specific overlay by name

**Steps:**

```bash
cd "$TEST_DIR"

# Create and apply a second overlay
mkdir -p other-overlay
cat > other-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = other-overlay
CCL
echo "other content" > other-overlay/.other

repoverlay apply ./other-overlay --target ./target-repo

# Update only the first overlay by name
repoverlay update my-overlay --target ./target-repo
```

**Expected Output:**

- Reports only `my-overlay` as a local overlay that is not updatable
- Does not process `other-overlay`; both overlays remain unchanged

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo"
# Should show both overlays still applied
```

### TC-04: Update with no GitHub overlays applied

**Steps:**

```bash
cd "$TEST_DIR"

# All overlays are local — update checks for remote changes
repoverlay update --target ./target-repo
```

**Expected Output:**

- Reports both local overlays as not updatable
- Reports `All overlays are up to date.` and exits with code 0

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo"
# Should show overlays still applied and unchanged
```

### TC-05 (Optional, requires network): Update from GitHub source

**Steps:**

```bash
cd "$TEST_DIR"

mkdir target-github && cd target-github
git init
git commit --allow-empty -m "init"
cd "$TEST_DIR"

# Replace with a real public overlay repository URL
repoverlay apply https://github.com/OWNER/REPO --target ./target-github

# Wait for upstream changes, then update
repoverlay update --target ./target-github
```

**Expected Output:**

- Fetches latest version from GitHub and re-applies

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-github"
# Should show the overlay as applied with updated information
```

### TC-06: Update reports library overlays as managed via git

**Steps:**

```bash
cd "$TEST_DIR"

mkdir -p library-overlay
cat > library-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = library-overlay
CCL
echo "from library" > library-overlay/.library-config

repoverlay library import ./library-overlay --target ./target-repo
repoverlay apply library-overlay --target ./target-repo --from @library

repoverlay update --target ./target-repo
```

**Expected Output:**

- Output includes `library-overlay (library overlay — update via git)`
- Other non-updatable overlays remain unchanged

**Verify:**

```bash
cat "$TEST_DIR/target-repo/.library-config"
# Should contain "from library"

repoverlay status --target "$TEST_DIR/target-repo"
# Should still show both "my-overlay" and "library-overlay" as applied
```

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
