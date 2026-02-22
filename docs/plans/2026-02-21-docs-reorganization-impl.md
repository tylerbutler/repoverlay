# Docs Reorganization & Source UX Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reorganize the documentation site into task-based guides, add browse-without-source and apply-with-source-prompt features, and fix stale/incorrect content.

**Architecture:** Two independent workstreams — Rust feature changes (`browse` accepts a source argument, `apply` prompts to save sources) and website docs rewrite (drop Concepts section, consolidate into 4 guides + How It Works). Feature changes land first so docs can reference the new behavior.

**Tech Stack:** Rust (clap CLI), Astro/Starlight (docs site), Markdown

---

### Task 1: Add optional source argument to `browse` command

**Files:**
- Modify: `src/cli.rs:351-377` (Browse command struct)
- Modify: `src/cli.rs:756-772` (Browse handler)
- Modify: `src/cli.rs:1239-1366` (`browse_overlays` function)

**Step 1: Add `source` positional argument to Browse**

In `src/cli.rs`, add an optional positional `source` argument to the `Browse` variant:

```rust
    Browse {
        /// Overlay source (GitHub username, owner/repo, or URL)
        ///
        /// Browse overlays from this source without adding it as a configured source.
        /// If omitted, uses configured sources.
        #[arg(value_name = "SOURCE")]
        source: Option<String>,

        // ... existing fields unchanged ...
    },
```

**Step 2: Pass `source` through the handler**

Update the `Commands::Browse` match arm at line ~756 to pass `source`:

```rust
        Commands::Browse {
            source,
            filter,
            update,
            target,
            no_interactive,
            dry_run,
            show_all,
        } => {
            browse_overlays(
                source.as_deref(),
                filter.as_deref(),
                update,
                target,
                no_interactive,
                dry_run,
                show_all,
            )?;
        }
```

**Step 3: Update `browse_overlays` to handle ephemeral source**

Add `source: Option<&str>` as the first parameter of `browse_overlays`. When present, fetch the repo directly (like `resolve_two_part` does) instead of requiring a configured source. The key change is at the top of the function — replace the `config.get_default_overlay_repo_config()?` call with a branch:

```rust
fn browse_overlays(
    source: Option<&str>,
    target_filter: Option<&str>,
    update: bool,
    target: Option<PathBuf>,
    no_interactive: bool,
    dry_run: bool,
    show_all: bool,
) -> Result<()> {
    use crate::config::load_config;
    use crate::overlay_repo::OverlayRepoManager;
    use crate::selection::{FlatSelectionConfig, SelectableItem, select_flat};
    use crate::state::{OverlaySource, normalize_overlay_name};

    // If a source argument was provided, use ephemeral browse mode
    if let Some(source_str) = source {
        return browse_ephemeral_source(
            source_str, target_filter, update, target, no_interactive, dry_run, show_all,
        );
    }

    // Otherwise, use configured sources (existing behavior)
    let config = load_config(None)?;
    let overlay_config = config.get_default_overlay_repo_config()?;
    // ... rest unchanged ...
```

The `browse_ephemeral_source` function should:
1. Parse the source string using `SourceReference::parse`
2. For `OnePart` (username): expand to `username/{default_repo}`, fetch via `CacheManager`
3. For `TwoPart` (owner/repo): fetch via `CacheManager`
4. For `GitHubUrl`: fetch via `CacheManager`
5. List overlays from the cached repo using `list_overlays_from_cached_repo`
6. Continue with the same interactive selection + apply logic as the existing `browse_overlays`

**Step 4: Write tests**

Add tests to the CLI parse tests section:

```rust
#[test]
fn browse_parses_source_argument() {
    let cli = Cli::try_parse_from(["repoverlay", "browse", "tylerbutler"]).unwrap();
    match cli.command {
        Some(Commands::Browse { source, .. }) => {
            assert_eq!(source, Some("tylerbutler".to_string()));
        }
        _ => panic!("Expected Browse command"),
    }
}

#[test]
fn browse_source_is_optional() {
    let cli = Cli::try_parse_from(["repoverlay", "browse"]).unwrap();
    match cli.command {
        Some(Commands::Browse { source, .. }) => {
            assert!(source.is_none());
        }
        _ => panic!("Expected Browse command"),
    }
}

#[test]
fn browse_source_with_other_flags() {
    let cli = Cli::try_parse_from(["repoverlay", "browse", "tylerbutler", "--show-all"]).unwrap();
    match cli.command {
        Some(Commands::Browse { source, show_all, .. }) => {
            assert_eq!(source, Some("tylerbutler".to_string()));
            assert!(show_all);
        }
        _ => panic!("Expected Browse command"),
    }
}
```

