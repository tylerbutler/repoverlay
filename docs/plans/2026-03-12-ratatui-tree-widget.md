# Ratatui Tree Widget Migration

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the custom crossterm rendering in `selection.rs` with a ratatui-based UI, wrapping `tui-tree-widget` in a custom `MultiSelectTree` widget designed for eventual extraction as a standalone crate.

**Architecture:** The custom widget (`MultiSelectTree`) wraps `tui-tree-widget`'s `Tree`/`TreeState` and adds multi-select with tri-state checkboxes. The existing `SelectionState` business logic stays untouched — only rendering and the event loop change. The widget is generic over identifier type and has no dependency on repoverlay domain types, making it extractable.

**Tech Stack:** ratatui 0.29+, tui-tree-widget 0.24+, crossterm 0.29 (already a dependency)

---

## Design Decisions

### Widget API (extractable)

The `MultiSelectTree` widget will:
- Accept `TreeItem` nodes from `tui-tree-widget`
- Maintain its own `MultiSelectTreeState` that wraps `TreeState` and adds a `HashSet` of selected identifiers
- Render tri-state checkboxes: `[✓]` all selected, `[-]` partial, `[ ]` none
- Be generic: `MultiSelectTree<'a, Id>` where `Id: Clone + Eq + Hash`
- Have no knowledge of `DetectedFile`, `FileCategory`, or any repoverlay types

### Integration layer (repoverlay-specific)

A thin adapter in `selection.rs` will:
- Convert `Vec<DetectedFile>` → `Vec<TreeItem<PathBuf>>`
- Map `SelectionState` operations to `MultiSelectTreeState` operations
- Handle category filters, search, and the prompt/help chrome
- Drive the ratatui `Terminal` event loop

### Module layout

```
src/
├── widgets/
│   ├── mod.rs                  # pub mod multi_select_tree;
│   └── multi_select_tree.rs    # Generic widget (extractable)
├── selection.rs                # Adapted to use ratatui + widget
└── ...
```

---

## Task 1: Add dependencies

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add ratatui and tui-tree-widget**

Add to `[dependencies]` in `Cargo.toml`:

```toml
ratatui = { version = "0.29", default-features = false, features = ["crossterm"] }
tui-tree-widget = "0.24"
```

Note: `crossterm` is already a dependency. ratatui re-exports it, but we keep the explicit dep for the event-reading code that lives outside the widget.

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles with no errors (new deps are unused so far)

**Step 3: Commit**

```
feat(deps): add ratatui and tui-tree-widget dependencies
```

---

## Task 2: Create the `MultiSelectTree` widget — data types

**Files:**
- Create: `src/widgets/mod.rs`
- Create: `src/widgets/multi_select_tree.rs`
- Modify: `src/lib.rs` (add `mod widgets;`)

**Step 1: Write the failing test**

In `src/widgets/multi_select_tree.rs`, add a test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tui_tree_widget::TreeItem;

    fn make_tree() -> Vec<TreeItem<'static, String>> {
        vec![
            TreeItem::new("root".to_string(), "root/", vec![
                TreeItem::new_leaf("child-a".to_string(), "a.txt"),
                TreeItem::new_leaf("child-b".to_string(), "b.txt"),
            ]).unwrap(),
            TreeItem::new_leaf("lone".to_string(), "lone.txt"),
        ]
    }

    #[test]
    fn toggle_leaf_selection() {
        let mut state = MultiSelectTreeState::<String>::default();
        assert!(!state.is_selected(&"child-a".to_string()));
        state.toggle(&"child-a".to_string());
        assert!(state.is_selected(&"child-a".to_string()));
        state.toggle(&"child-a".to_string());
        assert!(!state.is_selected(&"child-a".to_string()));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test multi_select_tree::tests::toggle_leaf_selection`
Expected: FAIL — `MultiSelectTreeState` doesn't exist yet

**Step 3: Write minimal implementation**

In `src/widgets/multi_select_tree.rs`:

```rust
//! A multi-select tree widget built on top of `tui-tree-widget`.
//!
//! Adds multi-selection with tri-state checkboxes to `tui-tree-widget`'s
//! `Tree` and `TreeState`. Generic over the identifier type.

use std::collections::HashSet;
use std::hash::Hash;

use tui_tree_widget::TreeState;

/// Selection state of a node based on its descendants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    /// No descendants selected.
    Unchecked,
    /// Some but not all descendants selected.
    Partial,
    /// All descendants selected.
    Checked,
}

