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

### TC-01: Apply local overlay, modify source, run update

**Steps:**

```bash
cd "$TEST_DIR"

# Modify the overlay source file to simulate an update
echo "version 2" > my-overlay/.config

# Run update
repoverlay update --target ./target-repo
```

**Expected Output:**

- Message indicating overlay updated or files refreshed

**Verify:**

```bash
cat "$TEST_DIR/target-repo/.config"
# For symlink mode: should already reflect "version 2" (symlinks auto-update)
# For copy mode: update would replace the copied file with new content

repoverlay status --target "$TEST_DIR/target-repo"
# Should show my-overlay as applied
```

### TC-02: Update with --dry-run

**Steps:**

```bash
cd "$TEST_DIR"

# Modify source again
echo "version 3" > my-overlay/.config

repoverlay update --target ./target-repo --dry-run
```

**Expected Output:**

- Shows what would be updated without making changes
- Lists any files that would change

**Verify:**

```bash
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

- Only my-overlay is updated
- other-overlay is not affected

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

- Message indicating no remote overlays to update, or that local overlays are up to date

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
