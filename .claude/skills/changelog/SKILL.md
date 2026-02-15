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
3. If the change is specific to a single command/subcommand, set the `component` field to the command name. Check `.changie.yaml` for the list of valid components. Leave `component` off for cross-cutting changes.
4. Write a YAML file at `.changes/unreleased/<Kind>-<YYYYMMDD>-<HHMMSS>.yaml` with this format:
   ```yaml
   kind: <Kind>
   body: |-
       Short imperative summary of the change

       Optional longer description of what changed and why.
   time: <ISO-8601 timestamp with timezone>
   component: <command-name>  # optional, only for command-specific changes
   ```
5. The `body` should start with a short imperative summary (e.g. "Add deep merge for JSON files"), then optionally a blank line and more details
6. Use the current date/time for both the filename timestamp and the `time` field
7. Preview the rendered output with `changie batch --dry-run auto` to verify formatting

## Changie configuration reference

The heading hierarchy in `.changie.yaml` is:

| Level | Format key | Rendered as |
|-------|-----------|-------------|
| Version | `versionFormat` | `##` (h2) |
| Component | `componentFormat` | `###` (h3) |
| Kind | `kindFormat` | `####` (h4) |
| Change | `changeFormat` | `-` bullet |

Key design decisions:
- **Components are outer, kinds are inner.** Changie renders component headers first, then kind headers nested underneath. This means `componentFormat` must use a higher heading level than `kindFormat`.
- **Empty component guard.** The `componentFormat` template uses `{{if .Component}}...{{end}}` so changes without a component skip the component heading entirely.
- **Component display.** Components render as `### Command: \`name\`` with backtick code style.
- **Bullet changes.** Changes use bullet points (`-`) instead of headings to avoid going to h5 depth.
- **Newlines.** `beforeComponent`/`afterComponent` control spacing around component sections.
