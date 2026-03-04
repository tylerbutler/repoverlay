# Apply - Manual Test

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

# Create a local overlay with a config and some files
mkdir -p my-overlay
cat > my-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = my-overlay
CCL
echo "export FOO=bar" > my-overlay/.envrc
echo '{"editor.tabSize": 2}' > my-overlay/.vscode-settings.json
```

## Test Cases

### TC-01: Apply local overlay (symlink mode, default)

**Steps:**

```bash
cd "$TEST_DIR"
repoverlay apply ./my-overlay --target ./target-repo
```

**Expected Output:**

- Success message indicating overlay applied
- Files listed as symlinked

**Verify:**

```bash
ls -la "$TEST_DIR/target-repo/.envrc"
# Should show a symlink pointing to the overlay source
file "$TEST_DIR/target-repo/.envrc"
# Should indicate "symbolic link"

cat "$TEST_DIR/target-repo/.git/info/exclude"
# Should contain the overlay files in the exclude list

repoverlay status --target "$TEST_DIR/target-repo"
# Should show "my-overlay" as applied
```

### TC-02: Apply with --copy

**Steps:**

```bash
cd "$TEST_DIR"

# Create a fresh target repo
mkdir target-copy && cd target-copy
git init
git commit --allow-empty -m "init"
cd "$TEST_DIR"

repoverlay apply ./my-overlay --target ./target-copy --copy
```

**Expected Output:**

- Success message indicating overlay applied in copy mode

**Verify:**

```bash
ls -la "$TEST_DIR/target-copy/.envrc"
# Should be a regular file, NOT a symlink
file "$TEST_DIR/target-copy/.envrc"
# Should indicate "ASCII text" or similar, not "symbolic link"

cat "$TEST_DIR/target-copy/.envrc"
# Should contain "export FOO=bar"
```

### TC-03: Apply with --name override

**Steps:**

```bash
cd "$TEST_DIR"

mkdir target-named && cd target-named
git init
git commit --allow-empty -m "init"
cd "$TEST_DIR"

repoverlay apply ./my-overlay --target ./target-named --name custom-name
```

**Expected Output:**

- Success message showing overlay applied under "custom-name"

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-named"
# Should show "custom-name" (not "my-overlay") as the applied overlay
```

### TC-04: Apply with --dry-run

**Steps:**

```bash
cd "$TEST_DIR"

mkdir target-dry && cd target-dry
git init
git commit --allow-empty -m "init"
cd "$TEST_DIR"

repoverlay apply ./my-overlay --target ./target-dry --dry-run
```

**Expected Output:**

- Shows what would be applied without making changes
- Lists files that would be created

**Verify:**

```bash
ls "$TEST_DIR/target-dry/.envrc" 2>&1
# Should report "No such file or directory" — file was NOT created

repoverlay status --target "$TEST_DIR/target-dry" --quiet
echo "Exit code: $?"
# Exit code should be 1 (no overlays applied)
```

### TC-05: Apply with --force to re-apply same overlay

**Steps:**

```bash
cd "$TEST_DIR"

# Use target-repo which already has my-overlay applied from TC-01
repoverlay apply ./my-overlay --target ./target-repo --force
```

**Expected Output:**

- Success message indicating overlay re-applied (force overwrite)

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-repo"
# Should still show "my-overlay" as applied
ls -la "$TEST_DIR/target-repo/.envrc"
# Symlink should still be valid
```

### TC-06: Apply with --skip-conflicts when target has conflicting file

**Steps:**

```bash
cd "$TEST_DIR"

mkdir target-skip && cd target-skip
git init
git commit --allow-empty -m "init"

# Create a conflicting file in the target repo
echo "existing content" > .envrc
git add .envrc && git commit -m "add envrc"
cd "$TEST_DIR"

repoverlay apply ./my-overlay --target ./target-skip --skip-conflicts
```

**Expected Output:**

- Message indicating `.envrc` was skipped due to conflict
- Non-conflicting files (like `.vscode-settings.json`) still applied

**Verify:**

```bash
cat "$TEST_DIR/target-skip/.envrc"
# Should still contain "existing content" (the original, not overwritten)

ls "$TEST_DIR/target-skip/.vscode-settings.json"
# Should exist (non-conflicting file was applied)
```

### TC-07: Apply with --merge on JSON files

**Steps:**

```bash
cd "$TEST_DIR"

mkdir target-merge && cd target-merge
git init
git commit --allow-empty -m "init"

# Create a JSON file in the target that will be merged
mkdir -p .vscode
echo '{"editor.fontSize": 14}' > .vscode/settings.json
git add . && git commit -m "add vscode settings"
cd "$TEST_DIR"

# Create an overlay with a JSON file that should merge
mkdir -p merge-overlay/.vscode
echo '{"editor.tabSize": 2}' > merge-overlay/.vscode/settings.json
cat > merge-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = merge-overlay
CCL

repoverlay apply ./merge-overlay --target ./target-merge --merge
```

**Expected Output:**

- Success message indicating overlay applied with merge

**Verify:**

```bash
cat "$TEST_DIR/target-merge/.vscode/settings.json"
# Should contain both keys: editor.fontSize and editor.tabSize
# (deep merged result)
```

### TC-08 (Optional, requires network): Apply from GitHub URL

**Steps:**

```bash
cd "$TEST_DIR"

mkdir target-github && cd target-github
git init
git commit --allow-empty -m "init"
cd "$TEST_DIR"

# Replace with a real public overlay repository URL
repoverlay apply https://github.com/OWNER/REPO --target ./target-github
```

**Expected Output:**

- Clones the repo, applies overlay files

**Verify:**

```bash
repoverlay status --target "$TEST_DIR/target-github"
# Should show the GitHub overlay as applied
```

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
