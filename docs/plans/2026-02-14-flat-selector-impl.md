# Flat List Selector Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the `dialoguer` overlay picker and `stdin::read_line` remove picker with a crossterm-based flat list selector that matches the file picker's visual style.

**Architecture:** Add a `select_flat()` function alongside the existing `select_files()` in `selection.rs`. It reuses the same crossterm rendering helpers (checkbox glyphs, help bar, scroll indicators, search, cursor) but has no tree/category/directory logic. Both the overlay picker (`lib.rs`) and remove picker (`cli.rs`) call `select_flat()`.

**Tech Stack:** crossterm 0.29 (already a dependency), no new crates needed. `dialoguer` dependency can be removed if no other callers remain.

---

### Task 1: Add `SelectableItem` and `FlatSelectionResult` types to `selection.rs`

**Files:**
- Modify: `src/selection.rs:32-58` (after `SelectionResult`, before `SelectionConfig`)

**Step 1: Write the failing test**

Add to the bottom of `src/selection.rs` inside `mod tests`:

```rust
#[test]
fn test_flat_state_new_sets_preselections() {
    let items = vec![
        SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: true,
            disabled: false,
        },
        SelectableItem {
            id: "b".into(),
            label: "Beta".into(),
            description: None,
            preselected: false,
            disabled: false,
        },
    ];
    let state = FlatSelectionState::new(items);
    assert!(state.selections.contains("a"));
    assert!(!state.selections.contains("b"));
}

#[test]
fn test_flat_state_toggle_skips_disabled() {
    let items = vec![
        SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: true,
        },
        SelectableItem {
            id: "b".into(),
            label: "Beta".into(),
            description: None,
            preselected: false,
            disabled: false,
        },
    ];
    let mut state = FlatSelectionState::new(items);
    state.toggle_selection(0); // "a" is disabled, should not toggle
    assert!(!state.selections.contains("a"));
    state.toggle_selection(1); // "b" is enabled, should toggle
    assert!(state.selections.contains("b"));
}

#[test]
fn test_flat_state_search_filters() {
    let items = vec![
        SelectableItem {
            id: "a".into(),
            label: "Alpha overlay".into(),
            description: None,
            preselected: false,
            disabled: false,
        },
        SelectableItem {
            id: "b".into(),
            label: "Beta config".into(),
            description: None,
            preselected: false,
            disabled: false,
        },
    ];
    let mut state = FlatSelectionState::new(items);
    state.search_query = "alpha".into();
    let visible = state.visible_items();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "a");
}

#[test]
fn test_flat_state_select_all_skips_disabled() {
    let items = vec![
        SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: true,
        },
        SelectableItem {
            id: "b".into(),
            label: "Beta".into(),
            description: None,
            preselected: false,
            disabled: false,
        },
        SelectableItem {
            id: "c".into(),
            label: "Gamma".into(),
            description: None,
            preselected: false,
            disabled: false,
        },
    ];
    let mut state = FlatSelectionState::new(items);
    state.select_all_visible();
    assert!(!state.selections.contains("a")); // disabled
    assert!(state.selections.contains("b"));
    assert!(state.selections.contains("c"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test flat_state -- --nocapture`
Expected: compilation errors — `SelectableItem`, `FlatSelectionState` don't exist yet.

**Step 3: Add the types and `FlatSelectionState`**

Add these types after line 38 (`SelectionResult` struct) in `selection.rs`:

```rust
/// An item in a flat (non-tree) selection list.
#[derive(Debug, Clone)]
pub struct SelectableItem {
    /// Unique identifier for this item (used in results).
    pub id: String,
    /// Display label shown to the user.
    pub label: String,
    /// Optional secondary description (shown dimmed after label).
    pub description: Option<String>,
    /// Whether this item starts selected.
    pub preselected: bool,
    /// Whether this item is disabled (visible but cannot be toggled).
    pub disabled: bool,
}

/// Result of a flat selection.
pub struct FlatSelectionResult {
    /// IDs of the selected items.
    pub selected_ids: Vec<String>,
    /// Whether the selection was cancelled.
    pub cancelled: bool,
}

/// Configuration for the flat selection UI.
pub struct FlatSelectionConfig {
    /// Prompt text shown at the top.
    pub prompt: String,
}

/// Internal state for the flat list selector.
struct FlatSelectionState {
    items: Vec<SelectableItem>,
    selections: HashSet<String>,
    search_query: String,
    mode: Mode, // reuse existing Mode enum
    cursor: usize,
    scroll_offset: usize,
}

impl FlatSelectionState {
    fn new(items: Vec<SelectableItem>) -> Self {
        let selections: HashSet<String> = items
            .iter()
            .filter(|i| i.preselected && !i.disabled)
            .map(|i| i.id.clone())
            .collect();
        Self {
            items,
            selections,
            search_query: String::new(),
            mode: Mode::Selection,
            cursor: 0,
            scroll_offset: 0,
        }
    }

    fn visible_items(&self) -> Vec<&SelectableItem> {
        self.items
            .iter()
            .filter(|i| {
                if self.search_query.is_empty() {
                    true
                } else {
                    i.label
                        .to_lowercase()
                        .contains(&self.search_query.to_lowercase())
                }
            })
            .collect()
    }

    fn toggle_selection(&mut self, visible_index: usize) {
        let visible = self.visible_items();
        if let Some(item) = visible.get(visible_index) {
            if item.disabled {
                return;
            }
            if self.selections.contains(&item.id) {
                self.selections.remove(&item.id);
            } else {
                self.selections.insert(item.id.clone());
            }
        }
    }

    fn select_all_visible(&mut self) {
        let visible = self.visible_items();
        let all_enabled_selected = visible
            .iter()
            .filter(|i| !i.disabled)
            .all(|i| self.selections.contains(&i.id));

        if all_enabled_selected {
            // Deselect all enabled visible
            for item in visible.iter().filter(|i| !i.disabled) {
                self.selections.remove(&item.id);
            }
        } else {
            // Select all enabled visible
            for item in visible.iter().filter(|i| !i.disabled) {
                self.selections.insert(item.id.clone());
            }
        }
    }

    fn clamp_cursor(&mut self) {
        let len = self.visible_items().len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test flat_state -- --nocapture`
Expected: all 4 tests PASS.

**Step 5: Commit**

```bash
git add src/selection.rs
git commit -m "feat(selection): add flat list selector types and state"
```

---

### Task 2: Add `select_flat()` entry point and event loop

**Files:**
- Modify: `src/selection.rs` (add `select_flat`, `run_flat_loop`, render functions)

**Step 1: Add the `select_flat()` public function and rendering**

Add after `select_files()` (around line 545):

