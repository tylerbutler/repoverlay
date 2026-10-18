# Profile - Manual Test

## Prerequisites

- `repoverlay` and `git` installed and on PATH
- Bash

Run in a new shell. Execute the cases in order, then repeat in another new
shell with `HARNESS=copilot` instead of `claude`. Neither harness executable
is required.

## Setup

```bash
TEST_DIR=$(mktemp -d -t repoverlay-profile-tests.XXXXXX)
export XDG_CONFIG_HOME="$TEST_DIR/config"
export XDG_DATA_HOME="$TEST_DIR/data"
export XDG_CACHE_HOME="$TEST_DIR/cache"
export REPOVERLAY_CACHE_DIR="$TEST_DIR/cache"
export REPOVERLAY_CLAUDE_HOME="$TEST_DIR/claude-home"
export REPOVERLAY_COPILOT_HOME="$TEST_DIR/copilot-home"
export REPOVERLAY_NO_UPDATE_CHECK=1
HARNESS=claude
CONTEXT=CLAUDE.md
if [ "$HARNESS" = copilot ]; then CONTEXT=AGENTS.md; fi

mkdir -p "$TEST_DIR/target-repo/.repoverlay" "$TEST_DIR/plugin/skills/demo"
printf '# Demo skill\n' > "$TEST_DIR/plugin/skills/demo/SKILL.md"
git -C "$TEST_DIR/target-repo" init
cd "$TEST_DIR/target-repo"
printf '# User context\n' > "$CONTEXT"
cat > .repoverlay/config.ccl << CCL
profiles =
  manual-profile =
    description = Manual test profile
    instructions =
      =
        content = Use concise test responses.
    plugins =
      = $TEST_DIR/plugin
CCL
git add .repoverlay/config.ccl "$CONTEXT"
git -c user.name=Test -c user.email=test@example.com commit -m "Add test inputs"
printf 'Disposable test directory: %s\n' "$TEST_DIR"
```

## Test Cases

### TC-01: Configure and apply a profile

```bash
repoverlay profile list
repoverlay profile show manual-profile
repoverlay profile apply manual-profile --harness "$HARNESS"
repoverlay profile status --harness "$HARNESS"
cat "$CONTEXT"
git status --short
```

Expect `manual-profile` in the configured and applied lists. The context file
must retain `# User context` and contain the profile's instruction in a managed
region. Claude places the skill in `.claude/skills/demo/`; Copilot uses
`.agents/skills/demo/`. Git must not list the generated skill or state files
as untracked. The tracked context file can appear as modified.

### TC-02: Profile-only status in each output mode

```bash
test ! -d .repoverlay/overlays
repoverlay status
repoverlay status --name manual-profile
repoverlay status --json
repoverlay status --json --name manual-profile
repoverlay status --quiet
echo "Exit code: $?"
repoverlay status --quiet --name manual-profile
echo "Exit code: $?"
repoverlay status --quiet --name missing
echo "Exit code: $?"
```

Expect the profile name, selected harness, and managed context and skill paths
in text output. JSON must contain an empty `overlays` array and one entry in
`profiles`, with `name`, `harness`, `mode`, `applied_at`, and `files` fields.
File entries must have `status: "ok"`. Quiet commands must print no text and
return 0, 0, and 1, in that order.

### TC-03: Mixed overlay and profile status

```bash
mkdir "$TEST_DIR/overlay"
printf 'test content\n' > "$TEST_DIR/overlay/.manual-overlay"
repoverlay apply "$TEST_DIR/overlay" --name manual-overlay
repoverlay status
repoverlay status --json
repoverlay status --name manual-profile
repoverlay status --json --name manual-profile
repoverlay status --name manual-overlay
repoverlay status --json --name manual-overlay
repoverlay status --name missing
repoverlay status --json --name missing
```

Unfiltered output must include both entries. Name filters must show only the
matching entry; the other JSON array must be empty. Both missing-name commands
must fail with a `not applied` error and list the available names.

### TC-04: Reject duplicate application and filter by harness

