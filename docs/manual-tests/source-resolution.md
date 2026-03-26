# Source Resolution - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH
- `git` installed

## Setup

```bash
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"

# Create a target git repository with fork + upstream remotes so upstream fallback
# can be exercised without any network access.
mkdir target-repo && cd target-repo
git init
git config user.email "manual-tests@example.com"
git config user.name "Manual Tests"
git commit --allow-empty -m "init"
git remote add origin git@github.com:fork-org/fork-repo.git
git remote add upstream git@github.com:upstream-org/upstream-repo.git

# Create an overlay that will be imported into the per-repo library.
mkdir -p library-source
cat > library-source/repoverlay.ccl << 'CCL'
overlay =
  name = shared-name
CCL
echo "from library" > library-source/resolution.txt

# Create a same-name local directory to show that bare-name resolution still
# prefers the library when a matching library overlay exists.
mkdir -p shared-name
cat > shared-name/repoverlay.ccl << 'CCL'
overlay =
  name = shared-name
CCL
echo "from ambiguous local path" > shared-name/resolution.txt

# Create two repo-local sources with the same structured overlay so priority
# order and --from can be tested locally.
mkdir -p sources/01-primary/acme/widgets/shared-overlay
cat > sources/01-primary/acme/widgets/shared-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = shared-overlay
CCL
echo "PRIMARY" > sources/01-primary/acme/widgets/shared-overlay/priority.txt

mkdir -p sources/02-secondary/acme/widgets/shared-overlay
cat > sources/02-secondary/acme/widgets/shared-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = shared-overlay
CCL
echo "SECONDARY" > sources/02-secondary/acme/widgets/shared-overlay/priority.txt

# Create a source that only contains the upstream org/repo path. The target repo
# will request the fork path and should fall back via its configured upstream remote.
mkdir -p sources/03-inheritance/upstream-org/upstream-repo/fork-overlay
cat > sources/03-inheritance/upstream-org/upstream-repo/fork-overlay/repoverlay.ccl << 'CCL'
overlay =
  name = fork-overlay
CCL
echo "UPSTREAM" > sources/03-inheritance/upstream-org/upstream-repo/fork-overlay/inheritance.txt

cd "$TEST_DIR"
```

## Test Cases

### TC-01: Bare-name resolution prefers the library over an ambiguous local directory

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay library import ./library-source --target . --name shared-name
repoverlay apply shared-name --target .
```

**Expected Output:**

- `library import` succeeds and stores `shared-name` in `.repoverlay/library/`
- `apply shared-name` succeeds
- The applied overlay resolves from the library, not from the `./shared-name` directory

**Verify:**

```bash
cat "$TEST_DIR/target-repo/resolution.txt"
# Should contain "from library"

readlink "$TEST_DIR/target-repo/resolution.txt"
# Should point inside ".repoverlay/library/shared-name"

repoverlay status --target "$TEST_DIR/target-repo"
# Should show "shared-name" and a library source
```

### TC-02: `--from @library` explicitly selects the built-in library source

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay remove shared-name --target .
repoverlay apply shared-name --target . --from @library --name shared-name-explicit
```

**Expected Output:**

- `remove` succeeds and clears the first application
- `apply` succeeds using the built-in `@library` source
- No configured source lookup is needed for this resolution

**Verify:**

```bash
cat "$TEST_DIR/target-repo/resolution.txt"
# Should contain "from library"

readlink "$TEST_DIR/target-repo/resolution.txt"
# Should still point inside ".repoverlay/library/shared-name"

repoverlay status --target "$TEST_DIR/target-repo"
# Should show "shared-name-explicit" and a library source
```

### TC-03: First match wins across multiple configured local sources

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay source add ./sources/01-primary --name primary
repoverlay source add ./sources/02-secondary --name secondary
repoverlay source list
repoverlay apply acme/widgets/shared-overlay --target . --name priority-default
```

**Expected Output:**

- Both local sources are added successfully
- `source list` shows `primary` before `secondary`
- `apply` succeeds and resolves `acme/widgets/shared-overlay` from the first matching source

**Verify:**

```bash
cat "$TEST_DIR/target-repo/priority.txt"
# Should contain "PRIMARY"

repoverlay status --target "$TEST_DIR/target-repo"
# Should show "priority-default"
# Source details should include "From: primary"
```

### TC-04: `--from` bypasses priority order and selects the named source

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay remove priority-default --target .
repoverlay apply acme/widgets/shared-overlay --target . --from secondary --name priority-secondary
```

**Expected Output:**

- `remove` succeeds
- `apply` succeeds even though `primary` still has a matching overlay
- The named source filter causes resolution to come from `secondary`

**Verify:**

```bash
cat "$TEST_DIR/target-repo/priority.txt"
# Should contain "SECONDARY"

repoverlay status --target "$TEST_DIR/target-repo"
# Should show "priority-secondary"
# Source details should include "From: secondary"
```

### TC-05: Fork-targeted lookup falls back to the upstream org/repo when available

**Steps:**

```bash
cd "$TEST_DIR/target-repo"

repoverlay source add ./sources/03-inheritance --name inherited
repoverlay apply fork-org/fork-repo/fork-overlay --target . --from inherited --name upstream-fallback
```

**Expected Output:**

- The source is added successfully
- `apply` succeeds even though the source only contains
  `upstream-org/upstream-repo/fork-overlay`
- Resolution reports upstream fallback because the target repo has an `upstream` remote

**Verify:**

```bash
cat "$TEST_DIR/target-repo/inheritance.txt"
# Should contain "UPSTREAM"

repoverlay status --target "$TEST_DIR/target-repo"
# Should show "upstream-fallback"
# Source details should include "upstream-org/upstream-repo/fork-overlay (via upstream)"
# Source details should include "From: inherited"
```

## Cleanup

```bash
rm -rf "$TEST_DIR"
```