/// State for a multi-select tree widget.
///
/// Wraps [`TreeState`] (navigation, expand/collapse) and adds a
/// `HashSet` of selected identifiers for multi-select support.
#[derive(Debug)]
pub struct MultiSelectTreeState<Id: Clone + Eq + Hash> {
    /// The underlying tree navigation/expand state.
    pub tree: TreeState<Id>,
    /// Set of currently selected item identifiers.
    selected: HashSet<Id>,
}

impl<Id: Clone + Eq + Hash> Default for MultiSelectTreeState<Id> {
    fn default() -> Self {
        Self {
            tree: TreeState::default(),
            selected: HashSet::new(),
        }
    }
}

impl<Id: Clone + Eq + Hash> MultiSelectTreeState<Id> {
    /// Check whether an item is selected.
    pub fn is_selected(&self, id: &Id) -> bool {
        self.selected.contains(id)
    }

    /// Toggle selection of a single item.
    pub fn toggle(&mut self, id: &Id) {
        if self.selected.contains(id) {
            self.selected.remove(id);
        } else {
            self.selected.insert(id.clone());
        }
    }

    /// Select a single item.
    pub fn select(&mut self, id: Id) {
        self.selected.insert(id);
    }

    /// Deselect a single item.
    pub fn deselect(&mut self, id: &Id) {
        self.selected.remove(id);
    }

    /// Select multiple items.
    pub fn select_many(&mut self, ids: impl IntoIterator<Item = Id>) {
        self.selected.extend(ids);
    }

    /// Deselect multiple items.
    pub fn deselect_many<'a>(&mut self, ids: impl IntoIterator<Item = &'a Id>)
    where
        Id: 'a,
    {
        for id in ids {
            self.selected.remove(id);
        }
    }

    /// Get the number of selected items.
    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    /// Get an iterator over selected identifiers.
    pub fn selected_ids(&self) -> impl Iterator<Item = &Id> {
        self.selected.iter()
    }

    /// Clear all selections.
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Check the selection state of a node given its descendants.
    ///
    /// The caller provides the list of descendant IDs (not including the
    /// node itself). Returns `CheckState` based on how many are selected.
    pub fn check_state(&self, descendant_ids: &[Id]) -> CheckState {
        if descendant_ids.is_empty() {
            return CheckState::Unchecked;
        }
        let count = descendant_ids
            .iter()
            .filter(|id| self.selected.contains(*id))
            .count();
        if count == 0 {
            CheckState::Unchecked
        } else if count == descendant_ids.len() {
            CheckState::Checked
        } else {
            CheckState::Partial
        }
    }
}
```

In `src/widgets/mod.rs`:

```rust
pub mod multi_select_tree;
```

In `src/lib.rs`, add near the other `mod` declarations:

```rust
mod widgets;
```

**Step 4: Run test to verify it passes**

Run: `cargo test multi_select_tree::tests::toggle_leaf_selection`
Expected: PASS

**Step 5: Commit**

```
feat(widget): add MultiSelectTreeState with multi-select support
```

---

## Task 3: Add tri-state checkbox tests and bulk operations

**Files:**
- Modify: `src/widgets/multi_select_tree.rs`

**Step 1: Write failing tests**

Add to the test module:

```rust
#[test]
fn check_state_all_selected() {
    let mut state = MultiSelectTreeState::<String>::default();
    state.select("child-a".to_string());
    state.select("child-b".to_string());
    let descendants = vec!["child-a".to_string(), "child-b".to_string()];
    assert_eq!(state.check_state(&descendants), CheckState::Checked);
}

#[test]
fn check_state_partial() {
    let mut state = MultiSelectTreeState::<String>::default();
    state.select("child-a".to_string());
    let descendants = vec!["child-a".to_string(), "child-b".to_string()];
    assert_eq!(state.check_state(&descendants), CheckState::Partial);
}

#[test]
fn check_state_none_selected() {
    let state = MultiSelectTreeState::<String>::default();
    let descendants = vec!["child-a".to_string(), "child-b".to_string()];
    assert_eq!(state.check_state(&descendants), CheckState::Unchecked);
}

