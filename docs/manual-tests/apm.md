# APM - Manual Test

## Prerequisites

- `repoverlay`, `git`, and APM 0.31 or later installed and on PATH
- Bash
- Network access for cases marked `(requires network)`

Run in a new shell. Execute the cases in order, then repeat in another new
shell with `HARNESS=copilot` instead of `claude`. The sample package uses a
version tag and contains skills, instructions, agents, and prompts, but no hooks.
Do not add `--allow-hooks` for these cases.

## Setup

```bash
TEST_DIR=$(mktemp -d -t repoverlay-apm-tests.XXXXXX)
export XDG_CONFIG_HOME="$TEST_DIR/config"
export XDG_DATA_HOME="$TEST_DIR/data"
export XDG_CACHE_HOME="$TEST_DIR/cache"
export REPOVERLAY_CACHE_DIR="$TEST_DIR/cache"
export REPOVERLAY_CLAUDE_HOME="$TEST_DIR/claude-home"
export REPOVERLAY_COPILOT_HOME="$TEST_DIR/copilot-home"
export REPOVERLAY_NO_UPDATE_CHECK=1
HARNESS=claude
PACKAGE=microsoft/apm-sample-package#v1.0.0
PROFILE=apm-microsoft-apm-sample-package

mkdir "$TEST_DIR/target-repo"
git -C "$TEST_DIR/target-repo" init
cd "$TEST_DIR/target-repo"
printf 'Disposable test directory: %s\n' "$TEST_DIR"
```

## Test Cases

### TC-01: Install a package and inspect profile-only status (requires network)

```bash
repoverlay apm install "$PACKAGE" --harness "$HARNESS"
test ! -f apm.yml
test ! -d .repoverlay/overlays
repoverlay profile status --harness "$HARNESS"
repoverlay profile list
repoverlay profile show "$PROFILE"
repoverlay status
repoverlay status --name "$PROFILE"
repoverlay status --json --name "$PROFILE"
repoverlay status --quiet
echo "Exit code: $?"
repoverlay status --quiet --name "$PROFILE"
echo "Exit code: $?"
git status --short
```

Expect repoverlay to install the package without creating an APM manifest
or adding a configured profile. Status must include the generated profile,
selected harness, and managed file paths despite the absence of overlay state.
JSON must contain `overlays: []` and one `profiles` entry with files marked
`status: "ok"`. Both quiet commands must return 0 with no text. Git must not
list generated files as untracked. `profile list` must include the generated
profile and applied harness. `profile show` must show its applied state and
managed files without requiring a configured definition.

### TC-02: Reject duplicate installation and harness switching (requires network)

Run after TC-01.

```bash
repoverlay apm install "$PACKAGE" --harness "$HARNESS"
OTHER_HARNESS=copilot
if [ "$HARNESS" = copilot ]; then OTHER_HARNESS=claude; fi
repoverlay apm install "$PACKAGE" --harness "$OTHER_HARNESS"
repoverlay status --name "$PROFILE"
repoverlay profile remove "$PROFILE" --harness "$HARNESS"
repoverlay status --quiet
echo "Exit code: $?"
```

Both install commands must fail and preserve the active profile. Removal must
delete its state and managed output. Expect exit code 1 after removal.

### TC-03: Reject a project with missing inputs

```bash
repoverlay apm sync --harness "$HARNESS"
cat > apm.yml << 'YAML'
name: repoverlay-manual-test
version: 1.0.0
dependencies:
  apm:
    - microsoft/apm-sample-package#v1.0.0
YAML
repoverlay apm sync --harness "$HARNESS"
repoverlay status --quiet --name apm-project
echo "Exit code: $?"
```

The first sync must report a missing manifest. The second must report a missing
lockfile and suggest `apm install`. Neither command may apply a profile.
Expect the quiet command to return 1.

### TC-04: Sync and replace a pinned project (requires network)

Generate the lockfile in a separate disposable project so APM does not deploy
unmanaged files into the target repository.

```bash
mkdir "$TEST_DIR/lock-project"
cp apm.yml "$TEST_DIR/lock-project/apm.yml"
(cd "$TEST_DIR/lock-project" && apm install --target "$HARNESS")
cp "$TEST_DIR/lock-project/apm.lock.yaml" apm.lock.yaml
cp apm.yml "$TEST_DIR/expected.yml"
cp apm.lock.yaml "$TEST_DIR/expected.lock.yaml"
git add apm.yml apm.lock.yaml
git -c user.name=Test -c user.email=test@example.com commit -m "Pin APM test inputs"

repoverlay apm sync --harness "$HARNESS"
repoverlay status
repoverlay status --json --name apm-project
repoverlay status --quiet --name apm-project
echo "Exit code: $?"
repoverlay apm sync --harness "$HARNESS"
repoverlay profile status --harness "$HARNESS"
cmp apm.yml "$TEST_DIR/expected.yml"
cmp apm.lock.yaml "$TEST_DIR/expected.lock.yaml"
git status --short
```

Both sync commands must succeed. Status must show one `apm-project` profile,
its harness, and managed output; JSON file statuses must be `ok`. Expect exit
code 0 from quiet status. Both comparisons must succeed, and Git must report
no changes to the tracked inputs or untracked generated output.

### TC-05: Restore APM output after Git cleanup (requires network)

Run after TC-04, and run cleanup commands only in this disposable repository.

```bash
git clean -ndx
git clean -fdx
repoverlay status --quiet
echo "Exit code: $?"
repoverlay restore
repoverlay status --name apm-project
repoverlay status --json --name apm-project
repoverlay profile remove apm-project --harness "$HARNESS"
repoverlay profile status
repoverlay status --quiet
echo "Exit code: $?"
```

Expect exit code 1 after cleanup removes the profile state. Restore must recover
the profile and managed output from the external snapshot without changing
the pinned manifest or lockfile. JSON file statuses must be `ok`. After removal,
expect no applied profiles and exit code 1.

### TC-06: Remove APM profiles with --all (requires network)

Run after TC-05. The tracked manifest and lockfile remain available for sync.

```bash
repoverlay apm sync --harness "$HARNESS"
test ! -d .repoverlay/overlays
repoverlay remove --all --dry-run
repoverlay status --quiet --name apm-project
echo "Exit code: $?"
repoverlay remove --all
repoverlay profile status
repoverlay status --quiet
echo "Exit code: $?"
repoverlay restore
repoverlay status --quiet
echo "Exit code: $?"
cmp apm.yml "$TEST_DIR/expected.yml"
cmp apm.lock.yaml "$TEST_DIR/expected.lock.yaml"
```

The dry run must list `apm-project` and its harness without changing managed
output or state. Expect exit code 0 before removal. Bulk removal must succeed
even without overlay state and remove the APM profile and its managed output.
Expect exit code 1 after removal and after restore; explicit removal must prevent
restore from reapplying the profile. Both input comparisons must succeed.

## Cleanup

Exit the test shell to restore your environment. Remove only the disposable
directory printed during Setup after you record the results.
