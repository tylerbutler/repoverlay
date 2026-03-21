# Create - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH
- `git` installed

## Setup

```bash
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"

# Create a source repository with files to extract into an overlay
mkdir source-repo && cd source-repo
git init
git commit --allow-empty -m "init"

# Add some configuration files to extract
echo "export FOO=bar" > .envrc
mkdir -p .vscode
echo '{"editor.tabSize": 2}' > .vscode/settings.json
echo "node_modules/" > .gitignore
git add . && git commit -m "add config files"
cd "$TEST_DIR"

# Create output directory for overlays
mkdir overlay-output

# Create a target repo for library-based create flows
mkdir target-repo && cd target-repo
git init
git commit --allow-empty -m "init"
cd "$TEST_DIR"
```

## Test Cases

### TC-01: Create overlay with --output to local directory

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay create my-overlay --source ./source-repo --output ./overlay-output --yes
```

**Expected Output:**

- Success message indicating overlay created
- Lists the files included in the overlay

**Verify:**

```bash
ls "$TEST_DIR/overlay-output/my-overlay/"
# Should contain overlay files

cat "$TEST_DIR/overlay-output/my-overlay/repoverlay.ccl"
# Should contain overlay configuration with name "my-overlay"

ls "$TEST_DIR/overlay-output/my-overlay/.envrc"
# Should exist (file extracted from source repo)
```

### TC-02: Create with --include to select specific files

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay create include-overlay --source ./source-repo --output ./overlay-output --include .envrc --include .gitignore --yes
```

**Expected Output:**

- Success message indicating overlay created with only specified files

**Verify:**

```bash
ls "$TEST_DIR/overlay-output/include-overlay/"
# Should contain repoverlay.ccl, .envrc, and .gitignore

ls "$TEST_DIR/overlay-output/include-overlay/.envrc"
# Should exist

ls "$TEST_DIR/overlay-output/include-overlay/.gitignore"
# Should exist

ls "$TEST_DIR/overlay-output/include-overlay/.vscode" 2>&1
# Should report "No such file or directory" — not included
```

### TC-03: Create with --dry-run

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay create dry-overlay --source ./source-repo --output ./overlay-output --yes --dry-run
```

**Expected Output:**

- Shows what files would be included in the overlay
- Does NOT create any files

**Verify:**

```bash
ls "$TEST_DIR/overlay-output/dry-overlay" 2>&1
# Should report "No such file or directory" — nothing was created
```

### TC-04: Create with --yes to skip prompts

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay create auto-overlay --source ./source-repo --output ./overlay-output --yes
```

**Expected Output:**

- Overlay created without any interactive prompts
- Success message displayed

**Verify:**

```bash
ls "$TEST_DIR/overlay-output/auto-overlay/repoverlay.ccl"
# Should exist (overlay was created non-interactively)
```

### TC-05: Create with --force to overwrite existing

**Steps:**

```bash
cd "$TEST_DIR"

# First, create an overlay
repoverlay create force-overlay --source ./source-repo --output ./overlay-output --yes

# Modify the source
echo "updated content" > "$TEST_DIR/source-repo/.envrc"

# Re-create with --force to overwrite
repoverlay create force-overlay --source ./source-repo --output ./overlay-output --yes --force
```

**Expected Output:**

- Success message indicating overlay overwritten

**Verify:**

```bash
cat "$TEST_DIR/overlay-output/force-overlay/.envrc"
# Should contain "updated content" (the overwritten version)
```

### TC-06: Create directly into the in-repo library without applying

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay create library-only --source ../source-repo --include .envrc --into library --no-apply --yes
```

**Expected Output:**

- Success message indicating the overlay was created in the library
- No overlay is applied to the target repo because `--no-apply` was used

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.repoverlay/library/library-only/.envrc"
# Should exist in the library

repoverlay status --target "$TEST_DIR/target-repo" --quiet
echo "Exit code: $?"
# Should be 1 — no overlays are currently applied
```

### TC-07: Create into the library and apply by default

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay create applied-library --source ../source-repo --include .envrc --into library --yes
```

**Expected Output:**

- Success message indicating the overlay was created
- The newly created library overlay is applied automatically

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.repoverlay/library/applied-library/.envrc"
# Should exist in the library

cat "$TEST_DIR/target-repo/.envrc"
# Should contain "export FOO=bar"

repoverlay status --target "$TEST_DIR/target-repo"
# Should show "applied-library" as the applied overlay
```

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