#[test]
fn check_state_empty_descendants() {
    let state = MultiSelectTreeState::<String>::default();
    assert_eq!(state.check_state(&[]), CheckState::Unchecked);
}

#[test]
fn select_many_and_deselect_many() {
    let mut state = MultiSelectTreeState::<String>::default();
    state.select_many(["a".to_string(), "b".to_string(), "c".to_string()]);
    assert_eq!(state.selected_count(), 3);
    state.deselect_many(&["a".to_string(), "b".to_string()]);
    assert_eq!(state.selected_count(), 1);
    assert!(state.is_selected(&"c".to_string()));
}

#[test]
fn clear_selection() {
    let mut state = MultiSelectTreeState::<String>::default();
    state.select_many(["a".to_string(), "b".to_string()]);
    state.clear_selection();
    assert_eq!(state.selected_count(), 0);
}
```

**Step 2: Run tests to verify they pass** (they should pass immediately since logic is already implemented)

Run: `cargo test multi_select_tree::tests`
Expected: all PASS

**Step 3: Commit**

```
test(widget): add tri-state checkbox and bulk operation tests
```

---

## Task 4: Implement the `MultiSelectTree` ratatui widget (rendering)

**Files:**
- Modify: `src/widgets/multi_select_tree.rs`

**Step 1: Write failing snapshot test**

Add `insta` to `[dev-dependencies]` in `Cargo.toml`:

```toml
insta = "1"
```

Add a rendering test:

```rust
#[test]
fn renders_tree_with_checkboxes() {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    let items = make_tree();
    let mut state = MultiSelectTreeState::<String>::default();
    state.select("child-a".to_string());
    state.tree.select(vec!["root".to_string()]);
    state.tree.open(vec!["root".to_string()]);

    terminal
        .draw(|frame| {
            let widget = MultiSelectTree::new(&items);
            frame.render_stateful_widget(widget, frame.area(), &mut state);
        })
        .unwrap();

    // Snapshot the rendered buffer as a string
    let buffer = terminal.backend().buffer().clone();
    insta::assert_snapshot!(buffer_to_string(&buffer));
}

/// Convert a ratatui Buffer to a trimmed string for snapshot testing.
fn buffer_to_string(buffer: &Buffer) -> String {
    let mut s = String::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            line.push_str(cell.symbol());
        }
        s.push_str(line.trim_end());
        s.push('\n');
    }
    // Trim trailing empty lines
    while s.ends_with("\n\n") {
        s.pop();
    }
    s
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test multi_select_tree::tests::renders_tree_with_checkboxes`
Expected: FAIL — `MultiSelectTree` struct doesn't exist

**Step 3: Implement the widget**

Add these imports to the top of `multi_select_tree.rs`:

```rust
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::StatefulWidget,
};
use tui_tree_widget::{Tree, TreeItem};
```

Add the widget struct and implementation:

```rust
/// Configuration for checkbox symbols.
#[derive(Debug, Clone)]
pub struct CheckboxSymbols {
    pub checked: &'static str,
    pub unchecked: &'static str,
    pub partial: &'static str,
}

impl Default for CheckboxSymbols {
    fn default() -> Self {
        Self {
            checked: "[✓] ",
            unchecked: "[ ] ",
            partial: "[-] ",
        }
    }
}

/// Configuration for checkbox styles.
#[derive(Debug, Clone)]
pub struct CheckboxStyles {
    pub checked: Style,
    pub unchecked: Style,
    pub partial: Style,
}

impl Default for CheckboxStyles {
    fn default() -> Self {
        Self {
            checked: Style::default().fg(Color::Green),
            unchecked: Style::default(),
            partial: Style::default().fg(Color::Yellow),
        }
    }
}

/// A callback that returns the list of descendant IDs for a given node.
///
/// Used to compute tri-state checkbox state for branch nodes.
pub type DescendantsFn<'a, Id> = Box<dyn Fn(&Id) -> Vec<Id> + 'a>;