```bash
repoverlay profile apply manual-profile --harness "$HARNESS"
OTHER_HARNESS=copilot
if [ "$HARNESS" = copilot ]; then OTHER_HARNESS=claude; fi
repoverlay profile status --harness "$OTHER_HARNESS"
repoverlay status --name manual-profile
```

Expect duplicate application to fail without changing files or state.
The other harness's profile list must report no applied profiles.
Top-level status must still show the original profile and harness.

### TC-05: Remove a profile without removing an independent overlay

```bash
repoverlay profile remove manual-profile --harness "$HARNESS"
repoverlay profile status
repoverlay status
repoverlay status --quiet --name manual-profile
echo "Exit code: $?"
repoverlay status --quiet
echo "Exit code: $?"
cat "$CONTEXT"
repoverlay remove --all
repoverlay status --quiet
echo "Exit code: $?"
```

Expect no applied profiles, but `manual-overlay` must remain until its removal.
The quiet commands must return 1, 0, and 1. The context file must contain only
the original user context; the profile's skill directory must no longer exist.

### TC-06: Restore a profile after Git cleanup

Run cleanup commands only in this disposable target repository.

```bash
repoverlay profile apply manual-profile --harness "$HARNESS"
git clean -ndx
git clean -fdx
repoverlay status --quiet
echo "Exit code: $?"
repoverlay restore
repoverlay status --name manual-profile
repoverlay status --json --name manual-profile
repoverlay profile remove manual-profile --harness "$HARNESS"
```

Expect exit code 1 after cleanup removes profile state. Restore must recover
the profile and skill from the external snapshot. Its files must have
`status: "ok"`, and the context file must contain one profile-managed region.

### TC-07: Bulk removal with an applied profile and overlay

```bash
repoverlay profile apply manual-profile --harness "$HARNESS"
repoverlay apply "$TEST_DIR/overlay" --name manual-overlay
repoverlay remove --all --dry-run
repoverlay profile status
repoverlay status --quiet
echo "Exit code: $?"
repoverlay remove --all
repoverlay profile status
repoverlay status --quiet
echo "Exit code: $?"
cat "$CONTEXT"
test ! -f .manual-overlay
repoverlay restore
repoverlay status --quiet
echo "Exit code: $?"
```

The dry run must list the profile, its harness, and the overlay without changing
files or state. Expect exit code 0 before removal. Bulk removal must remove both
entries, restore the original user context, and remove the profile's skill.
Expect exit code 1 after removal and after restore; explicit removal must prevent
restore from reapplying either entry. The profile configuration must remain.

### TC-08: Protect a live session from bulk removal

```bash
repoverlay profile apply manual-profile --harness "$HARNESS"
repoverlay apply "$TEST_DIR/overlay" --name manual-overlay
LOCK=".repoverlay/profiles/manual-profile.$HARNESS.lock"
printf '%s\n' "$$" > "$LOCK"
repoverlay remove --all
echo "Exit code: $?"
repoverlay status
test -f .manual-overlay
rm -- "$LOCK"
repoverlay remove --all
repoverlay status --quiet
echo "Exit code: $?"
```

The first removal must fail because the test shell holds a live profile lock.
Both entries and their files must remain. After removing the lock, bulk removal
must succeed. Expect exit code 1 from the failed removal and the final status.

### TC-09: Select profiles and overlays for removal

Run in an interactive terminal.

```bash
repoverlay profile apply manual-profile --harness "$HARNESS"
repoverlay apply "$TEST_DIR/overlay" --name manual-overlay
repoverlay remove --interactive
repoverlay profile status
test -f ".repoverlay/profiles/manual-profile.$HARNESS.ccl"
test -f .repoverlay/config.ccl
repoverlay remove --interactive --dry-run
repoverlay remove --interactive
repoverlay status --quiet
echo "Exit code: $?"
```

In the first menu, select only `manual-overlay`. The profile must remain applied
with its skill and context region intact. In the dry run, select the remaining
profile. It must remain applied. Select that profile in the last menu to remove
it. Expect final exit code 1. The configuration must remain.

## Cleanup

Exit the test shell to restore your environment. Remove only the disposable
directory printed during Setup after you record the results.