```rust
/// Run the interactive flat list selection UI.
///
/// Returns the selected item IDs, or a cancelled result if the user aborts.
///
/// # Non-TTY Fallback
///
/// If stdin is not a TTY, returns all preselected (non-disabled) items.
pub fn select_flat(
    items: &[SelectableItem],
    config: FlatSelectionConfig,
) -> anyhow::Result<FlatSelectionResult> {
    if !is_interactive() {
        let selected: Vec<String> = items
            .iter()
            .filter(|i| i.preselected && !i.disabled)
            .map(|i| i.id.clone())
            .collect();
        return Ok(FlatSelectionResult {
            selected_ids: selected,
            cancelled: false,
        });
    }

    if items.is_empty() {
        return Ok(FlatSelectionResult {
            selected_ids: Vec::new(),
            cancelled: false,
        });
    }

    let mut state = FlatSelectionState::new(items.to_vec());

    terminal::enable_raw_mode()?;
    let result = run_flat_loop(&mut state, &config.prompt);
    terminal::disable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::Show,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::All),
    )?;
    println!();
    stdout.flush()?;

    result
}

fn run_flat_loop(
    state: &mut FlatSelectionState,
    prompt: &str,
) -> anyhow::Result<FlatSelectionResult> {
    let mut stdout = io::stdout();
    execute!(stdout, cursor::Hide, terminal::Clear(ClearType::All))?;

    loop {
        render_flat_ui(&mut stdout, state, prompt)?;

        if let Event::Key(key) = event::read()? {
            match state.mode {
                Mode::Search => {
                    if handle_flat_search_key(key, state) {
                        state.mode = Mode::Selection;
                    }
                }
                Mode::Selection => match key.code {
                    KeyCode::Esc => {
                        return Ok(FlatSelectionResult {
                            selected_ids: Vec::new(),
                            cancelled: true,
                        });
                    }
                    KeyCode::Enter => {
                        let selected: Vec<String> = state
                            .items
                            .iter()
                            .filter(|i| state.selections.contains(&i.id))
                            .map(|i| i.id.clone())
                            .collect();
                        return Ok(FlatSelectionResult {
                            selected_ids: selected,
                            cancelled: false,
                        });
                    }
                    KeyCode::Char('c')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(FlatSelectionResult {
                            selected_ids: Vec::new(),
                            cancelled: true,
                        });
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if state.cursor > 0 {
                            state.cursor -= 1;
                        }
                        adjust_flat_scroll(state);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let len = state.visible_items().len();
                        if state.cursor + 1 < len {
                            state.cursor += 1;
                        }
                        adjust_flat_scroll(state);
                    }
                    KeyCode::Char(' ') => {
                        state.toggle_selection(state.cursor);
                    }
                    KeyCode::Char('a') => {
                        state.select_all_visible();
                    }
                    KeyCode::Char('/') => {
                        state.mode = Mode::Search;
                    }
                    _ => {}
                },
            }
        }
    }
}

fn adjust_flat_scroll(state: &mut FlatSelectionState) {
    let max_visible = 15;
    if state.cursor < state.scroll_offset {
        state.scroll_offset = state.cursor;
    } else if state.cursor >= state.scroll_offset + max_visible {
        state.scroll_offset = state.cursor - max_visible + 1;
    }
}

fn handle_flat_search_key(key: KeyEvent, state: &mut FlatSelectionState) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Enter => {
            state.clamp_cursor();
            true
        }
        KeyCode::Backspace => {
            state.search_query.pop();
            state.clamp_cursor();
            false
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.search_query.push(c);
            state.clamp_cursor();
            false
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.search_query.clear();
            state.clamp_cursor();
            true
        }
        _ => false,
    }
}

fn render_flat_ui(
    stdout: &mut io::Stdout,
    state: &FlatSelectionState,
    prompt: &str,
) -> io::Result<()> {
    execute!(
        stdout,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::FromCursorDown)
    )?;

    // Prompt
    execute!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print(prompt),
        ResetColor,
        Print("\r\n\r\n")
    )?;

    // Search line
    render_search_line_flat(stdout, state)?;

    // Count summary
    let enabled_count = state.items.iter().filter(|i| !i.disabled).count();
    let selected_count = state.selections.len();
    execute!(
        stdout,
        Print("Selected: "),
        Print(format!("{selected_count}/{enabled_count}")),
        Print("\r\n\r\n")
    )?;

    // Item list
    render_flat_items(stdout, state)?;

    // Help
    render_flat_help(stdout, state)?;

    stdout.flush()
}

fn render_search_line_flat(
    stdout: &mut io::Stdout,
    state: &FlatSelectionState,
) -> io::Result<()> {
    execute!(stdout, Print("Search: "))?;
    if state.mode == Mode::Search {
        execute!(
            stdout,
            SetForegroundColor(Color::Yellow),
            Print(&state.search_query),
            Print("_"),
            ResetColor
        )?;
    } else if state.search_query.is_empty() {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("(press / to search)"),
            ResetColor
        )?;
    } else {
        execute!(
            stdout,
            Print(&state.search_query),
            SetForegroundColor(Color::DarkGrey),
            Print(" (Esc to clear)"),
            ResetColor
        )?;
    }
    execute!(stdout, Print("\r\n"))
}

fn render_flat_items(stdout: &mut io::Stdout, state: &FlatSelectionState) -> io::Result<()> {
    let visible = state.visible_items();
    let max_visible = 15;

    if visible.is_empty() {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("  No items match the current search\r\n"),
            ResetColor
        )?;
        return Ok(());
    }

    if state.scroll_offset > 0 {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(format!(
                "  ↑ {} more above\r\n",
                humanize_count(state.scroll_offset)
            )),
            ResetColor
        )?;
    }

    for (i, item) in visible
        .iter()
        .enumerate()
        .skip(state.scroll_offset)
        .take(max_visible)
    {
        let is_cursor = i == state.cursor;
        let is_selected = state.selections.contains(&item.id);

        // Cursor
        if is_cursor {
            execute!(stdout, SetForegroundColor(Color::Cyan), Print("> "))?;
        } else {
            execute!(stdout, Print("  "))?;
        }

        // Checkbox
        if item.disabled {
            execute!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                Print("[✓] "),
                ResetColor
            )?;
        } else if is_selected {
            execute!(
                stdout,
                SetForegroundColor(Color::Green),
                Print("[✓] "),
                ResetColor
            )?;
        } else {
            execute!(stdout, Print("[ ] "))?;
        }

        // Label
        if item.disabled {
            execute!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                Print(&item.label),
            )?;
        } else if is_cursor {
            execute!(
                stdout,
                SetForegroundColor(Color::Cyan),
                Print(&item.label),
            )?;
        } else {
            execute!(stdout, Print(&item.label))?;
        }

        // Description
        if let Some(desc) = &item.description {
            execute!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                Print(format!("  ({desc})")),
            )?;
        }

        execute!(stdout, ResetColor, Print("\r\n"))?;
    }

    let remaining = visible
        .len()
        .saturating_sub(state.scroll_offset + max_visible);
    if remaining > 0 {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(format!("  ↓ {} more below\r\n", humanize_count(remaining))),
            ResetColor
        )?;
    }

    Ok(())
}

fn render_flat_help(stdout: &mut io::Stdout, state: &FlatSelectionState) -> io::Result<()> {
    execute!(stdout, Print("\r\n"))?;
    if state.mode == Mode::Search {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("Type to search "),
            ResetColor
        )?;
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("| "),
            ResetColor
        )?;
        render_key_hint(stdout, "Enter/Esc", "done")?;
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("| "),
            ResetColor
        )?;
        render_key_hint(stdout, "Ctrl+C", "clear")
    } else {
        render_key_hint(stdout, "↑↓", "move")?;
        render_key_hint(stdout, "Space", "toggle")?;
        render_key_hint(stdout, "Enter", "confirm")?;
        render_key_hint(stdout, "a", "all")?;
        render_key_hint(stdout, "/", "search")?;
        render_key_hint(stdout, "Esc", "cancel")
    }
}
```