/// A multi-select tree widget.
///
/// Wraps `tui-tree-widget`'s [`Tree`] and renders tri-state checkboxes
/// next to each node. The checkboxes are prepended to the node text.
pub struct MultiSelectTree<'a, Id: Clone + Eq + Hash> {
    /// The tree items to render.
    items: &'a [TreeItem<'a, Id>],
    /// The inner tree widget configuration.
    tree: Tree<'a, Id>,
    /// Checkbox symbols.
    checkbox_symbols: CheckboxSymbols,
    /// Checkbox styles.
    checkbox_styles: CheckboxStyles,
    /// Function to get descendant IDs for tri-state computation.
    /// If None, branch nodes show checked/unchecked based on their own ID only.
    descendants_fn: Option<DescendantsFn<'a, Id>>,
}

impl<'a, Id: Clone + Eq + Hash + 'a> MultiSelectTree<'a, Id> {
    /// Create a new multi-select tree from items.
    pub fn new(items: &'a [TreeItem<'a, Id>]) -> Self {
        let tree = Tree::new(items).expect("duplicate tree item identifiers");
        Self {
            items,
            tree,
            checkbox_symbols: CheckboxSymbols::default(),
            checkbox_styles: CheckboxStyles::default(),
            descendants_fn: None,
        }
    }

    /// Set the function used to compute descendant IDs for tri-state checkboxes.
    pub fn descendants_fn(mut self, f: DescendantsFn<'a, Id>) -> Self {
        self.descendants_fn = Some(f);
        self
    }

    /// Set the highlight style for the focused node.
    pub fn highlight_style(mut self, style: Style) -> Self {
        self.tree = self.tree.highlight_style(style);
        self
    }

    /// Set the highlight symbol prefix for the focused node.
    pub fn highlight_symbol(mut self, symbol: &'a str) -> Self {
        self.tree = self.tree.highlight_symbol(symbol);
        self
    }

    /// Set checkbox symbols.
    pub fn checkbox_symbols(mut self, symbols: CheckboxSymbols) -> Self {
        self.checkbox_symbols = symbols;
        self
    }

    /// Set checkbox styles.
    pub fn checkbox_styles(mut self, styles: CheckboxStyles) -> Self {
        self.checkbox_styles = styles;
        self
    }

    /// Set the symbol shown before expanded nodes.
    pub fn node_open_symbol(mut self, symbol: &'a str) -> Self {
        self.tree = self.tree.node_open_symbol(symbol);
        self
    }

    /// Set the symbol shown before collapsed nodes.
    pub fn node_closed_symbol(mut self, symbol: &'a str) -> Self {
        self.tree = self.tree.node_closed_symbol(symbol);
        self
    }

    /// Set the symbol shown before leaf nodes.
    pub fn node_no_children_symbol(mut self, symbol: &'a str) -> Self {
        self.tree = self.tree.node_no_children_symbol(symbol);
        self
    }
}
```

Now the key question: how to prepend checkboxes to tree items. `tui-tree-widget` renders `TreeItem` text as-is. The cleanest approach is to **rebuild the tree items with checkbox prefixes prepended to their text** right before rendering. This avoids forking the crate.

Add a helper method and the `StatefulWidget` impl:

```rust
impl<'a, Id: Clone + Eq + Hash + 'a> MultiSelectTree<'a, Id> {
    /// Build a new set of tree items with checkbox prefixes.
    fn items_with_checkboxes(
        &self,
        items: &'a [TreeItem<'a, Id>],
        state: &MultiSelectTreeState<Id>,
    ) -> Vec<TreeItem<'a, Id>> {
        items
            .iter()
            .map(|item| self.item_with_checkbox(item, state))
            .collect()
    }

    fn item_with_checkbox(
        &self,
        item: &TreeItem<'a, Id>,
        state: &MultiSelectTreeState<Id>,
    ) -> TreeItem<'a, Id> {
        let id = item.identifier().clone();
        let children = item.children();
        let has_children = !children.is_empty();

        // Determine check state
        let (symbol, style) = if has_children {
            if let Some(ref descendants_fn) = self.descendants_fn {
                let desc = descendants_fn(&id);
                match state.check_state(&desc) {
                    CheckState::Checked => (
                        self.checkbox_symbols.checked,
                        self.checkbox_styles.checked,
                    ),
                    CheckState::Partial => (
                        self.checkbox_symbols.partial,
                        self.checkbox_styles.partial,
                    ),
                    CheckState::Unchecked => (
                        self.checkbox_symbols.unchecked,
                        self.checkbox_styles.unchecked,
                    ),
                }
            } else if state.is_selected(&id) {
                (
                    self.checkbox_symbols.checked,
                    self.checkbox_styles.checked,
                )
            } else {
                (
                    self.checkbox_symbols.unchecked,
                    self.checkbox_styles.unchecked,
                )
            }
        } else if state.is_selected(&id) {
            (
                self.checkbox_symbols.checked,
                self.checkbox_styles.checked,
            )
        } else {
            (
                self.checkbox_symbols.unchecked,
                self.checkbox_styles.unchecked,
            )
        };

        // Build new text with checkbox prefix
        let checkbox_span = Span::styled(symbol.to_string(), style);
        let original_text = item.text().clone();
        let mut spans = vec![checkbox_span];
        spans.extend(original_text.into_iter().flat_map(|line| line.spans));
        let new_text = Line::from(spans);

        // Recurse into children
        let new_children = self.items_with_checkboxes(children, state);

        if new_children.is_empty() {
            TreeItem::new_leaf(id, new_text)
        } else {
            TreeItem::new(id, new_text, new_children)
                .expect("duplicate identifiers in children")
        }
    }
}

