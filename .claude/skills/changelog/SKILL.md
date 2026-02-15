---
name: changelog
description: Create a changie changelog entry for the current changes. Use when asked to create a changelog entry, changie entry, or change log.
---

Create a changelog entry for the current branch's changes:

1. Run `git log main..HEAD --oneline` to see the commit history, then run `git diff main..HEAD` to review the actual code changes. Base your changelog entry on the code diff, not just commit messages
2. Determine the appropriate kind from changie's configured options:
   - **Added** — New features or capabilities
   - **Changed** — Modifications to existing behavior
   - **Deprecated** — Features marked for future removal
   - **Fixed** — Bug fixes
   - **Removed** — Removed features or capabilities
   - **Security** — Security-related fixes
3. Write a YAML file at `.changes/unreleased/<Kind>-<YYYYMMDD>-<HHMMSS>.yaml` with this format:
   ```yaml
   kind: <Kind>
   body: |-
       Short imperative summary of the change

       Optional longer description of what changed and why.
   time: <ISO-8601 timestamp with timezone>
   ```
4. The `body` should start with a short imperative summary (e.g. "Add deep merge for JSON files"), then optionally a blank line and more details
5. Use the current date/time for both the filename timestamp and the `time` field