**Step 2: Run tests**

Run: `cargo test flat_state -- --nocapture`
Expected: all 4 tests still pass (new render code compiles but isn't tested interactively).

**Step 3: Run `just check` to verify clippy + formatting**

Run: `just check`
Expected: PASS

**Step 4: Commit**

```bash
git add src/selection.rs
git commit -m "feat(selection): add select_flat() UI and event loop"
```

---

### Task 3: Replace overlay picker in `lib.rs`

**Files:**
- Modify: `src/lib.rs:542-570` (replace `select_overlays_interactive`)
- Modify: `src/lib.rs:356-357` (update call site)

**Step 1: Replace `select_overlays_interactive` with `select_flat` call**

Replace the `select_overlays_interactive` function (lines 542–570) with:

```rust
/// Present an interactive multi-select picker for overlays.
fn select_overlays_interactive(
    owner: &str,
    repo: &str,
    overlays: &[String],
) -> Result<Vec<String>> {
    use crate::selection::{FlatSelectionConfig, SelectableItem, select_flat};

    let items: Vec<SelectableItem> = overlays
        .iter()
        .map(|o| SelectableItem {
            id: o.clone(),
            label: format_overlay_path(o),
            description: None,
            preselected: false,
            disabled: false,
        })
        .collect();

    let result = select_flat(
        &items,
        FlatSelectionConfig {
            prompt: format!("Select overlay(s) from {owner}/{repo}:"),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlays selected");
    }

    Ok(result.selected_ids)
}
```

**Step 2: Run the full test suite**

Run: `just test`
Expected: PASS (existing tests use non-interactive paths)

**Step 3: Check for remaining `dialoguer::MultiSelect` usage**

Run: `rg "dialoguer::MultiSelect\|MultiSelect" src/`
Expected: no results (the only usage was in `select_overlays_interactive`)

**Step 4: Commit**

```bash
git add src/lib.rs
git commit -m "feat(apply): use flat selector for overlay picker

Replaces dialoguer::MultiSelect with the crossterm-based flat
selector for visual consistency with the file picker."
```

---

### Task 4: Replace remove picker in `cli.rs`

**Files:**
- Modify: `src/cli.rs:827-929` (replace `handle_remove` interactive section)

**Step 1: Replace the interactive section of `handle_remove`**

Replace lines 851–926 (the interactive section after the `if !interactive` bail) with:

```rust
    // Interactive selection
    let target = canonicalize_path(target, "Target directory")?;
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    use crate::selection::{FlatSelectionConfig, SelectableItem, select_flat};

    let items: Vec<SelectableItem> = applied_overlays
        .iter()
        .map(|name| SelectableItem {
            id: name.clone(),
            label: name.clone(),
            description: None,
            preselected: false,
            disabled: false,
        })
        .collect();

    let result = select_flat(
        &items,
        FlatSelectionConfig {
            prompt: "Select overlay(s) to remove:".into(),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlays selected for removal");
    }

    let remove_all = result.selected_ids.len() == applied_overlays.len();

    for overlay_name in &result.selected_ids {
        if dry_run {
            println!(
                "{} Dry run - would remove overlay '{overlay_name}'",
                "Note:".yellow()
            );
        } else {
            remove_single_overlay(&target, &overlays_dir, overlay_name)?;
        }
    }

    if !dry_run {
        if remove_all {
            fs::remove_dir_all(target.join(STATE_DIR))?;
            println!(
                "\n{} Removed all overlays",
                "✓".green().bold()
            );
        } else {
            let remaining = list_applied_overlays(&target)?;
            if remaining.is_empty() {
                fs::remove_dir_all(target.join(STATE_DIR))?;
            }
        }
    }
```

**Step 2: Run the full test suite**

Run: `just test`
Expected: PASS

**Step 3: Commit**

```bash
git add src/cli.rs
git commit -m "feat(remove): use flat selector for overlay removal

Replaces the manual stdin::read_line menu with the crossterm-based
flat selector. Now supports multi-select removal."
```

---

### Task 5: Remove `dialoguer` dependency if unused

**Files:**
- Modify: `Cargo.toml` (remove `dialoguer` line)
- Possibly modify: `src/lib.rs` (check for remaining `dialoguer::Input` usage at line 2477)

**Step 1: Check remaining dialoguer usage**

Run: `rg "dialoguer" src/`
If line 2477 in `lib.rs` still uses `dialoguer::Input`, that usage needs to be replaced or kept. If it's the only remaining usage, evaluate whether to keep dialoguer for that one call or replace it too.

**Step 2: If dialoguer can be fully removed**

Remove the line `dialoguer = "0.12.0"` from `Cargo.toml`.

If `dialoguer::Input` is still used elsewhere, leave the dependency.

**Step 3: Run `just check`**

Run: `just check`
Expected: PASS (compiles, lints, tests pass)

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/
git commit -m "chore: remove dialoguer dependency

All interactive selection now uses the crossterm-based flat selector."
```

(Skip this commit if dialoguer is still needed for `Input`.)

---

### Task 6: Add disabled overlay support to apply command

**Files:**
- Modify: `src/lib.rs` (in `select_overlays_interactive`, check applied overlays)

**Step 1: Write a test**

This is best tested manually since it requires interactive TTY. But add a unit test for the item-building logic:

Add a new test in `src/lib.rs` `mod tests`:

```rust
#[test]
fn test_overlay_items_mark_applied_as_disabled() {
    // Simulate building SelectableItem list with some already applied
    let available = vec!["org/repo/a".to_string(), "org/repo/b".to_string(), "org/repo/c".to_string()];
    let applied = vec!["org/repo/b".to_string()];
    let applied_set: std::collections::HashSet<&str> = applied.iter().map(|s| s.as_str()).collect();

    let items: Vec<_> = available
        .iter()
        .map(|o| {
            let disabled = applied_set.contains(o.as_str());
            (o.clone(), disabled)
        })
        .collect();

    assert!(!items[0].1); // a not disabled
    assert!(items[1].1);  // b disabled (already applied)
    assert!(!items[2].1); // c not disabled
}
```

**Step 2: Update `select_overlays_interactive` to accept applied overlays**

Change the signature to accept applied overlay names and mark them disabled:

```rust
fn select_overlays_interactive(
    owner: &str,
    repo: &str,
    overlays: &[String],
    applied_overlays: &[String],
) -> Result<Vec<String>> {
    use crate::selection::{FlatSelectionConfig, SelectableItem, select_flat};

    let applied_set: std::collections::HashSet<&str> =
        applied_overlays.iter().map(|s| s.as_str()).collect();

    let items: Vec<SelectableItem> = overlays
        .iter()
        .map(|o| {
            let disabled = applied_set.contains(o.as_str());
            SelectableItem {
                id: o.clone(),
                label: format_overlay_path(o),
                description: if disabled {
                    Some("already applied".into())
                } else {
                    None
                },
                preselected: false,
                disabled,
            }
        })
        .collect();

    let result = select_flat(
        &items,
        FlatSelectionConfig {
            prompt: format!("Select overlay(s) from {owner}/{repo}:"),
        },
    )?;

    if result.cancelled || result.selected_ids.is_empty() {
        bail!("No overlays selected");
    }

    Ok(result.selected_ids)
}
```

**Step 3: Update the call site to pass applied overlays**

At the call site (~line 356), load the current applied overlays and pass them:

```rust
let applied = list_applied_overlays(&target)?;
let selected_overlays = if is_interactive() {
    select_overlays_interactive(owner, repo, &available_overlays, &applied)?
} else {
    // ... unchanged non-interactive path
};
```

**Step 4: Run `just check`**

Run: `just check`
Expected: PASS

**Step 5: Commit**

```bash
git add src/lib.rs
git commit -m "feat(apply): show already-applied overlays as disabled

Closes #90. Already-applied overlays appear dimmed with
'(already applied)' and cannot be selected."
```