impl<'a, Id: Clone + Eq + Hash + 'a> StatefulWidget for MultiSelectTree<'a, Id> {
    type State = MultiSelectTreeState<Id>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // Rebuild items with checkboxes prepended
        let items_with_cb = self.items_with_checkboxes(self.items, state);

        // Create a fresh Tree widget with the checkbox-prefixed items
        let mut tree = Tree::new(&items_with_cb).expect("duplicate identifiers");

        // Apply same configuration — we need to re-apply since we can't move
        // the inner tree (it references the original items). This is the cost
        // of the "prepend to text" approach.
        tree = tree
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

        // Render using the inner tree state
        StatefulWidget::render(tree, area, buf, &mut state.tree);
    }
}
```

> **Note to implementer:** The `render` method above has a design issue — we can't move configuration from `self.tree` because it borrows the original items. The actual implementation should store configuration fields (highlight_style, symbols, etc.) separately on `MultiSelectTree` and apply them when constructing the fresh `Tree` in `render`. Adjust as needed to pass clippy and compile.

**Step 4: Run the snapshot test**

Run: `cargo test multi_select_tree::tests::renders_tree_with_checkboxes`
Expected: First run creates snapshot. Review with `cargo insta review`.

**Step 5: Commit**

```
feat(widget): implement MultiSelectTree StatefulWidget rendering
```

---

## Task 5: Integrate ratatui terminal into `select_files`

**Files:**
- Modify: `src/selection.rs`

This is the largest task. Replace the crossterm `execute!` rendering in `select_files` / `run_selection_loop` with a ratatui `Terminal` + `Frame` rendering model.

**Step 1: Update imports in `selection.rs`**

Replace the crossterm rendering imports with ratatui equivalents. Keep `crossterm::event` for input handling (ratatui doesn't wrap event reading).

Remove:
```rust
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};
```

Replace with:
```rust
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, StatefulWidget},
};
use tui_tree_widget::TreeItem;

