# Changie Check Action — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a reusable `changie-check` GitHub Action in `tylerbutler/actions` that detects PR-added changie fragments, renders them via changie CLI, and reports whether entries are required. Consume it in repoverlay's `pr.yml` with sticky PR comments.

**Architecture:** Two-repo change. The reusable composite action installs changie, uses `git diff` to isolate PR-added fragments, removes all others, then runs `changie batch auto --dry-run` to render only the PR's entries. It also inspects commit messages for conventional commit types that require changelog entries. The consuming workflow in repoverlay posts/removes sticky comments using the same `marocchino/sticky-pull-request-comment` pattern already used for semver and binary-size checks.

**Tech Stack:** GitHub Actions (composite), changie CLI via `miniscruff/changie-action@v2.0.0`, bash, `marocchino/sticky-pull-request-comment@v2`

---

## Task 1: Create `changie-check` action in `tylerbutler/actions`

**Repo:** `tylerbutler/actions` (cloned at `/tmp/tylerbutler-actions`)

**Files:**
- Create: `changie-check/action.yml`

### Step 1: Create the action file

Create `changie-check/action.yml` with this content:

```yaml
name: 'Changie Check'
description: 'Detect PR-added changie fragments, render preview, and check if changelog entry is required'

inputs:
  changie-version:
    description: 'Changie CLI version to install'
    required: false
    default: 'latest'
  working-directory:
    description: 'Directory containing .changie.yaml'
    required: false
    default: '.'
  base-sha:
    description: 'Base commit SHA to diff against (typically the PR base)'
    required: true
  head-sha:
    description: 'Head commit SHA (typically the PR head)'
    required: true
  require-for-types:
    description: 'Comma-separated conventional commit types that require a changelog entry'
    required: false
    default: 'feat,fix,refactor,security'

outputs:
  has-entries:
    description: 'Whether the PR adds changie fragments'
    value: ${{ steps.check.outputs.has-entries }}
  preview:
    description: 'Rendered markdown preview of PR-added changelog entries'
    value: ${{ steps.render.outputs.preview }}
  needs-entry:
    description: 'Whether the PR should have a changelog entry but does not'
    value: ${{ steps.require.outputs.needs-entry }}
  commit-types-found:
    description: 'Comma-separated conventional commit types found in PR commits'
    value: ${{ steps.require.outputs.commit-types-found }}

runs:
  using: composite
  steps:
    - name: Install changie
      uses: miniscruff/changie-action@6dcc2533cac0495148ed4046c438487e4dceaa23 # ratchet:miniscruff/changie-action@v2.0.0
      with:
        version: ${{ inputs.changie-version }}

    - name: Detect PR-added fragments
      id: check
      shell: bash
      working-directory: ${{ inputs.working-directory }}
      env:
        BASE_SHA: ${{ inputs.base-sha }}
        HEAD_SHA: ${{ inputs.head-sha }}
      run: |
        # Read changie config to find the unreleased directory
        CHANGES_DIR=$(grep '^changesDir:' .changie.yaml | awk '{print $2}' || echo ".changes")
        UNRELEASED_DIR=$(grep '^unreleasedDir:' .changie.yaml | awk '{print $2}' || echo "unreleased")
        UNRELEASED_PATH="${CHANGES_DIR}/${UNRELEASED_DIR}"

        # Find fragments added in this PR
        PR_FRAGMENTS=$(git diff --name-only --diff-filter=A "${BASE_SHA}...${HEAD_SHA}" -- "${UNRELEASED_PATH}/*.yaml" || true)

        if [ -z "$PR_FRAGMENTS" ]; then
          echo "has-entries=false" >> "$GITHUB_OUTPUT"
          echo "No changie fragments added in this PR"
        else
          echo "has-entries=true" >> "$GITHUB_OUTPUT"
          echo "PR adds changie fragments:"
          echo "$PR_FRAGMENTS"

          # Save the list for later steps
          {
            echo "fragments<<EOF_FRAGMENTS"
            echo "$PR_FRAGMENTS"
            echo "EOF_FRAGMENTS"
          } >> "$GITHUB_OUTPUT"
        fi

        # Export paths for other steps
        echo "unreleased-path=$UNRELEASED_PATH" >> "$GITHUB_OUTPUT"

    - name: Render PR-only preview
      id: render
      if: steps.check.outputs.has-entries == 'true'
      shell: bash
      working-directory: ${{ inputs.working-directory }}
      env:
        PR_FRAGMENTS: ${{ steps.check.outputs.fragments }}
        UNRELEASED_PATH: ${{ steps.check.outputs.unreleased-path }}
      run: |
        # Remove all fragments NOT added in this PR so changie only renders PR entries
        ALL_FRAGMENTS=$(find "$UNRELEASED_PATH" -maxdepth 1 -name '*.yaml' 2>/dev/null || true)
        for f in $ALL_FRAGMENTS; do
          if ! echo "$PR_FRAGMENTS" | grep -qF "$f"; then
            rm "$f"
          fi
        done

        # Render using changie
        set +e
        PREVIEW=$(changie batch auto --dry-run 2>&1)
        EXIT_CODE=$?
        set -e

        if [ $EXIT_CODE -eq 0 ] && [ -n "$PREVIEW" ]; then
          {
            echo "preview<<EOF_PREVIEW"
            echo "$PREVIEW"
            echo "EOF_PREVIEW"
          } >> "$GITHUB_OUTPUT"
        else
          echo "preview=" >> "$GITHUB_OUTPUT"
          echo "::warning::changie batch --dry-run failed or produced empty output"
          echo "$PREVIEW"
        fi

    - name: Check if changelog entry is required
      id: require
      if: steps.check.outputs.has-entries == 'false'
      shell: bash
      env:
        BASE_SHA: ${{ inputs.base-sha }}
        HEAD_SHA: ${{ inputs.head-sha }}
        REQUIRE_TYPES: ${{ inputs.require-for-types }}
      run: |
        # Get conventional commit types from PR commits
        COMMIT_TYPES=$(git log --format='%s' "${BASE_SHA}..${HEAD_SHA}" \
          | grep -oP '^[a-z]+(?=[(!:])'  \
          | sort -u \
          | tr '\n' ',' \
          | sed 's/,$//')

        echo "commit-types-found=$COMMIT_TYPES" >> "$GITHUB_OUTPUT"

        # Check for breaking changes (! prefix)
        HAS_BREAKING=$(git log --format='%s' "${BASE_SHA}..${HEAD_SHA}" \
          | grep -cP '^[a-z]+![(:]' || true)

        # Check if any found types are in the required list
        NEEDS_ENTRY=false
        IFS=',' read -ra REQUIRED <<< "$REQUIRE_TYPES"
        IFS=',' read -ra FOUND <<< "$COMMIT_TYPES"
        for req in "${REQUIRED[@]}"; do
          for found in "${FOUND[@]}"; do
            if [ "$req" = "$found" ]; then
              NEEDS_ENTRY=true
              break 2
            fi
          done
        done

        if [ "$HAS_BREAKING" -gt 0 ]; then
          NEEDS_ENTRY=true
        fi

        echo "needs-entry=$NEEDS_ENTRY" >> "$GITHUB_OUTPUT"
        echo "Commit types found: $COMMIT_TYPES"
        echo "Needs changelog entry: $NEEDS_ENTRY"
```

