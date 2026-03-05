# Status - Manual Test

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
echo "export FOO=bar" > my-overlay/.envrc
echo '{"editor.tabSize": 2}' > my-overlay/.vscode-settings.json
```

## Test Cases

### TC-01: Status with no overlays applied

**Steps:**

```bash
cd "$TEST_DIR"
repoverlay status --target ./target-repo
```

**Expected Output:**

- Message indicating no overlays are currently applied

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo" --quiet
echo "Exit code: $?"
# Exit code should be 1 (no overlays)
```

### TC-02: Status with applied overlay

**Steps:**

```bash
cd "$TEST_DIR"
repoverlay apply ./my-overlay --target ./target-repo

repoverlay status --target ./target-repo
```

**Expected Output:**

- Displays overlay name ("my-overlay")
- Shows applied files
- Shows source path
- Shows application mode (symlink or copy)

**Verify:**

Visual inspection that the output includes:
- Overlay name
- File list
- Source information

### TC-03: Status with --json

**Steps:**

```bash
cd "$TEST_DIR"
repoverlay status --target ./target-repo --json
```

**Expected Output:**

- Valid JSON output containing overlay information

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo" --json | python3 -m json.tool
# Should parse successfully as valid JSON
# JSON should contain the overlay name and file information
```

### TC-04: Status with --quiet

**Steps:**

```bash
cd "$TEST_DIR"

# With overlays applied (from TC-02)
repoverlay status --target ./target-repo --quiet
echo "Exit code: $?"
# Expected: Exit code 0 (overlays present)

# Remove all overlays and check again
repoverlay remove --all --target ./target-repo

repoverlay status --target ./target-repo --quiet
echo "Exit code: $?"
# Expected: Exit code 1 (no overlays)
```

**Expected Output:**

- No text output in quiet mode
- Exit code 0 when overlays are applied
- Exit code 1 when no overlays are applied

**Verify:**

Check exit codes match expectations above.

### TC-05: Status with --name filter

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

repoverlay apply ./my-overlay --target ./target-repo
repoverlay apply ./other-overlay --target ./target-repo

# Filter by name
repoverlay status --target ./target-repo --name my-overlay
```

**Expected Output:**

- Shows only "my-overlay" information
- Does NOT show "other-overlay"

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo" --name my-overlay
# Should show only my-overlay details

repoverlay status --target "$TEST_DIR/target-repo" --name other-overlay
# Should show only other-overlay details

repoverlay status --target "$TEST_DIR/target-repo" --name nonexistent
# Should show no results or error
```

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