use crate::widgets::multi_select_tree::{
    MultiSelectTree, MultiSelectTreeState,
};
```

**Step 2: Rewrite `run_selection_loop`**

Replace the function body to use ratatui's terminal:

```rust
fn run_selection_loop(state: &mut SelectionState, prompt: &str) -> anyhow::Result<SelectionResult> {
    let mut terminal = ratatui::init();

    let result = loop {
        terminal.draw(|frame| {
            render_selection_frame(frame, state, prompt);
        })?;

        if let Event::Key(key) = event::read()? {
            match state.mode {
                Mode::Selection => match handle_selection_key(state, key) {
                    SelectionAction::Continue => {}
                    SelectionAction::Confirm => {
                        break Ok(SelectionResult {
                            selected_files: state.resolve_selected_paths(),
                            cancelled: false,
                        });
                    }
                    SelectionAction::Cancel => {
                        break Ok(SelectionResult {
                            selected_files: Vec::new(),
                            cancelled: true,
                        });
                    }
                    SelectionAction::EnterSearch => {
                        state.mode = Mode::Search;
                    }
                },
                Mode::Search => {
                    if handle_search_key(state, key) {
                        state.mode = Mode::Selection;
                    }
                }
            }
        }
    };

    ratatui::restore();
    result
}
```

**Step 3: Implement `render_selection_frame`**

This replaces `render_ui` and all its sub-functions. Use ratatui `Layout` constraints to split the frame vertically:

```rust
fn render_selection_frame(frame: &mut Frame, state: &SelectionState, prompt: &str) {
    let area = frame.area();

    // Vertical layout: prompt, categories, search, summary, separator, file tree, help
    let chunks = Layout::vertical([
        Constraint::Length(2), // prompt + blank line
        Constraint::Length(1), // category toggles
        Constraint::Length(1), // search
        Constraint::Length(1), // selection summary
        Constraint::Length(1), // separator
        Constraint::Min(3),    // file tree (fills remaining space)
        Constraint::Length(2), // help line
    ])
    .split(area);

    // Prompt
    let prompt_line = Line::from(Span::styled(prompt, Style::default().fg(Color::Cyan)));
    frame.render_widget(Paragraph::new(prompt_line), chunks[0]);

    // Category toggles
    render_category_line_ratatui(frame, chunks[1], state);

    // Search
    render_search_line_ratatui(frame, chunks[2], state);

    // Selection summary
    render_summary_ratatui(frame, chunks[3], state);

    // File tree using MultiSelectTree widget
    render_file_tree(frame, chunks[5], state);

    // Help
    render_help_ratatui(frame, chunks[6], state);
}
```

Each `render_*_ratatui` function builds `Line`/`Span` styled text and renders via `Paragraph` or the `MultiSelectTree` widget. These are straightforward translations of the existing `execute!` calls into ratatui styled text.

**Step 4: Convert `DetectedFile` list to `TreeItem` list**

Add a conversion function:

```rust
fn build_tree_items(files: &[&DetectedFile]) -> Vec<TreeItem<'static, PathBuf>> {
    // Build top-level items, nesting children under their parent directories
    let mut top_level = Vec::new();

    for file in files {
        if file.parent_dir.is_none() {
            if file.category == FileCategory::AiConfigDirectory {
                // Collect children
                let children: Vec<TreeItem<'static, PathBuf>> = files
                    .iter()
                    .filter(|f| f.parent_dir.as_deref() == Some(&file.path))
                    .map(|f| {
                        let name = f.path.file_name()
                            .map_or_else(
                                || f.path.to_string_lossy().to_string(),
                                |n| n.to_string_lossy().to_string(),
                            );
                        TreeItem::new_leaf(f.path.clone(), name)
                    })
                    .collect();

                let label = format!("{}/", file.path.display());
                top_level.push(
                    TreeItem::new(file.path.clone(), label, children)
                        .expect("duplicate paths")
                );
            } else {
                let label = file.path.to_string_lossy().to_string();
                top_level.push(TreeItem::new_leaf(file.path.clone(), label));
            }
        }
    }

    top_level
}
```

**Step 5: Update `select_files` to remove manual raw mode management**

`ratatui::init()` handles raw mode and alternate screen. Update `select_files`:

```rust
pub(crate) fn select_files(
    files: &[DetectedFile],
    config: &SelectionConfig,
) -> anyhow::Result<SelectionResult> {
    if !is_interactive() {
        let selected: Vec<PathBuf> = files
            .iter()
            .filter(|f| f.preselected)
            .map(|f| f.path.clone())
            .collect();
        return Ok(SelectionResult {
            selected_files: selected,
            cancelled: false,
        });
    }

    if files.is_empty() {
        return Ok(SelectionResult {
            selected_files: Vec::new(),
            cancelled: false,
        });
    }

    let mut state = SelectionState::new(files.to_vec(), config.default_hidden_categories.clone());
    // No manual enable_raw_mode/disable_raw_mode — ratatui::init/restore handles it
    run_selection_loop(&mut state, &config.prompt)
}
```

Remove the `restore_terminal()` function (ratatui handles this).

**Step 6: Run full test suite**

Run: `just check`
Expected: All tests pass, clippy clean

**Step 7: Commit**

```
feat(selection): migrate file selection UI to ratatui