**Step 5: Run tests**

Run: `just test`
Expected: All tests pass including new ones.

**Step 6: Commit**

```
feat(browse): allow browsing without a configured source

browse now accepts an optional source argument (username, owner/repo,
or GitHub URL) to fetch and browse overlays without adding a persistent
source. Existing behavior (using configured sources) is unchanged when
no argument is provided.
```

---

### Task 2: Add source-save prompt to `apply` for username/two-part references

**Files:**
- Modify: `src/lib.rs:440-570` (`resolve_two_part` function)
- Modify: `src/config.rs` (need `add_source` helper or reuse existing logic)

**Step 1: Add a prompt after successful apply in `resolve_two_part`**

The prompt should happen after overlay selection but before returning the resolved sources. In `resolve_two_part`, after the interactive selection succeeds (around line 527), check if the source is already configured and prompt if not:

```rust
    // After selection, prompt to save source if not already configured
    prompt_save_source(owner, repo)?;
```

Create a helper function:

```rust
/// Prompt the user to save a source for future use, if not already configured.
fn prompt_save_source(owner: &str, repo: &str) -> Result<()> {
    use crate::config::{load_config, save_config, Source};

    let config = load_config(None)?;
    let url = format!("https://github.com/{owner}/{repo}");

    // Check if already configured
    if config.sources.iter().any(|s| s.url == url) {
        return Ok(());
    }

    if !is_interactive() {
        return Ok(());
    }

    // Prompt
    let prompt = format!("Save {owner}/{repo} as a source for future use?");
    let save = dialoguer::Confirm::new()
        .with_prompt(&prompt)
        .default(true)
        .interact()?;

    if save {
        let mut config = load_config(None)?;
        let source_name = repo.to_string();

        // Avoid duplicate names
        if config.sources.iter().any(|s| s.name == source_name) {
            return Ok(());
        }

        config.sources.push(Source {
            name: source_name.clone(),
            url: url.clone(),
        });
        save_config(&config)?;

        println!(
            "{} source '{}' ({})",
            "Saved".green().bold(),
            source_name,
            url
        );
    }

    Ok(())
}
```

**Step 2: Write tests**

Test that the prompt function doesn't error when source already exists, and that in non-interactive mode it silently skips. Integration testing the interactive prompt itself is harder; focus on the config-checking logic.

**Step 3: Run tests**

Run: `just test`
Expected: All tests pass.

**Step 4: Commit**

```
feat(apply): prompt to save source on first use

When apply resolves a username or owner/repo reference for the first
time, prompt the user to save it as a configured source. Skips the
prompt if the source is already configured or in non-interactive mode.
```

---

### Task 3: Regenerate CLI reference

**Files:**
- Modify: `docs/cli-reference.md`

**Step 1: Regenerate the CLI reference**

Check how the CLI reference is currently generated:

```bash
grep -r "clap-markdown\|cli-reference\|markdown" justfile Cargo.toml
```

If there's a just recipe or build script, run it. Otherwise, check if `clap-markdown` is a dependency and run the appropriate command. The file header says it was generated by `clap-markdown`.

**Step 2: Verify the output includes `browse` with the new `source` argument**

**Step 3: Commit**

```
docs: regenerate CLI reference
```

---

### Task 4: Fix landing page typo and update banner

**Files:**
- Modify: `website/src/content/docs/index.mdx:19`

**Step 1: Fix the typo**

Change:
```
Reoverlay 0.7.0 is out now!
```
To:
```
repoverlay 0.7.0 is out now!
```

**Step 2: Commit**

```
fix(docs): fix repoverlay typo in landing page banner
```

---

### Task 5: Update sidebar config and remove Concepts section

**Files:**
- Modify: `website/astro.config.mjs:55-85` (sidebar)
- Delete: `website/src/content/docs/concepts/how-overlays-work.md`
- Delete: `website/src/content/docs/concepts/sources.md`
- Delete: `website/src/content/docs/concepts/configuration.md`
- Delete: `website/src/content/docs/concepts/overlay-repos.md`
- Delete: `website/src/content/docs/concepts/fork-inheritance.md`

**Step 1: Update the sidebar in `astro.config.mjs`**

Replace the sidebar array (lines 55-85) with:

```javascript
sidebar: [
    {
        label: "Start Here",
        items: [
            {
                label: "What is repoverlay?",
                slug: "introduction",
            },
            {
                label: "Installation",
                slug: "installation",
            },
            {
                label: "Quick Start",
                slug: "quick-start",
            },
        ],
    },
    {
        label: "Guides",
        items: [
            {
                label: "Applying Overlays",
                slug: "guides/applying",
            },
            {
                label: "Creating & Sharing",
                slug: "guides/creating",
            },
            {
                label: "Managing Applied Overlays",
                slug: "guides/managing",
            },
            {
                label: "Restoring After Git Clean",
                slug: "guides/restoring",
            },
            {
                label: "How It Works",
                slug: "guides/how-it-works",
            },
        ],
    },
    {
        label: "CLI Reference",
        slug: "cli-reference",
    },
],
```

**Step 2: Delete the concepts directory**

```bash
rm -f website/src/content/docs/concepts/how-overlays-work.md
rm -f website/src/content/docs/concepts/sources.md
rm -f website/src/content/docs/concepts/configuration.md
rm -f website/src/content/docs/concepts/overlay-repos.md
rm -f website/src/content/docs/concepts/fork-inheritance.md
rmdir website/src/content/docs/concepts/
```

**Step 3: Delete the old guide pages that are being consolidated**

```bash
rm -f website/src/content/docs/guides/managing-files.md
rm -f website/src/content/docs/guides/updating.md
rm -f website/src/content/docs/guides/switching.md
rm -f website/src/content/docs/guides/cache.md
```

**Step 4: Commit**

```
docs: restructure sidebar, remove Concepts section

Drop the separate Concepts section. Content will be consolidated
into task-based Guides pages. Remove old guide pages that are being
replaced (managing-files, updating, switching, cache).
```

---

### Task 6: Rewrite Quick Start

**Files:**
- Modify: `website/src/content/docs/quick-start.mdx`

**Step 1: Rewrite the Quick Start using tylerbutler as example**

```mdx
---
title: Quick Start
---

import { Steps } from '@astrojs/starlight/components';

Get up and running with repoverlay in under two minutes.

<Steps>

1. **Install repoverlay**

   See [Installation](/installation/) for all options. The quickest way:

   ```bash
   # macOS/Linux
   brew install tylerbutler/tap/repoverlay

   # Or with cargo
   cargo binstall repoverlay
   ```

2. **Check that nothing is applied yet**

   Navigate to a git repository and check the current status:

   ```bash
   cd ~/projects/my-repo
   repoverlay status
   ```

   You should see: `No overlays applied.`

3. **Apply an overlay**

   Apply an overlay from a shared source — this will show you available overlays and let you pick:

   ```bash
   repoverlay apply tylerbutler
   ```

   Select an overlay from the interactive list. repoverlay will ask if you want to save the source for future use.

4. **Check status again**

   ```bash
   repoverlay status
   ```

   You'll see the applied overlay, its source, and the files it manages.

5. **Remove when done**

   ```bash
   repoverlay remove <name>
   ```

</Steps>

## Next steps

- Learn about [applying overlays](/guides/applying/) from different sources
- [Create and share](/guides/creating/) your own overlays
- Understand [how it works](/guides/how-it-works/) under the hood
```

**Step 2: Commit**

```
docs: rewrite Quick Start with tylerbutler example

Walk through apply with a username, showing interactive selection
and the source-save prompt. Start with status to show nothing applied.
```

---

### Task 7: Rewrite Applying Overlays guide

**Files:**
- Create: `website/src/content/docs/guides/applying.md` (overwrite existing)

**Step 1: Write the new applying guide**

Content outline (from the design doc):

1. **Basic usage** — `apply ./path`, `apply https://github.com/...`, `apply tylerbutler`
2. **Source types** (inline) — local dirs, GitHub URLs, usernames, org/repo/name
3. **Interactive selection** — what happens with username or two-part reference
4. **Source persistence** — the prompt, `source add/remove/list` for manual management
5. **Conflict handling** — `--force`, `--skip-conflicts`, `--merge` (JSON deep merge), `--interactive`
6. **Other options** — `--copy`, `--name`, `--ref`, `--target`, `--dry-run`, `--from`
7. **Aside:** `browse` for exploration without applying

This page absorbs content from the old `concepts/sources.md` and `concepts/overlay-repos.md` (browsing parts). The conflict handling section addresses issue #127 (JSON merging docs). The source management section addresses issue #84 (vocabulary).

Reference the existing Applying guide for structure but rewrite completely. Use Starlight admonitions (`:::tip`, `:::note`, `:::caution`) for the browse aside and conflict warnings.

