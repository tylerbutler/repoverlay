# Edit - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH
- `git` installed

## Setup

> **Note:** `edit add` currently requires the target repository to have a git
> remote origin, even for locally-applied overlays. See
> [#204](https://github.com/tylerbutler/repoverlay/issues/204). The setup
> below includes a fake remote to work around this.

```bash
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"

# Create a target git repository with some existing files
mkdir target-repo && cd target-repo
git init
git remote add origin https://github.com/example/repo.git
git commit --allow-empty -m "init"
echo "app config" > .app-config
echo "editor prefs" > .editorconfig
git add . && git commit -m "add config files"
cd "$TEST_DIR"

# Create a local overlay with one file
mkdir -p my-overlay
cat > my-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = my-overlay
CCL
echo "overlay env" > my-overlay/.envrc

# Apply the overlay to the target repo
repoverlay apply ./my-overlay --target ./target-repo
```

## Test Cases

### TC-01: Add a file to an applied overlay

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay edit add my-overlay .app-config --target ./target-repo
```

**Expected Output:**

- Success message indicating file added to overlay
- File in target repo replaced with symlink to overlay source

**Verify:**

```bash
ls -la "$TEST_DIR/target-repo/.app-config"
# Should be a symlink pointing to the overlay source

ls "$TEST_DIR/my-overlay/.app-config"
# Should exist in the overlay source directory

repoverlay status --target "$TEST_DIR/target-repo"
# Should show .app-config as part of my-overlay's files
```

### TC-02: Add multiple files to an applied overlay

**Steps:**

```bash
cd "$TEST_DIR"

# Create additional files in the target repo
echo "tool versions" > "$TEST_DIR/target-repo/.tool-versions"
echo "prettierrc" > "$TEST_DIR/target-repo/.prettierrc"

repoverlay edit add my-overlay .tool-versions .prettierrc --target ./target-repo
```

**Expected Output:**

- Success message indicating both files added to overlay

**Verify:**

```bash
ls "$TEST_DIR/my-overlay/.tool-versions"
# Should exist in overlay source

ls "$TEST_DIR/my-overlay/.prettierrc"
# Should exist in overlay source

ls -la "$TEST_DIR/target-repo/.tool-versions"
# Should be a symlink

ls -la "$TEST_DIR/target-repo/.prettierrc"
# Should be a symlink
```

### TC-03: Remove a file from an applied overlay

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay edit remove my-overlay .prettierrc --target ./target-repo
```

**Expected Output:**

- Success message indicating file removed from overlay

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.prettierrc" 2>&1
# Should report "No such file or directory" — symlink removed from target

repoverlay status --target "$TEST_DIR/target-repo"
# Should NOT list .prettierrc in my-overlay's files

ls "$TEST_DIR/my-overlay/.prettierrc"
# File is preserved in the overlay source directory (not deleted)
```

### TC-04: Add with --dry-run

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay edit add my-overlay .editorconfig --target ./target-repo --dry-run
```

**Expected Output:**

- Shows what would be added without making changes

**Verify:**

```bash
ls -la "$TEST_DIR/target-repo/.editorconfig"
# Should still be a regular file, NOT a symlink

ls "$TEST_DIR/my-overlay/.editorconfig" 2>&1
# Should report "No such file or directory" — not copied to overlay source
```

### TC-05: Remove with --dry-run

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay edit remove my-overlay .tool-versions --target ./target-repo --dry-run
```

**Expected Output:**

- Shows what would be removed without making changes

**Verify:**

```bash
ls "$TEST_DIR/my-overlay/.tool-versions"
# Should still exist in overlay source

ls -la "$TEST_DIR/target-repo/.tool-versions"
# Should still be a symlink — dry-run made no changes
```

### TC-06: Add a file that doesn't exist in target repo

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay edit add my-overlay nonexistent.txt --target ./target-repo
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating file does not exist
- Non-zero exit code

### TC-07: Remove a file not managed by the overlay

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay edit remove my-overlay not-in-overlay.txt --target ./target-repo
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating file is not managed by the overlay
- Non-zero exit code

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
