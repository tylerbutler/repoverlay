# Switch and Browse - Manual Test

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

# Create overlay A
mkdir -p overlay-a
cat > overlay-a/repoverlay.ccl << 'CCL'
overlay =
  name = overlay-a
CCL
echo "alpha config" > overlay-a/.config-a

# Create overlay B
mkdir -p overlay-b
cat > overlay-b/repoverlay.ccl << 'CCL'
overlay =
  name = overlay-b
CCL
echo "beta config" > overlay-b/.config-b

# Create a local overlay source for browse tests
mkdir -p browse-source/acme/widgets/widget-overlay
cat > browse-source/acme/widgets/widget-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = widget-overlay
CCL
echo "widget overlay" > browse-source/acme/widgets/widget-overlay/.widget-config

mkdir -p browse-source/other/repo/other-overlay
cat > browse-source/other/repo/other-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = other-overlay
CCL
echo "other overlay" > browse-source/other/repo/other-overlay/.other-config

cd browse-source
git init
git add . && git commit -m "init browse source"
cd "$TEST_DIR"

# Apply overlay A initially
repoverlay apply ./overlay-a --target ./target-repo
```

## Test Cases — Switch

### TC-01: Apply overlay A, switch to overlay B

**Steps:**

```bash
cd "$TEST_DIR"

# Verify overlay A is applied
repoverlay status --target ./target-repo
# Should show overlay-a

# Switch to overlay B
repoverlay switch ./overlay-b --target ./target-repo
```

**Expected Output:**

- Message indicating overlay-a removed and overlay-b applied
- Previous overlay files cleaned up, new overlay files present

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.config-a" 2>&1
# Should report "No such file or directory" — overlay A removed

ls "$TEST_DIR/target-repo/.config-b"
# Should exist — overlay B applied

repoverlay status --target "$TEST_DIR/target-repo"
# Should show only overlay-b, NOT overlay-a
```

### TC-02: Switch with --dry-run

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay switch ./overlay-a --target ./target-repo --dry-run
```

**Expected Output:**

- Shows what would be removed and what would be applied
- No actual changes made

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo"
# Should still show overlay-b (unchanged by dry-run)

ls "$TEST_DIR/target-repo/.config-b"
# Should still exist
```

### TC-03: Switch with --force when conflicts exist

**Steps:**

```bash
cd "$TEST_DIR"

# Create a conflicting file in the target repo
echo "conflicting content" > "$TEST_DIR/target-repo/.config-a"

# Switch to overlay A with --force
repoverlay switch ./overlay-a --target ./target-repo --force
```

**Expected Output:**

- overlay-b removed, overlay-a applied with force overwrite of conflicting file

**Verify:**

```bash
cat "$TEST_DIR/target-repo/.config-a"
# Should contain "alpha config" (overlay content, not "conflicting content")

ls "$TEST_DIR/target-repo/.config-b" 2>&1
# Should report "No such file or directory" — overlay B removed

repoverlay status --target "$TEST_DIR/target-repo"
# Should show only overlay-a
```

## Test Cases — Browse

### TC-04: Browse with --no-interactive

**Steps:**

```bash
cd "$TEST_DIR"

# Browse requires a configured source or a source argument
# Use a local path as source argument to test non-interactive listing
repoverlay browse ./overlay-a --no-interactive --target ./target-repo
```

**Expected Output:**

- Lists available overlays from the source without interactive selection
- Overlay information printed to stdout

**Verify:**

Visual inspection that overlay information is printed to the terminal.

### TC-05: Browse with --show-all

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay browse ./overlay-a --show-all --no-interactive --target ./target-repo
```

**Expected Output:**

- Lists all overlays, including those that may not match the current repository

**Verify:**

Visual inspection that the overlay list is displayed.

### TC-06: Browse with `--filter`

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay browse ./browse-source --filter acme/widgets --no-interactive --target ./target-repo > "$TEST_DIR/browse-filter.txt"
cat "$TEST_DIR/browse-filter.txt"
```

**Expected Output:**

- Lists `widget-overlay`
- Does not list `other-overlay` because it targets a different repository

**Verify:**

```bash
grep "widget-overlay" "$TEST_DIR/browse-filter.txt"
# Should find widget-overlay

grep "other-overlay" "$TEST_DIR/browse-filter.txt" && echo "unexpected overlay listed"
# Should print nothing after grep because other-overlay should not be listed
```

### TC-07: Browse interactively and apply the selected overlay

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay browse ./browse-source --filter acme/widgets --target ./target-repo
```

When the interactive list appears, select `widget-overlay` and press Enter.

**Expected Output:**

- The interactive selector shows `widget-overlay`
- After selection, the overlay is applied to the target repo

**Verify:**

```bash
cat "$TEST_DIR/target-repo/.widget-config"
# Should contain "widget overlay"

repoverlay status --target "$TEST_DIR/target-repo"
# Should show "widget-overlay" as applied
```

### TC-08 (Optional, requires network): Browse GitHub source

**Steps:**

```bash
cd "$TEST_DIR"

# Replace with a real GitHub username or owner/repo
repoverlay browse OWNER --no-interactive --target ./target-repo
```

**Expected Output:**

- Lists overlays available from the GitHub source

**Verify:**

Visual inspection that remote overlays are listed.

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
