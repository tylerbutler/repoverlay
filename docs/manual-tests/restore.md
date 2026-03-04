# Restore - Manual Test

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

# Apply the overlay
repoverlay apply ./my-overlay --target ./target-repo
```

## Test Cases

### TC-01: Apply overlay, delete the symlinked file, restore

**Steps:**

```bash
cd "$TEST_DIR"

# Verify file exists before deletion
ls -la "$TEST_DIR/target-repo/.envrc"

# Simulate git clean or manual deletion of overlay files
rm -f "$TEST_DIR/target-repo/.envrc"
rm -f "$TEST_DIR/target-repo/.vscode-settings.json"

# Verify files are gone
ls "$TEST_DIR/target-repo/.envrc" 2>&1
# Should report "No such file or directory"

# Restore
repoverlay restore --target "$TEST_DIR/target-repo"
```

**Expected Output:**

- Success message indicating overlay files restored

**Verify:**

```bash
ls -la "$TEST_DIR/target-repo/.envrc"
# Should exist again (symlink recreated)

cat "$TEST_DIR/target-repo/.envrc"
# Should contain "export FOO=bar"

repoverlay status --target "$TEST_DIR/target-repo"
# Should show my-overlay as applied and healthy
```

### TC-02: Restore with --dry-run

**Steps:**

```bash
cd "$TEST_DIR"

# Delete overlay files again
rm -f "$TEST_DIR/target-repo/.envrc"
rm -f "$TEST_DIR/target-repo/.vscode-settings.json"

repoverlay restore --target "$TEST_DIR/target-repo" --dry-run
```

**Expected Output:**

- Shows what would be restored without making changes
- Lists files that would be recreated

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.envrc" 2>&1
# Should report "No such file or directory" — dry-run made no changes
```

### TC-03: Restore with --force when conflicting file exists

**Steps:**

```bash
cd "$TEST_DIR"

# First, actually restore the files (from TC-02 they are still missing)
repoverlay restore --target "$TEST_DIR/target-repo"

# Now create a conflicting regular file (not a symlink)
rm -f "$TEST_DIR/target-repo/.envrc"
echo "conflicting content" > "$TEST_DIR/target-repo/.envrc"

# Try restore with --force to overwrite
repoverlay restore --target "$TEST_DIR/target-repo" --force
```

**Expected Output:**

- Success message indicating files restored with force overwrite

**Verify:**

```bash
ls -la "$TEST_DIR/target-repo/.envrc"
# Should be a symlink pointing to the overlay source (overwritten the regular file)

cat "$TEST_DIR/target-repo/.envrc"
# Should contain "export FOO=bar" (overlay content, not "conflicting content")
```

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