**Step 2: Commit**

```
docs: rewrite Applying Overlays guide

Covers all source types, interactive selection, source persistence,
conflict handling (--force, --skip-conflicts, --merge, --interactive),
and browse aside. Absorbs content from old concepts/sources and
concepts/overlay-repos pages.

Addresses #127, #84.
```

---

### Task 8: Rewrite Creating & Sharing guide

**Files:**
- Create: `website/src/content/docs/guides/creating.md` (overwrite existing)

**Step 1: Write the new creating guide**

Content outline:

1. **Creating from a repo** — `create my-overlay`, `create org/repo/name`, `--include`, interactive selector
2. **Local output** — `create-local ./output`
3. **Overlay configuration (advanced)** — `repoverlay.ccl` format: name, mappings, directories. Position as advanced, most useful for hand-authored overlays or cases where you need to remap files from their source location. Note that most overlays don't need a config file.
4. **Overlay repository structure** — the `org/repo/name` directory layout
5. **Sharing workflow** — push to GitHub, others apply via username or org/repo/name

Absorbs content from old `concepts/configuration.md` and `concepts/overlay-repos.md` (structure parts).

**Step 2: Commit**

```
docs: rewrite Creating & Sharing guide

Covers create, create-local, overlay config (as advanced topic),
repo structure, and sharing workflow. Absorbs content from old
concepts/configuration and concepts/overlay-repos pages.
```

---

### Task 9: Write Managing Applied Overlays guide

**Files:**
- Create: `website/src/content/docs/guides/managing.md`

**Step 1: Write the new managing guide**

Content outline:

1. **Check status** — `status`, `status --name` (start with this to show current state)
2. **Edit an overlay** — `edit --add`, `edit --remove`, `edit --interactive`
3. **Sync changes back** — `sync`
4. **Update from remote** — `update`, `update --dry-run`, when to update
5. **Remove overlays** — `remove <name>`, `remove --all`, `remove --interactive`
6. **Switch overlays** — `switch` as atomic remove-all + apply

Absorbs content from old `guides/managing-files.md` (fixed to use `edit` not `add`), `guides/updating.md`, and `guides/switching.md`.

**Step 2: Commit**

```
docs: add Managing Applied Overlays guide

Covers status, edit, sync, update, remove, and switch. Starts with
status to show current state. Fixes stale 'add' command references
to use 'edit --add'. Absorbs content from old managing-files,
updating, and switching guides.
```

---

### Task 10: Write How It Works page

**Files:**
- Create: `website/src/content/docs/guides/how-it-works.md`

**Step 1: Write the How It Works page**

Content outline:

1. **Symlinks vs copies** — default behavior, `--copy`, platform considerations
2. **Git exclusion** — `.git/info/exclude` sections, why not `.gitignore`
3. **State tracking** — in-repo `.repoverlay/` + external backup at `~/.local/share/repoverlay/applied/`
4. **Caching** — shallow clones, cache location, `cache list/clear/remove/path`
5. **Fork inheritance** — upstream detection, resolution order, status display

Absorbs content from old `concepts/how-overlays-work.md`, `concepts/fork-inheritance.md`, and `guides/cache.md`.

**Step 2: Commit**

```
docs: add How It Works reference page

Covers symlinks, git exclusion, state tracking, caching, and fork
inheritance. Absorbs content from old how-overlays-work,
fork-inheritance, and cache pages.
```

---

### Task 11: Update Restoring guide

**Files:**
- Modify: `website/src/content/docs/guides/restoring.md`

**Step 1: Minor updates**

- Remove the TODO comment
- Adjust the sidebar order to `4` (after managing, before how-it-works)
- Add a cross-reference to the How It Works page for more detail on state tracking
- Content is mostly fine as-is

**Step 2: Commit**

```
docs: update Restoring guide with cross-references
```

---

### Task 12: Build and validate the docs site

**Files:** None (validation only)

**Step 1: Build the site**

```bash
cd website && pnpm install && pnpm build
```

**Step 2: Check for broken links**

The `starlight-links-validator` plugin runs at build time. Fix any broken links that surface.

**Step 3: Verify sidebar order**

Start a dev server and check:
```bash
pnpm dev
```

Confirm pages appear in the correct order in the sidebar.

**Step 4: Commit any fixes**

```
fix(docs): resolve broken links from site restructuring
```

---

### Task 13: Run full test suite and final commit

**Step 1: Run all checks**

```bash
just check
```

**Step 2: Fix any failures**

**Step 3: Final commit if needed**