### Step 2: Commit the action

```bash
cd /tmp/tylerbutler-actions
git add changie-check/action.yml
git commit -m "feat: add changie-check action for PR changelog validation

Detects PR-added changie fragments, renders a preview using
changie batch --dry-run, and checks if a changelog entry is
required based on conventional commit types."
```

### Step 3: Push to a branch and create PR

```bash
git checkout -b feat/changie-check
git push -u origin feat/changie-check
gh pr create --title "feat: add changie-check action" --body "$(cat <<'EOF'
## Summary

- New `changie-check` composite action for PR changelog validation
- Detects PR-added changie fragments via git diff
- Renders preview of only PR-scoped entries using `changie batch auto --dry-run`
- Checks conventional commit types to determine if a changelog entry is required
- Outputs: `has-entries`, `preview`, `needs-entry`, `commit-types-found`

## Test plan

- [ ] Test with a PR that adds a changie fragment — verify `has-entries=true` and preview output
- [ ] Test with a PR that has no fragment but has `feat:` commits — verify `needs-entry=true`
- [ ] Test with a PR that has no fragment and only `chore:` commits — verify `needs-entry=false`
EOF
)"
```

---

## Task 2: Update `tylerbutler/actions` README

**Repo:** `tylerbutler/actions`

**Files:**
- Modify: `README.md`

### Step 1: Add changie-check section to README

Add the following section after the `changie-auto-tag` section in `README.md`:

```markdown
### changie-check

Check PRs for [changie](https://changie.dev/) changelog entries. Detects PR-added fragments, renders a preview, and reports whether a changelog entry is required based on conventional commit types.

\```yaml
- uses: tylerbutler/actions/changie-check@v1
  with:
    base-sha: ${{ github.event.pull_request.base.sha }}
    head-sha: ${{ github.event.pull_request.head.sha }}
\```

**Inputs:**

| Input | Default | Description |
|-------|---------|-------------|
| `changie-version` | `latest` | Changie CLI version to install |
| `working-directory` | `.` | Directory containing `.changie.yaml` |
| `base-sha` | *(required)* | Base commit SHA to diff against |
| `head-sha` | *(required)* | Head commit SHA |
| `require-for-types` | `feat,fix,refactor,security` | Conventional commit types that require a changelog entry |

**Outputs:**

| Output | Description |
|--------|-------------|
| `has-entries` | Whether the PR adds changie fragments |
| `preview` | Rendered markdown preview of PR-added entries |
| `needs-entry` | Whether the PR should have a changelog entry but doesn't |
| `commit-types-found` | Conventional commit types found in PR commits |

**Example (PR comment with preview or missing-entry warning):**

\```yaml
jobs:
  changelog:
    runs-on: ubuntu-latest
    permissions:
      pull-requests: write
    steps:
      - uses: actions/checkout@v6
        with:
          fetch-depth: 0
      - uses: tylerbutler/actions/changie-check@v1
        id: changelog
        with:
          base-sha: ${{ github.event.pull_request.base.sha }}
          head-sha: ${{ github.event.pull_request.head.sha }}
\```
```

### Step 2: Commit

```bash
git add README.md
git commit -m "docs: add changie-check to README"
git push
```

---

## Task 3: Add `changelog` job to repoverlay `pr.yml`

**Repo:** `repoverlay` (json-merging branch)

**Files:**
- Modify: `.github/workflows/pr.yml`

### Step 1: Add the changelog job

Append the following job at the end of `.github/workflows/pr.yml`:

```yaml
  # Changelog entry check
  changelog:
    name: Changelog
    runs-on: ubuntu-latest
    if: github.event.pull_request.draft != true
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # ratchet:actions/checkout@v6
        with:
          fetch-depth: 0
      - uses: tylerbutler/actions/changie-check@84a793038df3b338852564c5986911cbfe45c94b # ratchet:tylerbutler/actions/changie-check@main
        id: changelog
        with:
          base-sha: ${{ github.event.pull_request.base.sha }}
          head-sha: ${{ github.event.pull_request.head.sha }}
      - name: Comment with changelog preview
        if: steps.changelog.outputs.has-entries == 'true'
        uses: marocchino/sticky-pull-request-comment@773744901bac0e8cbb5a0dc842800d45e9b2b405 # ratchet:marocchino/sticky-pull-request-comment@v2
        with:
          header: changelog
          message: |
            ## Changelog Preview

            This PR adds the following changelog entries:

            ${{ steps.changelog.outputs.preview }}
      - name: Comment about missing changelog
        if: steps.changelog.outputs.has-entries == 'false' && steps.changelog.outputs.needs-entry == 'true'
        uses: marocchino/sticky-pull-request-comment@773744901bac0e8cbb5a0dc842800d45e9b2b405 # ratchet:marocchino/sticky-pull-request-comment@v2
        with:
          header: changelog
          message: |
            ## Missing Changelog Entry

            This PR includes commits with types that typically require a changelog entry (`${{ steps.changelog.outputs.commit-types-found }}`), but no changie fragment was found.

            To add one, run:

            ```
            changie new
            ```
      - name: Remove changelog comment
        if: steps.changelog.outputs.has-entries == 'false' && steps.changelog.outputs.needs-entry == 'false'
        uses: marocchino/sticky-pull-request-comment@773744901bac0e8cbb5a0dc842800d45e9b2b405 # ratchet:marocchino/sticky-pull-request-comment@v2
        with:
          header: changelog
          delete: true
```

### Step 2: Commit and push

```bash
git add .github/workflows/pr.yml
git commit -m "ci: add changelog check job to PR workflow

Uses tylerbutler/actions/changie-check to preview PR-added
changelog entries and warn when entries are missing for
feat/fix/refactor/security commits."
git push
```

---

## Task 4: Test end-to-end

### Step 1: Merge changie-check PR in tylerbutler/actions

After the PR from Task 1 is merged, update the ratchet pin in repoverlay's `pr.yml` to point to the new commit on `main`.

### Step 2: Verify on the json-merging PR

Open/update the PR for the `json-merging` branch and confirm:
- The changelog job runs
- The sticky comment shows the preview of the added changie fragment
- The comment renders the "Added" entry for JSON deep merge

### Step 3: Test missing-entry case

Create a test branch with a `feat:` commit but no changie fragment. Open a PR and confirm:
- The changelog job detects `needs-entry=true`
- The sticky comment shows the "Missing Changelog Entry" warning
