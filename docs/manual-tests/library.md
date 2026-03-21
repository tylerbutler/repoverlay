# Library - Manual Test

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
git config user.email "manual-tests@example.com"
git config user.name "Manual Tests"
git commit --allow-empty -m "init"

# Simulate a repo that currently ignores .repoverlay/
printf ".repoverlay/\n" > .gitignore
cd "$TEST_DIR"

# Create an overlay to import by filesystem path
mkdir -p path-overlay/.config/repoverlay
cat > path-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = path-overlay
CCL
echo "export PATH_OVERLAY=1" > path-overlay/.envrc
echo "path overlay config" > path-overlay/.config/repoverlay/config.txt

# Create a second overlay that will be applied, then imported by applied name
mkdir -p applied-source
cat > applied-source/repoverlay.ccl << 'CCL'
overlay =
  name = applied-source
CCL
echo "from applied overlay" > applied-source/CLAUDE.md
echo '{"theme":"dark"}' > applied-source/.vscode-settings.json
```

## Test Cases

### TC-01: library list with an empty library

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay library list --target ./target-repo
```

**Expected Output:**

- Prints `No overlays in library.`

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.repoverlay/library" 2>&1
# Should report "No such file or directory" — list does not create the library
```

### TC-02: library import from a local path

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay library import ./path-overlay --target ./target-repo --name team-library
```

**Expected Output:**

- Success message indicating `team-library` was imported into the library
- If `.repoverlay/` is gitignored, a note indicates `.gitignore` was updated to track the library path

**Verify:**

```bash
repoverlay library list --target "$TEST_DIR/target-repo"
# Should show "team-library"

cat "$TEST_DIR/target-repo/.repoverlay/library/team-library/.envrc"
# Should contain "export PATH_OVERLAY=1"

cat "$TEST_DIR/target-repo/.gitignore"
# Should contain ".repoverlay/*" and "!.repoverlay/library/"
# Should NOT still contain a plain ".repoverlay/" line by itself
```

### TC-03: library export to a local directory

**Steps:**

```bash
cd "$TEST_DIR"

mkdir -p exported
repoverlay library export team-library --to ./exported --target ./target-repo
```

**Expected Output:**

- Success message indicating `team-library` was exported to `./exported/team-library`

**Verify:**

```bash
cat "$TEST_DIR/exported/team-library/.envrc"
# Should contain "export PATH_OVERLAY=1"

cat "$TEST_DIR/exported/team-library/.config/repoverlay/config.txt"
# Should contain "path overlay config"
```

### TC-04: library remove

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay library remove team-library --target ./target-repo
```

**Expected Output:**

- Success message indicating `team-library` was removed from the library

**Verify:**

```bash
ls "$TEST_DIR/target-repo/.repoverlay/library/team-library" 2>&1
# Should report "No such file or directory"

repoverlay library list --target "$TEST_DIR/target-repo"
# Should NOT show "team-library"
```

### TC-05: library import by applied overlay name

**Steps:**

```bash
cd "$TEST_DIR"

repoverlay apply ./applied-source --target ./target-repo --name applied-overlay
repoverlay library import applied-overlay --target ./target-repo
```

**Expected Output:**

- `apply` succeeds and records `applied-overlay` as applied to the target repo
- `library import` succeeds without needing the original source path

**Verify:**

```bash
repoverlay library list --target "$TEST_DIR/target-repo"
# Should show "applied-overlay"

cat "$TEST_DIR/target-repo/.repoverlay/library/applied-overlay/CLAUDE.md"
# Should contain "from applied overlay"

repoverlay status --target "$TEST_DIR/target-repo"
# Should still show "applied-overlay" as applied to the target repo
```

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