Replace manual crossterm rendering with ratatui Terminal + Frame model.
The file tree now uses the MultiSelectTree widget. Terminal height is
dynamic instead of the hardcoded MAX_VISIBLE_ITEMS=15.
```

---

## Task 6: Migrate `select_flat` to ratatui

**Files:**
- Modify: `src/selection.rs`

**Step 1: Rewrite `run_flat_loop`**

Same pattern as Task 5: use `ratatui::init()`, `terminal.draw()`, and `ratatui::restore()`. The flat selector uses a plain `ratatui::widgets::List` instead of `MultiSelectTree`.

**Step 2: Remove old flat rendering functions**

Delete: `render_flat_ui`, `render_search_line_flat`, `render_flat_items`, `render_flat_help`, `adjust_flat_scroll`.

Replace with ratatui `Frame`-based equivalents.

**Step 3: Run tests**

Run: `just check`
Expected: PASS

**Step 4: Commit**

```
feat(selection): migrate flat selector to ratatui
```

---

## Task 7: Remove dead crossterm rendering code

**Files:**
- Modify: `src/selection.rs`
- Modify: `Cargo.toml`

**Step 1: Remove old rendering functions**

Delete all the old `render_*` functions that used `execute!`:
- `render_ui`
- `render_category_line`
- `render_category_toggle`
- `render_search_line`
- `render_selection_summary`
- `render_file_list`
- `render_key_hint`
- `render_help_line`
- `restore_terminal`

**Step 2: Remove unused crossterm features from Cargo.toml**

The `crossterm` dep can be removed from the explicit dependencies — ratatui re-exports it. Keep only if event reading uses it directly. Check if `crossterm::event` is re-exported by ratatui; if so, switch to `ratatui::crossterm::event`.

**Step 3: Remove unused imports**

Clean up any remaining `crossterm` style/cursor/terminal imports.

**Step 4: Run full checks**

Run: `just check`
Expected: PASS, no warnings

**Step 5: Commit**

```
refactor(selection): remove legacy crossterm rendering code
```

---

## Task 8: Add insta snapshot tests for the full selection UI

**Files:**
- Modify: `src/selection.rs` (test module)
- Create: `src/snapshots/` (insta snapshot files, auto-generated)

**Step 1: Add snapshot tests**

Using ratatui's `TestBackend`, render the full selection frame and snapshot it:

```rust
#[test]
fn snapshot_selection_ui_initial() {
    use ratatui::{Terminal, backend::TestBackend};

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let files = make_test_files();
    let state = SelectionState::new(files, HashSet::new());

    terminal.draw(|frame| {
        render_selection_frame(frame, &state, "Select files to include in overlay");
    }).unwrap();

    let buffer = terminal.backend().buffer().clone();
    insta::assert_snapshot!(buffer_to_string(&buffer));
}
```

Add variants: with search active, with category hidden, with directory expanded, with selections.

**Step 2: Run and review snapshots**

Run: `cargo test snapshot_selection && cargo insta review`

**Step 3: Commit**

```
test(selection): add insta snapshot tests for ratatui UI
```

---

## Task 9: Dynamic terminal height

**Files:**
- Modify: `src/selection.rs`

**Step 1: Remove `MAX_VISIBLE_ITEMS` constant**

The ratatui `Frame` provides `frame.area().height` which dynamically adapts to the actual terminal size. The `MultiSelectTree` widget and ratatui's `List` widget handle scrolling within their allocated area automatically via `TreeState`/`ListState`.

Verify that `adjust_scroll` uses the tree area height from the layout, not the old constant.

**Step 2: Run tests**

Run: `just check`

**Step 3: Commit**

```
feat(selection): use dynamic terminal height instead of hardcoded limit
```

---

## Summary of deliverables

| Task | What | Extractable? |
|------|------|--------------|
| 1 | Add deps | — |
| 2 | `MultiSelectTreeState` data types | Yes |
| 3 | Tri-state tests + bulk ops | Yes |
| 4 | `MultiSelectTree` `StatefulWidget` | Yes |
| 5 | `select_files` → ratatui | No (app-specific) |
| 6 | `select_flat` → ratatui | No (app-specific) |
| 7 | Remove dead code | No |
| 8 | Snapshot tests | Partially |
| 9 | Dynamic height | No |

Tasks 2–4 form the extractable `multi-select-tree-widget` crate. They have zero dependency on repoverlay types and could be published as `tui-multi-select-tree` or contributed upstream to `tui-tree-widget`.
