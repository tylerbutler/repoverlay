# Cache - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH

## Setup

Use an isolated root for both config and cache.

```bash
ORIG_PWD="$PWD"
TEST_ROOT="$(mktemp -d "$PWD/.manual-cache-test.XXXXXX")" || exit 1
cleanup() {
  trap - EXIT
  cd "$ORIG_PWD" || exit 1
  rm -rf -- "$TEST_ROOT"
  unset XDG_CONFIG_HOME REPOVERLAY_CACHE_DIR ORIG_PWD TEST_ROOT DEMO_SOURCE_URL DEMO_SOURCE_NAME RETENTION_SOURCE_URL RETENTION_SOURCE_NAME CACHE_OUTPUT
  unset -f cleanup
}

export TEST_ROOT ORIG_PWD
mkdir -p "$TEST_ROOT/config" "$TEST_ROOT/cache" "$TEST_ROOT/repo"
export XDG_CONFIG_HOME="$TEST_ROOT/config"
export REPOVERLAY_CACHE_DIR="$TEST_ROOT/cache"
cd "$TEST_ROOT/repo"
git init >/dev/null || exit 1
trap cleanup EXIT
```

## Test Cases

### TC-01: Show cache path

**Steps:**

```bash
repoverlay cache path
```

**Expected Output:**

- Prints the absolute cache path
- Matches `$REPOVERLAY_CACHE_DIR`

### TC-02: List cached namespaces

**Steps:**

```bash
mkdir -p "$REPOVERLAY_CACHE_DIR/github/OWNER/REPO"
mkdir -p "$REPOVERLAY_CACHE_DIR/sources/overlay"

CACHE_OUTPUT=$(repoverlay cache list)
printf '%s\n' "$CACHE_OUTPUT"
```

**Expected Output:**

- Shows separate `GitHub repositories` and `Configured source clones` sections
- Lists `OWNER/REPO` and `overlay` under the correct section

### TC-03 (Optional, requires network): Create and locate a configured source clone

Set these before running the networked cases:

```bash
: "${DEMO_SOURCE_URL:?Set DEMO_SOURCE_URL to a real public overlay source repo before TC-03}"
case "$DEMO_SOURCE_URL" in
  *OWNER/REPO*|https://example.com/*)
    echo "Set DEMO_SOURCE_URL to a real public overlay source repo before TC-03." >&2
    exit 1
    ;;
esac
```

**Steps:**

```bash
DEMO_SOURCE_NAME="network-source"
repoverlay source add "$DEMO_SOURCE_URL" --name "$DEMO_SOURCE_NAME"
repoverlay browse --no-interactive
CACHE_OUTPUT=$(repoverlay cache list)
printf '%s\n' "$CACHE_OUTPUT"
```

**Expected Output:**

- A source-resolving command creates the configured-source clone
- `cache list` shows `network-source` under `Configured source clones`

### TC-04: Remove a cached configured source clone

**Steps:**

```bash
mkdir -p "$REPOVERLAY_CACHE_DIR/sources/overlay"
repoverlay cache remove --source overlay
```

**Expected Output:**

- `Removed configured source clone overlay from cache.`

### TC-05: Remove a missing configured source clone

**Steps:**

```bash
repoverlay cache remove --source missing
```

**Expected Output:**

- `Configured source clone missing is not cached.`

### TC-06: Remove all cached namespaces with confirmation

**Steps:**

```bash
mkdir -p "$REPOVERLAY_CACHE_DIR/github/OWNER/REPO"
mkdir -p "$REPOVERLAY_CACHE_DIR/sources/overlay"

repoverlay cache remove --all
```

**Expected Output:**

- Prompt: `Remove all cached GitHub repositories and configured source clones? [y/N]`
- Entering `n` prints the cancellation message and removes nothing

### TC-07: Remove all cached namespaces without prompting

**Steps:**

```bash
rm -rf "$REPOVERLAY_CACHE_DIR"
mkdir -p "$REPOVERLAY_CACHE_DIR/github/OWNER/REPO"
mkdir -p "$REPOVERLAY_CACHE_DIR/sources/overlay"

repoverlay cache remove --all --yes
```

**Expected Output:**

- Removes exactly 1 GitHub repository and 1 configured source clone
- Example: `Removed 1 GitHub repository(s) and 1 configured source clone(s).`

### TC-08: Repo-local path sources do not create cache entries

**Steps:**

```bash
mkdir -p "$PWD/overlays/local-src"
repoverlay source add "$PWD/overlays/local-src" --name local-src
repoverlay source list
CACHE_OUTPUT=$(repoverlay cache list)
printf '%s\n' "$CACHE_OUTPUT"
if printf '%s\n' "$CACHE_OUTPUT" | grep -q 'local-src'; then
  echo "unexpected cache entry for local-src" >&2
  exit 1
fi
```

**Expected Output:**

- `source list` includes `local-src`
- `cache list` shows no new configured-source clone for `local-src`

### TC-09: Removing a configured Git source from config leaves its cache entry

**Steps:**

```bash
RETENTION_SOURCE_URL="https://example.invalid/retention/repo"
RETENTION_SOURCE_NAME="retention-source"

repoverlay source add "$RETENTION_SOURCE_URL" --name "$RETENTION_SOURCE_NAME"
mkdir -p "$REPOVERLAY_CACHE_DIR/sources/$RETENTION_SOURCE_NAME"

repoverlay source remove "$RETENTION_SOURCE_NAME"
CACHE_OUTPUT=$(repoverlay cache list)
printf '%s\n' "$CACHE_OUTPUT"
printf '%s\n' "$CACHE_OUTPUT" | grep -q "$RETENTION_SOURCE_NAME"
repoverlay cache remove --source "$RETENTION_SOURCE_NAME"
CACHE_OUTPUT=$(repoverlay cache list)
printf '%s\n' "$CACHE_OUTPUT"
if printf '%s\n' "$CACHE_OUTPUT" | grep -q "$RETENTION_SOURCE_NAME"; then
  echo "unexpected cache entry for $RETENTION_SOURCE_NAME" >&2
  exit 1
fi
```

**Expected Output:**

- `source remove` succeeds but the cached clone remains listed
- `cache remove --source retention-source` removes the cached clone

## Cleanup

```bash
cleanup
```
