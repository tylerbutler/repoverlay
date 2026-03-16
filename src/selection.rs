//! Interactive file selection UI for overlay creation.
//!
//! This module provides a terminal-based UI for selecting files to include
//! in an overlay, with support for category filtering, search, and bulk selection.

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};

use crate::detection::{DetectedFile, FileCategory};

/// Conversion trait for types that can be represented as a [`SelectableItem`]
/// in the interactive selection UI.
///
/// Implementations provide the mapping from domain types (like overlay names)
/// to the generic selection UI model, including display labels, descriptions,
/// and disabled state.
pub(crate) trait ToSelectableItem {
    /// Convert this value into a [`SelectableItem`] for display in the selection UI.
    ///
    /// `target` is the path to the target repository, used to load overlay state
    /// for timestamp information.
    fn to_selectable_item(&self, target: &Path) -> SelectableItem;
}

/// Maximum number of items visible in the scrollable viewport.
const MAX_VISIBLE_ITEMS: usize = 15;

/// Format a number in a human-readable way (e.g., 1.2K, 3.5M).
#[allow(clippy::cast_precision_loss)]
pub(crate) fn humanize_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Result of the interactive selection process.
pub(crate) struct SelectionResult {
    /// Files that were selected by the user.
    pub(crate) selected_files: Vec<PathBuf>,
    /// Whether the selection was cancelled.
    pub(crate) cancelled: bool,
}

/// An item in a flat (non-tree) selection list.
#[derive(Debug, Clone)]
pub(crate) struct SelectableItem {
    /// Unique identifier for this item (used in results).
    pub(crate) id: String,
    /// Display label shown to the user.
    pub(crate) label: String,
    /// Optional secondary description (shown dimmed after label).
    pub(crate) description: Option<String>,
    /// Whether this item starts selected.
    pub(crate) preselected: bool,
    /// Whether this item is disabled (visible but cannot be toggled).
    pub(crate) disabled: bool,
}

/// Result of a flat selection.
pub(crate) struct FlatSelectionResult {
    /// IDs of the selected items.
    pub(crate) selected_ids: Vec<String>,
    /// Whether the selection was cancelled.
    pub(crate) cancelled: bool,
}

/// Configuration for the flat selection UI.
pub(crate) struct FlatSelectionConfig {
    /// Prompt text shown at the top.
    pub(crate) prompt: String,
}

/// Internal state for the flat list selector.
struct FlatSelectionState {
    items: Vec<SelectableItem>,
    selections: HashSet<String>,
    search_query: String,
    mode: Mode,
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
        if self.search_query.is_empty() {
            return self.items.iter().collect();
        }
        let query = self.search_query.to_lowercase();
        self.items
            .iter()
            .filter(|i| i.label.to_lowercase().contains(&query))
            .collect()
    }

    fn toggle_selection(&mut self, visible_index: usize) {
        let visible = self.visible_items();
        if let Some(item) = visible.get(visible_index) {
            if item.disabled {
                return;
            }
            let id = item.id.clone();
            if self.selections.contains(&id) {
                self.selections.remove(&id);
            } else {
                self.selections.insert(id);
            }
        }
    }

    fn select_all_visible(&mut self) {
        let ids: Vec<(String, bool)> = self
            .visible_items()
            .iter()
            .map(|i| (i.id.clone(), i.disabled))
            .collect();

        let all_enabled_selected = ids
            .iter()
            .filter(|(_, disabled)| !disabled)
            .all(|(id, _)| self.selections.contains(id));

        if all_enabled_selected {
            for (id, disabled) in &ids {
                if !disabled {
                    self.selections.remove(id);
                }
            }
        } else {
            for (id, disabled) in ids {
                if !disabled {
                    self.selections.insert(id);
                }
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

/// Configuration for the selection UI.
pub(crate) struct SelectionConfig {
    /// Prompt text shown at the top.
    pub(crate) prompt: String,
    /// Categories to hide by default.
    pub(crate) default_hidden_categories: HashSet<FileCategory>,
}

impl Default for SelectionConfig {
    fn default() -> Self {
        let mut hidden = HashSet::new();
        // Hide gitignored by default (can be very large, e.g. node_modules)
        hidden.insert(FileCategory::Gitignored);
        Self {
            prompt: "Select files to include in overlay".to_string(),
            default_hidden_categories: hidden,
        }
    }
}

/// Input mode for the selection UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Normal selection mode.
    Selection,
    /// Search/filter mode.
    Search,
}

/// Selection state of a directory's children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirSelectionState {
    /// No children selected.
    None,
    /// Some children selected.
    Partial,
    /// All children selected.
    All,
}

/// Internal state for the selection UI.
struct SelectionState {
    /// All files available for selection.
    all_files: Vec<DetectedFile>,
    /// Currently selected file paths (persists across filter changes).
    selections: HashSet<PathBuf>,
    /// Currently visible categories.
    visible_categories: HashSet<FileCategory>,
    /// Expanded directory paths (directories whose children are visible).
    expanded_dirs: HashSet<PathBuf>,
    /// Lookup from path → `parent_dir` for O(1) ancestor traversal.
    parent_map: HashMap<PathBuf, Option<PathBuf>>,
    /// Current search query.
    search_query: String,
    /// Current input mode.
    mode: Mode,
    /// Current cursor position in the visible file list.
    cursor: usize,
    /// Scroll offset for the file list.
    scroll_offset: usize,
}

impl SelectionState {
    fn new(files: Vec<DetectedFile>, hidden_categories: HashSet<FileCategory>) -> Self {
        // Start with all categories visible except those explicitly hidden
        let mut visible: HashSet<FileCategory> = FileCategory::ALL.iter().copied().collect();

        for cat in hidden_categories {
            visible.remove(&cat);
        }

        // Pre-select files that are marked as preselected
        let selections: HashSet<PathBuf> = files
            .iter()
            .filter(|f| f.preselected)
            .map(|f| f.path.clone())
            .collect();

        // AI config directories start expanded (they're preselected, so
        // showing contents aids discoverability)
        let expanded_dirs: HashSet<PathBuf> = files
            .iter()
            .filter(|f| f.category == FileCategory::AiConfigDirectory)
            .map(|f| f.path.clone())
            .collect();

        // Build parent lookup map for O(1) ancestor traversal
        let parent_map: HashMap<PathBuf, Option<PathBuf>> = files
            .iter()
            .map(|f| (f.path.clone(), f.parent_dir.clone()))
            .collect();

        Self {
            all_files: files,
            selections,
            visible_categories: visible,
            expanded_dirs,
            parent_map,
            search_query: String::new(),
            mode: Mode::Selection,
            cursor: 0,
            scroll_offset: 0,
        }
    }

    /// Get files that are currently visible (match category filter, search, and expand state).
    fn visible_files(&self) -> Vec<&DetectedFile> {
        self.all_files
            .iter()
            .filter(|f| self.visible_categories.contains(&f.category))
            .filter(|f| self.all_ancestors_expanded(f))
            .filter(|f| {
                if self.search_query.is_empty() {
                    true
                } else {
                    f.path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&self.search_query.to_lowercase())
                }
            })
            .collect()
    }

    /// Check that all ancestor directories of a file are expanded.
    ///
    /// Walks up the parent chain using the pre-built `parent_map` for O(1) lookups.
    /// If any ancestor is collapsed, the file should be hidden.
    fn all_ancestors_expanded(&self, file: &DetectedFile) -> bool {
        let mut current = file.parent_dir.as_deref();
        while let Some(parent) = current {
            if !self.expanded_dirs.contains(parent) {
                return false;
            }
            // Walk up using the pre-built parent map (O(1) per level)
            current = self.parent_map.get(parent).and_then(|opt| opt.as_deref());
        }
        true
    }

    /// Check if any filters are active.
    fn has_active_filters(&self) -> bool {
        !self.search_query.is_empty() || self.visible_categories.len() < FileCategory::ALL.len()
    }

    /// Toggle visibility of a category.
    fn toggle_category(&mut self, cat: FileCategory) {
        if self.visible_categories.contains(&cat) {
            // Don't allow hiding all categories
            if self.visible_categories.len() > 1 {
                self.visible_categories.remove(&cat);
            }
        } else {
            self.visible_categories.insert(cat);
        }
        self.clamp_cursor();
    }

    /// Set the search query.
    #[cfg(test)]
    fn set_search(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.clamp_cursor();
    }

    /// Toggle selection of a file at the given path.
    fn toggle_selection(&mut self, path: &Path) {
        if self.selections.contains(path) {
            self.selections.remove(path);
        } else {
            self.selections.insert(path.to_path_buf());
        }
    }

    /// Toggle selection of the file at the current cursor position.
    ///
    /// For directories: toggles all children (select all if not all selected, deselect all otherwise).
    /// For regular files: toggles individual selection.
    fn toggle_current(&mut self) {
        let visible = self.visible_files();
        if let Some(file) = visible.get(self.cursor) {
            if file.category == FileCategory::AiConfigDirectory {
                let dir_path = file.path.clone();
                let descendants: Vec<PathBuf> = self.descendants_of(&dir_path);
                let all_selected = !descendants.is_empty()
                    && descendants.iter().all(|c| self.selections.contains(c));
                if all_selected {
                    // Deselect directory and all descendants
                    self.selections.remove(&dir_path);
                    for desc in descendants {
                        self.selections.remove(&desc);
                    }
                } else {
                    // Select directory and all descendants
                    self.selections.insert(dir_path);
                    for desc in descendants {
                        self.selections.insert(desc);
                    }
                }
            } else {
                let path = file.path.clone();
                self.toggle_selection(&path);
            }
        }
    }

    /// Select all visible files.
    fn select_all_visible(&mut self) {
        let paths: Vec<PathBuf> = self
            .visible_files()
            .iter()
            .map(|f| f.path.clone())
            .collect();
        for path in paths {
            self.selections.insert(path);
        }
    }

    /// Select all files (regardless of filters).
    fn select_all(&mut self) {
        for file in &self.all_files {
            self.selections.insert(file.path.clone());
        }
    }

    /// Deselect all visible files.
    fn deselect_all_visible(&mut self) {
        let paths: Vec<PathBuf> = self
            .visible_files()
            .iter()
            .map(|f| f.path.clone())
            .collect();
        for path in paths {
            self.selections.remove(&path);
        }
    }

    /// Get selection counts per category: (selected, total).
    fn selection_counts(&self) -> HashMap<FileCategory, (usize, usize)> {
        let mut counts = HashMap::new();

        for cat in FileCategory::ALL {
            let total = self.all_files.iter().filter(|f| f.category == *cat).count();
            let selected = self
                .all_files
                .iter()
                .filter(|f| f.category == *cat && self.selections.contains(&f.path))
                .count();
            counts.insert(*cat, (selected, total));
        }

        counts
    }

    /// Move cursor up.
    fn cursor_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.adjust_scroll();
        }
    }

    /// Move cursor down.
    fn cursor_down(&mut self) {
        let visible_count = self.visible_files().len();
        if self.cursor + 1 < visible_count {
            self.cursor += 1;
            self.adjust_scroll();
        }
    }

    /// Clamp cursor to valid range after filter changes.
    fn clamp_cursor(&mut self) {
        let visible_count = self.visible_files().len();
        if visible_count == 0 {
            self.cursor = 0;
        } else if self.cursor >= visible_count {
            self.cursor = visible_count - 1;
        }
        self.adjust_scroll();
    }

    /// Adjust scroll offset to keep cursor visible.
    #[allow(clippy::missing_const_for_fn)]
    fn adjust_scroll(&mut self) {
        let max_visible = MAX_VISIBLE_ITEMS;
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + max_visible {
            self.scroll_offset = self.cursor - max_visible + 1;
        }
    }

    /// Check if all visible files are selected.
    fn all_visible_selected(&self) -> bool {
        let visible = self.visible_files();
        if visible.is_empty() {
            return false;
        }
        visible.iter().all(|f| self.selections.contains(&f.path))
    }

    /// Check if a file entry is an expandable directory.
    fn is_expandable(file: &DetectedFile) -> bool {
        file.category == FileCategory::AiConfigDirectory
    }

    /// Get immediate child paths belonging to a directory.
    #[cfg(test)]
    fn children_of(&self, dir_path: &Path) -> Vec<PathBuf> {
        self.all_files
            .iter()
            .filter(|f| f.parent_dir.as_deref() == Some(dir_path))
            .map(|f| f.path.clone())
            .collect()
    }

    /// Get all descendant paths of a directory (recursive).
    fn descendants_of(&self, dir_path: &Path) -> Vec<PathBuf> {
        let mut result = Vec::new();
        let mut dirs_to_check = vec![dir_path.to_path_buf()];

        while let Some(dir) = dirs_to_check.pop() {
            for f in &self.all_files {
                if f.parent_dir.as_deref() == Some(&dir) {
                    result.push(f.path.clone());
                    if f.category == FileCategory::AiConfigDirectory {
                        dirs_to_check.push(f.path.clone());
                    }
                }
            }
        }

        result
    }

    /// Get the selection state of a directory based on all descendants.
    fn dir_selection_state(&self, dir_path: &Path) -> DirSelectionState {
        let descendants = self.descendants_of(dir_path);
        if descendants.is_empty() {
            return DirSelectionState::None;
        }
        let selected_count = descendants
            .iter()
            .filter(|c| self.selections.contains(*c))
            .count();
        if selected_count == 0 {
            DirSelectionState::None
        } else if selected_count == descendants.len() {
            DirSelectionState::All
        } else {
            DirSelectionState::Partial
        }
    }

    /// Toggle expand/collapse of a directory.
    #[cfg(test)]
    fn toggle_expand(&mut self, dir_path: &Path) {
        if self.expanded_dirs.contains(dir_path) {
            self.expanded_dirs.remove(dir_path);
        } else {
            self.expanded_dirs.insert(dir_path.to_path_buf());
        }
        self.clamp_cursor();
    }

    /// Expand the directory at the current cursor position (no-op if not a directory).
    fn expand_current(&mut self) {
        let visible = self.visible_files();
        if let Some(file) = visible.get(self.cursor)
            && Self::is_expandable(file)
            && !self.expanded_dirs.contains(&file.path)
        {
            let path = file.path.clone();
            self.expanded_dirs.insert(path);
            self.clamp_cursor();
        }
    }

    /// Collapse the directory at the current cursor, or navigate to parent if on a child.
    fn collapse_current(&mut self) {
        let visible = self.visible_files();
        if let Some(file) = visible.get(self.cursor) {
            if Self::is_expandable(file) && self.expanded_dirs.contains(&file.path) {
                // Collapse this directory
                let path = file.path.clone();
                self.expanded_dirs.remove(&path);
                self.clamp_cursor();
            } else if let Some(ref parent) = file.parent_dir {
                // Navigate cursor to parent directory
                let parent = parent.clone();
                let visible_after = self.visible_files();
                if let Some(pos) = visible_after.iter().position(|f| f.path == parent) {
                    self.cursor = pos;
                    self.adjust_scroll();
                }
            }
        }
    }

    /// Resolve selected paths for output.
    ///
    /// If ALL children of a directory are selected, emit the directory path.
    /// If only SOME children are selected, emit only the individual file paths.
    fn resolve_selected_paths(&self) -> Vec<PathBuf> {
        let mut result: Vec<PathBuf> = Vec::new();
        let mut covered: HashSet<PathBuf> = HashSet::new();

        // Check each directory: if all descendants selected, emit directory path.
        // Process top-level directories first so they can cover nested ones.
        for file in &self.all_files {
            if file.category == FileCategory::AiConfigDirectory
                && self.selections.contains(&file.path)
                && !covered.contains(&file.path)
            {
                let descendants = self.descendants_of(&file.path);
                let all_selected = !descendants.is_empty()
                    && descendants.iter().all(|c| self.selections.contains(c));
                if all_selected {
                    result.push(file.path.clone());
                    // Mark the directory and all descendants as covered
                    covered.insert(file.path.clone());
                    for desc in descendants {
                        covered.insert(desc);
                    }
                }
                // If not all selected, individual files will be added below
            }
        }

        // Add remaining selected files not covered by a directory
        for path in &self.selections {
            if !covered.contains(path) {
                // Skip directory entries when children are emitted individually
                let is_dir = self
                    .all_files
                    .iter()
                    .any(|f| f.path == *path && f.category == FileCategory::AiConfigDirectory);
                if !is_dir {
                    result.push(path.clone());
                }
            }
        }

        result
    }
}

/// Restore terminal state after raw mode UI.
///
/// Shows the cursor, clears the screen, and flushes stdout.
fn restore_terminal() -> anyhow::Result<()> {
    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::Show,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::All),
    )?;
    // Print newline to ensure clean state for subsequent prompts
    println!();
    stdout.flush()?;
    Ok(())
}

/// Run the interactive file selection UI.
///
/// Returns the selected files, or a cancelled result if the user aborts.
///
/// # Non-TTY Fallback
///
/// If stdin is not a TTY (e.g., piped input), this function falls back to
/// returning all preselected files (AI configs) without showing the UI.
pub(crate) fn select_files(
    files: &[DetectedFile],
    config: &SelectionConfig,
) -> anyhow::Result<SelectionResult> {
    // Non-TTY fallback: return preselected files
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

    // Empty file list edge case
    if files.is_empty() {
        return Ok(SelectionResult {
            selected_files: Vec::new(),
            cancelled: false,
        });
    }

    let mut state = SelectionState::new(files.to_vec(), config.default_hidden_categories.clone());

    terminal::enable_raw_mode()?;
    let result = run_selection_loop(&mut state, &config.prompt);
    terminal::disable_raw_mode()?;
    restore_terminal()?;

    result
}

/// Run the interactive flat list selection UI.
///
/// Returns the selected item IDs, or a cancelled result if the user aborts.
///
/// # Non-TTY Fallback
///
/// If stdin is not a TTY, returns all preselected (non-disabled) items.
pub(crate) fn select_flat(
    items: &[SelectableItem],
    config: &FlatSelectionConfig,
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
    restore_terminal()?;

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
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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

const fn adjust_flat_scroll(state: &mut FlatSelectionState) {
    let max_visible = MAX_VISIBLE_ITEMS;
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

fn render_search_line_flat(stdout: &mut io::Stdout, state: &FlatSelectionState) -> io::Result<()> {
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
    let max_visible = MAX_VISIBLE_ITEMS;

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
            execute!(stdout, SetForegroundColor(Color::Cyan), Print(&item.label),)?;
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
            Print("Type to search | "),
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

/// Check if the terminal is interactive.
///
/// Returns false in these cases:
/// - stdin or stdout is not a TTY
/// - Running in a CI environment (CI env var is set)
/// - Running as a cargo test binary (compiled with cfg(test))
/// - TERM is unset or "dumb"
/// - `REPOVERLAY_NON_INTERACTIVE` env var is set
pub(crate) fn is_interactive() -> bool {
    use std::io::IsTerminal;

    // Explicit non-interactive override
    if std::env::var("REPOVERLAY_NON_INTERACTIVE").is_ok() {
        return false;
    }

    // CI environments are never interactive
    if std::env::var("CI").is_ok() {
        return false;
    }

    // When compiled with cfg(test), we're running inside a test binary
    if cfg!(test) {
        return false;
    }

    // Check TERM - if not set or "dumb", assume non-interactive
    match std::env::var("TERM") {
        Ok(term) if !term.is_empty() && term != "dumb" => {}
        _ => return false,
    }

    // Check if both stdin and stdout are terminals
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Main selection loop.
fn run_selection_loop(state: &mut SelectionState, prompt: &str) -> anyhow::Result<SelectionResult> {
    let mut stdout = io::stdout();

    loop {
        // Render the UI
        render_ui(&mut stdout, state, prompt)?;

        // Wait for input
        if let Event::Key(key) = event::read()? {
            match state.mode {
                Mode::Selection => match handle_selection_key(state, key) {
                    SelectionAction::Continue => {}
                    SelectionAction::Confirm => {
                        return Ok(SelectionResult {
                            selected_files: state.resolve_selected_paths(),
                            cancelled: false,
                        });
                    }
                    SelectionAction::Cancel => {
                        return Ok(SelectionResult {
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
    }
}

/// Actions that can result from key handling.
enum SelectionAction {
    Continue,
    Confirm,
    Cancel,
    EnterSearch,
}

/// Handle a key press in selection mode.
fn handle_selection_key(state: &mut SelectionState, key: KeyEvent) -> SelectionAction {
    match key.code {
        // Navigation
        KeyCode::Up | KeyCode::Char('k') => state.cursor_up(),
        KeyCode::Down | KeyCode::Char('j') => state.cursor_down(),

        // Tree expand/collapse
        KeyCode::Right | KeyCode::Char('l') => state.expand_current(),
        KeyCode::Left | KeyCode::Char('h') => state.collapse_current(),

        // Selection
        KeyCode::Char(' ') => state.toggle_current(),
        KeyCode::Enter => return SelectionAction::Confirm,
        KeyCode::Esc => return SelectionAction::Cancel,

        // Ctrl+C to cancel
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return SelectionAction::Cancel;
        }

        // Category toggles
        KeyCode::Char('1') => state.toggle_category(FileCategory::AiConfig),
        KeyCode::Char('2') => state.toggle_category(FileCategory::AiConfigDirectory),
        KeyCode::Char('3') => state.toggle_category(FileCategory::TrackedConfig),
        KeyCode::Char('4') => state.toggle_category(FileCategory::Gitignored),
        KeyCode::Char('5') => state.toggle_category(FileCategory::Untracked),

        // Search
        KeyCode::Char('/') => return SelectionAction::EnterSearch,

        // Select all
        KeyCode::Char('a') => {
            if state.has_active_filters() {
                // Toggle between select visible and deselect
                if state.all_visible_selected() {
                    state.deselect_all_visible();
                } else {
                    state.select_all_visible();
                }
            } else {
                state.select_all();
            }
        }

        // Shift+A to select all (even with filters)
        KeyCode::Char('A') => {
            state.select_all();
        }

        _ => {}
    }
    SelectionAction::Continue
}

/// Handle a key press in search mode. Returns true if should exit search mode.
fn handle_search_key(state: &mut SelectionState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => {
            // Exit search mode (keep the query)
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
            // Clear search on Ctrl+C in search mode
            state.search_query.clear();
            state.clamp_cursor();
            true
        }
        _ => false,
    }
}

/// Render the selection UI.
fn render_ui(stdout: &mut io::Stdout, state: &SelectionState, prompt: &str) -> io::Result<()> {
    // Move to top and clear
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

    // Category toggles with counts
    render_category_line(stdout, state)?;

    // Search line
    render_search_line(stdout, state)?;

    // Selection summary
    render_selection_summary(stdout, state)?;

    // Separator
    execute!(stdout, Print("\r\n"))?;

    // File list
    render_file_list(stdout, state)?;

    // Help line
    render_help_line(stdout, state)?;

    stdout.flush()
}

/// Render the category toggle line.
fn render_category_line(stdout: &mut io::Stdout, state: &SelectionState) -> io::Result<()> {
    let counts = state.selection_counts();

    execute!(stdout, Print("Categories: "))?;

    // AI Config
    let ai_visible = state.visible_categories.contains(&FileCategory::AiConfig);
    let (ai_sel, ai_total) = counts.get(&FileCategory::AiConfig).unwrap_or(&(0, 0));
    render_category_toggle(
        stdout,
        "1",
        "AI",
        *ai_sel,
        *ai_total,
        ai_visible,
        Color::Green,
    )?;

    execute!(stdout, Print(" "))?;

    // AI Config Directories
    let aid_visible = state
        .visible_categories
        .contains(&FileCategory::AiConfigDirectory);
    let (aid_sel, aid_total) = counts
        .get(&FileCategory::AiConfigDirectory)
        .unwrap_or(&(0, 0));
    render_category_toggle(
        stdout,
        "2",
        "DIR",
        *aid_sel,
        *aid_total,
        aid_visible,
        Color::Magenta,
    )?;

    execute!(stdout, Print(" "))?;

    // Tracked Config
    let tc_visible = state
        .visible_categories
        .contains(&FileCategory::TrackedConfig);
    let (tc_sel, tc_total) = counts.get(&FileCategory::TrackedConfig).unwrap_or(&(0, 0));
    render_category_toggle(
        stdout,
        "3",
        "TC",
        *tc_sel,
        *tc_total,
        tc_visible,
        Color::Cyan,
    )?;

    execute!(stdout, Print(" "))?;

    // Gitignored
    let gi_visible = state.visible_categories.contains(&FileCategory::Gitignored);
    let (gi_sel, gi_total) = counts.get(&FileCategory::Gitignored).unwrap_or(&(0, 0));
    render_category_toggle(
        stdout,
        "4",
        "GI",
        *gi_sel,
        *gi_total,
        gi_visible,
        Color::Yellow,
    )?;

    execute!(stdout, Print(" "))?;

    // Untracked
    let ut_visible = state.visible_categories.contains(&FileCategory::Untracked);
    let (ut_sel, ut_total) = counts.get(&FileCategory::Untracked).unwrap_or(&(0, 0));
    render_category_toggle(
        stdout,
        "5",
        "UT",
        *ut_sel,
        *ut_total,
        ut_visible,
        Color::Blue,
    )?;

    execute!(stdout, Print("\r\n"))
}

/// Render a single category toggle button.
fn render_category_toggle(
    stdout: &mut io::Stdout,
    key: &str,
    label: &str,
    selected: usize,
    total: usize,
    visible: bool,
    color: Color,
) -> io::Result<()> {
    let count_str = format!("({}/{})", humanize_count(selected), humanize_count(total));
    if visible {
        execute!(
            stdout,
            Print("["),
            SetForegroundColor(color),
            Print(key),
            ResetColor,
            Print("] "),
            SetForegroundColor(color),
            Print(label),
            ResetColor,
            Print(format!(" {count_str}"))
        )
    } else {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(format!("[{key}] {label} {count_str}")),
            ResetColor
        )
    }
}

/// Render the search line.
fn render_search_line(stdout: &mut io::Stdout, state: &SelectionState) -> io::Result<()> {
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

/// Render the selection summary line.
fn render_selection_summary(stdout: &mut io::Stdout, state: &SelectionState) -> io::Result<()> {
    let counts = state.selection_counts();
    let total_selected: usize = counts.values().map(|(s, _)| s).sum();

    execute!(stdout, Print("Selected: "))?;

    if total_selected == 0 {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("none"),
            ResetColor
        )?;
    } else {
        let parts: Vec<String> = [
            (FileCategory::AiConfig, "AI", Color::Green),
            (FileCategory::AiConfigDirectory, "DIR", Color::Magenta),
            (FileCategory::TrackedConfig, "TC", Color::Cyan),
            (FileCategory::Gitignored, "GI", Color::Yellow),
            (FileCategory::Untracked, "UT", Color::Blue),
        ]
        .iter()
        .filter_map(|(cat, label, _color)| {
            let (selected, _) = counts.get(cat).unwrap_or(&(0, 0));
            if *selected > 0 {
                Some(format!("{selected} {label}"))
            } else {
                None
            }
        })
        .collect();

        execute!(stdout, Print(parts.join(", ")))?;
    }

    execute!(stdout, Print("\r\n"))
}

/// Render the file list.
fn render_file_list(stdout: &mut io::Stdout, state: &SelectionState) -> io::Result<()> {
    let visible = state.visible_files();
    let max_visible = MAX_VISIBLE_ITEMS;

    if visible.is_empty() {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("  No files match the current filters\r\n"),
            ResetColor
        )?;
        return Ok(());
    }

    // Show scroll indicator if needed
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

    for (i, file) in visible
        .iter()
        .enumerate()
        .skip(state.scroll_offset)
        .take(max_visible)
    {
        let is_cursor = i == state.cursor;
        let is_selected = state.selections.contains(&file.path);

        // Cursor indicator
        if is_cursor {
            execute!(stdout, SetForegroundColor(Color::Cyan), Print("> "))?;
        } else {
            execute!(stdout, Print("  "))?;
        }

        // Indentation for tree children
        let indent = "  ".repeat(file.depth as usize);
        if !indent.is_empty() {
            execute!(stdout, Print(&indent))?;
        }

        // Checkbox (tri-state for directories)
        if file.category == FileCategory::AiConfigDirectory {
            let dir_state = state.dir_selection_state(&file.path);
            match dir_state {
                DirSelectionState::All => {
                    execute!(
                        stdout,
                        SetForegroundColor(Color::Green),
                        Print("[✓] "),
                        ResetColor
                    )?;
                }
                DirSelectionState::Partial => {
                    execute!(
                        stdout,
                        SetForegroundColor(Color::Yellow),
                        Print("[-] "),
                        ResetColor
                    )?;
                }
                DirSelectionState::None => {
                    execute!(stdout, Print("[ ] "))?;
                }
            }
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

        // Category indicator
        let cat_color = match file.category {
            FileCategory::AiConfig => Color::Green,
            FileCategory::AiConfigDirectory => Color::Magenta,
            FileCategory::TrackedConfig => Color::Cyan,
            FileCategory::Gitignored => Color::Yellow,
            FileCategory::Untracked => Color::Blue,
        };

        // Expand/collapse indicator for directories
        if file.category == FileCategory::AiConfigDirectory {
            let indicator = if state.expanded_dirs.contains(&file.path) {
                "▾ "
            } else {
                "▸ "
            };
            execute!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                Print(indicator),
                ResetColor
            )?;
        }

        // File path display
        // For children, show just the filename (parent path implied by tree)
        let path_str = if file.parent_dir.is_some() {
            file.path.file_name().map_or_else(
                || file.path.to_string_lossy().to_string(),
                |n| n.to_string_lossy().to_string(),
            )
        } else if file.category == FileCategory::AiConfigDirectory {
            format!("{}/", file.path.to_string_lossy())
        } else {
            file.path.to_string_lossy().to_string()
        };
        if is_cursor {
            execute!(
                stdout,
                SetForegroundColor(cat_color),
                Print(&path_str),
                ResetColor
            )?;
        } else {
            execute!(stdout, Print(&path_str))?;
        }

        execute!(stdout, ResetColor, Print("\r\n"))?;
    }

    // Show scroll indicator if more below
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

/// Render a key hint with highlighted key.
fn render_key_hint(stdout: &mut io::Stdout, key: &str, action: &str) -> io::Result<()> {
    execute!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print(key),
        SetForegroundColor(Color::DarkGrey),
        Print(format!(" {action} ")),
        ResetColor
    )
}

/// Render the help line.
fn render_help_line(stdout: &mut io::Stdout, state: &SelectionState) -> io::Result<()> {
    execute!(stdout, Print("\r\n"))?;

    if state.mode == Mode::Search {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("Type to search | "),
            ResetColor
        )?;
        render_key_hint(stdout, "Enter/Esc", "done")?;
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("| "),
            ResetColor
        )?;
        render_key_hint(stdout, "Ctrl+C", "clear")?;
        Ok(())
    } else {
        render_key_hint(stdout, "↑↓", "move")?;
        render_key_hint(stdout, "←→", "expand")?;
        render_key_hint(stdout, "Space", "toggle")?;
        render_key_hint(stdout, "Enter", "confirm")?;
        render_key_hint(stdout, "a", "all")?;
        render_key_hint(stdout, "1-4", "filter")?;
        render_key_hint(stdout, "/", "search")?;
        render_key_hint(stdout, "Esc", "cancel")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_files() -> Vec<DetectedFile> {
        vec![
            DetectedFile {
                path: PathBuf::from("CLAUDE.md"),
                category: FileCategory::AiConfig,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".claude/settings.json"),
                category: FileCategory::AiConfig,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".envrc"),
                category: FileCategory::Gitignored,
                preselected: false,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".env.local"),
                category: FileCategory::Gitignored,
                preselected: false,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from("scratch.txt"),
                category: FileCategory::Untracked,
                preselected: false,
                depth: 0,
                parent_dir: None,
            },
        ]
    }

    #[test]
    fn test_toggle_category_hides_files() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // All categories visible initially
        assert_eq!(state.visible_files().len(), 5);

        // Hide AI configs
        state.toggle_category(FileCategory::AiConfig);
        let visible = state.visible_files();
        assert_eq!(visible.len(), 3);
        assert!(visible.iter().all(|f| f.category != FileCategory::AiConfig));
    }

    #[test]
    fn test_search_filters_by_path() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Search for "claude"
        state.set_search("claude");
        let visible = state.visible_files();
        assert_eq!(visible.len(), 2);
        assert!(
            visible
                .iter()
                .all(|f| f.path.to_string_lossy().to_lowercase().contains("claude"))
        );
    }

    #[test]
    fn test_selections_persist_across_filter_changes() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Select a gitignored file
        state.toggle_selection(Path::new(".envrc"));
        assert!(state.selections.contains(Path::new(".envrc")));

        // Hide gitignored category
        state.toggle_category(FileCategory::Gitignored);

        // Selection should still be there
        assert!(state.selections.contains(Path::new(".envrc")));

        // Show gitignored again
        state.toggle_category(FileCategory::Gitignored);
        assert!(state.selections.contains(Path::new(".envrc")));
    }

    #[test]
    fn test_select_all_visible_respects_filters() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Clear preselections
        state.selections.clear();

        // Hide gitignored and untracked
        state.toggle_category(FileCategory::Gitignored);
        state.toggle_category(FileCategory::Untracked);

        // Select all visible (only AI configs)
        state.select_all_visible();

        // Should only have AI configs selected
        assert_eq!(state.selections.len(), 2);
        assert!(state.selections.contains(Path::new("CLAUDE.md")));
        assert!(
            state
                .selections
                .contains(Path::new(".claude/settings.json"))
        );
        assert!(!state.selections.contains(Path::new(".envrc")));
    }

    #[test]
    fn test_selection_counts() {
        let files = make_test_files();
        let state = SelectionState::new(files, HashSet::new());

        let counts = state.selection_counts();

        // AI configs are preselected
        assert_eq!(counts.get(&FileCategory::AiConfig), Some(&(2, 2)));
        // Others are not
        assert_eq!(counts.get(&FileCategory::Gitignored), Some(&(0, 2)));
        assert_eq!(counts.get(&FileCategory::Untracked), Some(&(0, 1)));
    }

    #[test]
    fn test_cannot_hide_all_categories() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Try to hide all categories
        state.toggle_category(FileCategory::AiConfig);
        state.toggle_category(FileCategory::Gitignored);
        state.toggle_category(FileCategory::Untracked); // Should fail

        // At least one category should remain visible
        assert!(!state.visible_categories.is_empty());
    }

    #[test]
    fn test_cursor_bounds() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Cursor starts at 0
        assert_eq!(state.cursor, 0);

        // Can't go above 0
        state.cursor_up();
        assert_eq!(state.cursor, 0);

        // Can move down
        state.cursor_down();
        assert_eq!(state.cursor, 1);

        // Move to end
        for _ in 0..10 {
            state.cursor_down();
        }
        assert_eq!(state.cursor, 4); // 5 files, max index is 4
    }

    #[test]
    fn test_has_active_filters() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // No filters active initially
        assert!(!state.has_active_filters());

        // Search is a filter
        state.set_search("test");
        assert!(state.has_active_filters());

        state.set_search("");
        assert!(!state.has_active_filters());

        // Hidden category is a filter
        state.toggle_category(FileCategory::Untracked);
        assert!(state.has_active_filters());
    }

    #[test]
    fn test_humanize_count_small_numbers() {
        assert_eq!(humanize_count(0), "0");
        assert_eq!(humanize_count(1), "1");
        assert_eq!(humanize_count(42), "42");
        assert_eq!(humanize_count(999), "999");
    }

    #[test]
    fn test_humanize_count_thousands() {
        assert_eq!(humanize_count(1000), "1.0K");
        assert_eq!(humanize_count(1500), "1.5K");
        assert_eq!(humanize_count(12345), "12.3K");
        assert_eq!(humanize_count(999_999), "1000.0K");
    }

    #[test]
    fn test_humanize_count_millions() {
        assert_eq!(humanize_count(1_000_000), "1.0M");
        assert_eq!(humanize_count(2_500_000), "2.5M");
        assert_eq!(humanize_count(10_000_000), "10.0M");
    }

    #[test]
    fn test_selection_config_default() {
        let config = SelectionConfig::default();

        assert_eq!(config.prompt, "Select files to include in overlay");
        assert!(
            config
                .default_hidden_categories
                .contains(&FileCategory::Gitignored)
        );
        assert!(
            !config
                .default_hidden_categories
                .contains(&FileCategory::AiConfig)
        );
        assert!(
            !config
                .default_hidden_categories
                .contains(&FileCategory::Untracked)
        );
    }

    #[test]
    fn test_toggle_current() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Clear preselections for clean test
        state.selections.clear();

        // Toggle current (first file)
        state.toggle_current();
        assert!(state.selections.contains(Path::new("CLAUDE.md")));

        // Toggle again to deselect
        state.toggle_current();
        assert!(!state.selections.contains(Path::new("CLAUDE.md")));
    }

    #[test]
    fn test_toggle_current_moves_with_cursor() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        state.selections.clear();

        // Move to second file and toggle
        state.cursor_down();
        state.toggle_current();

        assert!(!state.selections.contains(Path::new("CLAUDE.md")));
        assert!(
            state
                .selections
                .contains(Path::new(".claude/settings.json"))
        );
    }

    #[test]
    fn test_toggle_current_empty_visible_list() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Filter to show nothing by searching for nonexistent file
        state.set_search("nonexistent_file_xyz");
        assert!(state.visible_files().is_empty());

        // Toggle current should do nothing (not crash)
        state.toggle_current();
        // No assertions needed - just verifying it doesn't panic
    }

    #[test]
    fn test_select_all() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        state.selections.clear();

        // Hide some categories
        state.toggle_category(FileCategory::Untracked);

        // Select all (should select all files regardless of visibility)
        state.select_all();

        assert_eq!(state.selections.len(), 5);
        assert!(state.selections.contains(Path::new("CLAUDE.md")));
        assert!(
            state
                .selections
                .contains(Path::new(".claude/settings.json"))
        );
        assert!(state.selections.contains(Path::new(".envrc")));
        assert!(state.selections.contains(Path::new(".env.local")));
        assert!(state.selections.contains(Path::new("scratch.txt")));
    }

    #[test]
    fn test_deselect_all_visible() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Start with AI configs preselected
        assert!(state.selections.contains(Path::new("CLAUDE.md")));
        assert!(
            state
                .selections
                .contains(Path::new(".claude/settings.json"))
        );

        // Add selection to gitignored file
        state.toggle_selection(Path::new(".envrc"));

        // Hide gitignored category
        state.toggle_category(FileCategory::Gitignored);

        // Deselect all visible (should only deselect AI configs and untracked)
        state.deselect_all_visible();

        // AI configs should be deselected
        assert!(!state.selections.contains(Path::new("CLAUDE.md")));
        assert!(
            !state
                .selections
                .contains(Path::new(".claude/settings.json"))
        );

        // Hidden gitignored file should still be selected
        assert!(state.selections.contains(Path::new(".envrc")));
    }

    #[test]
    fn test_all_visible_selected_empty() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Filter to show nothing
        state.set_search("nonexistent_file_xyz");
        assert!(state.visible_files().is_empty());

        // Empty visible list returns false
        assert!(!state.all_visible_selected());
    }

    #[test]
    fn test_all_visible_selected_partial() {
        let files = make_test_files();
        let state = SelectionState::new(files, HashSet::new());

        // AI configs are preselected, but gitignored and untracked are not
        assert!(!state.all_visible_selected());
    }

    #[test]
    fn test_all_visible_selected_all() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Select everything
        state.select_all();

        assert!(state.all_visible_selected());
    }

    #[test]
    fn test_all_visible_selected_with_filter() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // AI configs are preselected
        // Hide everything except AI configs
        state.toggle_category(FileCategory::AiConfigDirectory);
        state.toggle_category(FileCategory::Gitignored);
        state.toggle_category(FileCategory::Untracked);

        // Now all visible (AI configs only) are selected
        assert!(state.all_visible_selected());
    }

    #[test]
    fn test_scroll_offset_adjustment_down() {
        // Create more files to trigger scrolling
        let mut files = Vec::new();
        for i in 0..20 {
            files.push(DetectedFile {
                path: PathBuf::from(format!("file{i}.txt")),
                category: FileCategory::Untracked,
                preselected: false,
                depth: 0,
                parent_dir: None,
            });
        }

        let mut state = SelectionState::new(files, HashSet::new());

        // Initially at top
        assert_eq!(state.cursor, 0);
        assert_eq!(state.scroll_offset, 0);

        // Move down past visible area (MAX_VISIBLE_ITEMS = 15)
        for _ in 0..16 {
            state.cursor_down();
        }

        // Cursor should be at 16, scroll_offset should have adjusted
        assert_eq!(state.cursor, 16);
        assert!(state.scroll_offset > 0);
    }

    #[test]
    fn test_scroll_offset_adjustment_up() {
        // Create more files to trigger scrolling
        let mut files = Vec::new();
        for i in 0..20 {
            files.push(DetectedFile {
                path: PathBuf::from(format!("file{i}.txt")),
                category: FileCategory::Untracked,
                preselected: false,
                depth: 0,
                parent_dir: None,
            });
        }

        let mut state = SelectionState::new(files, HashSet::new());

        // Move to bottom
        for _ in 0..19 {
            state.cursor_down();
        }

        // Scroll offset should be > 0
        let scroll_after_down = state.scroll_offset;
        assert!(scroll_after_down > 0);

        // Move back up
        for _ in 0..19 {
            state.cursor_up();
        }

        // Should be back at top
        assert_eq!(state.cursor, 0);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn test_clamp_cursor_when_filter_reduces_list() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Move cursor to last file (index 4)
        for _ in 0..4 {
            state.cursor_down();
        }
        assert_eq!(state.cursor, 4);

        // Hide all but AI configs (2 files) - this calls clamp_cursor internally
        state.toggle_category(FileCategory::AiConfigDirectory);
        state.toggle_category(FileCategory::Gitignored);
        state.toggle_category(FileCategory::Untracked);

        // Cursor should be clamped to valid range (less than 2 visible files)
        assert!(state.cursor < 2);
    }

    #[test]
    fn test_clamp_cursor_empty_list() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        state.cursor_down();
        assert_eq!(state.cursor, 1);

        // Filter to nothing
        state.set_search("nonexistent_file_xyz");

        // Cursor should be 0 for empty list
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_selection_state_default_hidden_categories() {
        let files = make_test_files();
        let mut hidden = HashSet::new();
        hidden.insert(FileCategory::Gitignored);

        let state = SelectionState::new(files, hidden);

        // Gitignored should be hidden
        assert!(!state.visible_categories.contains(&FileCategory::Gitignored));
        // Others should be visible
        assert!(state.visible_categories.contains(&FileCategory::AiConfig));
        assert!(state.visible_categories.contains(&FileCategory::Untracked));
    }

    #[test]
    fn test_visible_files_respects_category_and_search() {
        let files = make_test_files();
        let mut state = SelectionState::new(files, HashSet::new());

        // Hide untracked
        state.toggle_category(FileCategory::Untracked);

        // Search for "env"
        state.set_search("env");

        let visible = state.visible_files();

        // Should only show gitignored files matching "env"
        assert_eq!(visible.len(), 2);
        assert!(
            visible
                .iter()
                .all(|f| f.path.to_string_lossy().to_lowercase().contains("env"))
        );
        assert!(
            visible
                .iter()
                .all(|f| f.category != FileCategory::Untracked)
        );
    }

    #[test]
    fn test_mode_enum_equality() {
        assert_eq!(Mode::Selection, Mode::Selection);
        assert_eq!(Mode::Search, Mode::Search);
        assert_ne!(Mode::Selection, Mode::Search);
    }

    #[test]
    fn test_selection_result_fields() {
        let result = SelectionResult {
            selected_files: vec![PathBuf::from("test.txt")],
            cancelled: false,
        };

        assert_eq!(result.selected_files.len(), 1);
        assert!(!result.cancelled);

        let cancelled_result = SelectionResult {
            selected_files: Vec::new(),
            cancelled: true,
        };

        assert!(cancelled_result.selected_files.is_empty());
        assert!(cancelled_result.cancelled);
    }

    fn make_test_files_with_directories() -> Vec<DetectedFile> {
        vec![
            DetectedFile {
                path: PathBuf::from("CLAUDE.md"),
                category: FileCategory::AiConfig,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".claude"),
                category: FileCategory::AiConfigDirectory,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".cursor"),
                category: FileCategory::AiConfigDirectory,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".envrc"),
                category: FileCategory::Gitignored,
                preselected: false,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from("notes.txt"),
                category: FileCategory::Untracked,
                preselected: false,
                depth: 0,
                parent_dir: None,
            },
        ]
    }

    #[test]
    fn test_toggle_ai_config_directory_category() {
        let files = make_test_files_with_directories();
        let mut state = SelectionState::new(files, HashSet::new());

        // Initially visible
        assert!(
            state
                .visible_categories
                .contains(&FileCategory::AiConfigDirectory)
        );

        // Count visible files
        let initial_count = state.visible_files().len();
        assert_eq!(initial_count, 5);

        // Toggle off
        state.toggle_category(FileCategory::AiConfigDirectory);
        assert!(
            !state
                .visible_categories
                .contains(&FileCategory::AiConfigDirectory)
        );

        // Should have 2 fewer files (the directory entries)
        let after_toggle_count = state.visible_files().len();
        assert_eq!(after_toggle_count, 3);

        // Toggle back on
        state.toggle_category(FileCategory::AiConfigDirectory);
        assert!(
            state
                .visible_categories
                .contains(&FileCategory::AiConfigDirectory)
        );
        assert_eq!(state.visible_files().len(), 5);
    }

    #[test]
    fn test_selection_counts_includes_directories() {
        let files = make_test_files_with_directories();
        let state = SelectionState::new(files, HashSet::new());

        let counts = state.selection_counts();

        // Check AiConfigDirectory count
        let (selected, total) = counts
            .get(&FileCategory::AiConfigDirectory)
            .unwrap_or(&(0, 0));
        assert_eq!(*total, 2); // .claude and .cursor
        assert_eq!(*selected, 2); // Both preselected
    }

    #[test]
    fn test_has_active_filters_with_four_categories() {
        let files = make_test_files_with_directories();
        let mut state = SelectionState::new(files, HashSet::new());

        // No filters active (all 4 categories visible, no search)
        assert!(!state.has_active_filters());

        // Hide one category
        state.toggle_category(FileCategory::AiConfigDirectory);
        assert!(state.has_active_filters());

        // Restore it
        state.toggle_category(FileCategory::AiConfigDirectory);
        assert!(!state.has_active_filters());

        // Add search filter
        state.set_search("claude");
        assert!(state.has_active_filters());
    }

    #[test]
    fn test_directory_preselection() {
        let files = make_test_files_with_directories();
        let state = SelectionState::new(files, HashSet::new());

        // AiConfigDirectory entries should be preselected
        assert!(state.selections.contains(&PathBuf::from(".claude")));
        assert!(state.selections.contains(&PathBuf::from(".cursor")));
    }

    #[test]
    fn test_visible_categories_includes_directory_by_default() {
        let files = make_test_files_with_directories();
        let state = SelectionState::new(files, HashSet::new());

        assert!(state.visible_categories.contains(&FileCategory::AiConfig));
        assert!(
            state
                .visible_categories
                .contains(&FileCategory::AiConfigDirectory)
        );
        assert!(state.visible_categories.contains(&FileCategory::Gitignored));
        assert!(state.visible_categories.contains(&FileCategory::Untracked));
        assert!(
            state
                .visible_categories
                .contains(&FileCategory::TrackedConfig)
        );
        assert_eq!(state.visible_categories.len(), FileCategory::ALL.len());
    }

    #[test]
    fn is_interactive_returns_false_in_tests() {
        // In test context, is_interactive should return false
        // because cfg!(test) is true
        assert!(!is_interactive());
    }

    /// Helper to create test files with directories and children for tree tests.
    ///
    /// Tree structure:
    /// ```text
    /// CLAUDE.md                     (AiConfig, depth 0)
    /// .claude/                      (AiConfigDirectory, depth 0)
    ///   settings.json               (AiConfig, depth 1, parent: .claude)
    ///   commands/                   (AiConfigDirectory, depth 1, parent: .claude)
    ///     test.md                   (AiConfig, depth 2, parent: .claude/commands)
    /// .envrc                        (Gitignored, depth 0)
    /// ```
    fn make_test_files_with_children() -> Vec<DetectedFile> {
        vec![
            DetectedFile {
                path: PathBuf::from("CLAUDE.md"),
                category: FileCategory::AiConfig,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".claude"),
                category: FileCategory::AiConfigDirectory,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".claude/settings.json"),
                category: FileCategory::AiConfig,
                preselected: true,
                depth: 1,
                parent_dir: Some(PathBuf::from(".claude")),
            },
            DetectedFile {
                path: PathBuf::from(".claude/commands"),
                category: FileCategory::AiConfigDirectory,
                preselected: true,
                depth: 1,
                parent_dir: Some(PathBuf::from(".claude")),
            },
            DetectedFile {
                path: PathBuf::from(".claude/commands/test.md"),
                category: FileCategory::AiConfig,
                preselected: true,
                depth: 2,
                parent_dir: Some(PathBuf::from(".claude/commands")),
            },
            DetectedFile {
                path: PathBuf::from(".envrc"),
                category: FileCategory::Gitignored,
                preselected: false,
                depth: 0,
                parent_dir: None,
            },
        ]
    }

    #[test]
    fn test_expanded_dirs_default_state() {
        let files = make_test_files_with_children();
        let state = SelectionState::new(files, HashSet::new());

        // AI config directories start expanded (including nested ones)
        assert!(state.expanded_dirs.contains(&PathBuf::from(".claude")));
        assert!(
            state
                .expanded_dirs
                .contains(&PathBuf::from(".claude/commands"))
        );
    }

    #[test]
    fn test_toggle_expand_collapses_directory() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        assert!(state.expanded_dirs.contains(&PathBuf::from(".claude")));

        state.toggle_expand(Path::new(".claude"));
        assert!(!state.expanded_dirs.contains(&PathBuf::from(".claude")));

        state.toggle_expand(Path::new(".claude"));
        assert!(state.expanded_dirs.contains(&PathBuf::from(".claude")));
    }

    #[test]
    fn test_visible_files_hides_collapsed_children() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // With .claude expanded, all children are visible
        let visible = state.visible_files();
        // CLAUDE.md, .claude, settings.json, commands/, commands/test.md, .envrc
        assert_eq!(visible.len(), 6);

        // Collapse .claude — hides ALL descendants (including nested ones)
        state.toggle_expand(Path::new(".claude"));
        let visible = state.visible_files();
        assert_eq!(visible.len(), 3); // CLAUDE.md, .claude, .envrc
        assert!(
            visible
                .iter()
                .all(|f| f.path.as_path() != Path::new(".claude/settings.json"))
        );
        assert!(
            visible
                .iter()
                .all(|f| f.path.as_path() != Path::new(".claude/commands"))
        );
        assert!(
            visible
                .iter()
                .all(|f| f.path.as_path() != Path::new(".claude/commands/test.md"))
        );
    }

    #[test]
    fn test_expand_current_on_non_directory() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // Cursor is on CLAUDE.md (index 0, not a directory)
        state.expand_current();
        // No-op, no crash
        assert_eq!(state.expanded_dirs.len(), 2); // .claude and .claude/commands
    }

    #[test]
    fn test_collapse_current_navigates_to_parent() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // Move cursor to .claude/settings.json (index 2 when expanded)
        state.cursor_down(); // .claude
        state.cursor_down(); // .claude/settings.json
        assert_eq!(state.cursor, 2);

        // Collapse current (child) should navigate to parent
        state.collapse_current();

        // Cursor should be at .claude (index 1)
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn test_toggle_directory_selects_all_children() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());
        state.selections.clear();

        // Move cursor to .claude directory (index 1)
        state.cursor_down();
        assert_eq!(
            state.visible_files()[state.cursor].path,
            PathBuf::from(".claude")
        );

        // Toggle: should select directory + all children
        state.toggle_current();

        assert!(state.selections.contains(&PathBuf::from(".claude")));
        assert!(
            state
                .selections
                .contains(&PathBuf::from(".claude/settings.json"))
        );
        assert!(
            state
                .selections
                .contains(&PathBuf::from(".claude/commands/test.md"))
        );
    }

    #[test]
    fn test_toggle_directory_deselects_all_children() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // All are preselected; move cursor to .claude directory
        state.cursor_down();

        // Toggle: since all children are selected, should deselect all
        state.toggle_current();

        assert!(!state.selections.contains(&PathBuf::from(".claude")));
        assert!(
            !state
                .selections
                .contains(&PathBuf::from(".claude/settings.json"))
        );
        assert!(
            !state
                .selections
                .contains(&PathBuf::from(".claude/commands/test.md"))
        );
    }

    #[test]
    fn test_dir_selection_state_none() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());
        state.selections.clear();

        assert_eq!(
            state.dir_selection_state(Path::new(".claude")),
            DirSelectionState::None
        );
    }

    #[test]
    fn test_dir_selection_state_partial() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());
        state.selections.clear();

        // Select only one child
        state
            .selections
            .insert(PathBuf::from(".claude/settings.json"));

        assert_eq!(
            state.dir_selection_state(Path::new(".claude")),
            DirSelectionState::Partial
        );
    }

    #[test]
    fn test_dir_selection_state_all() {
        let files = make_test_files_with_children();
        let state = SelectionState::new(files, HashSet::new());

        // All children are preselected
        assert_eq!(
            state.dir_selection_state(Path::new(".claude")),
            DirSelectionState::All
        );
    }

    #[test]
    fn test_cursor_clamp_on_collapse() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // Move cursor to last visible item (index 5: .envrc)
        for _ in 0..5 {
            state.cursor_down();
        }
        assert_eq!(state.cursor, 5);

        // Collapse .claude — all descendants disappear, cursor should clamp
        state.toggle_expand(Path::new(".claude"));

        // After collapse, only 3 items visible (CLAUDE.md, .claude, .envrc)
        assert!(state.cursor < 3);
    }

    #[test]
    fn test_search_filters_within_expanded_children() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // Search for "settings" — should find child
        state.set_search("settings");
        let visible = state.visible_files();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].path, PathBuf::from(".claude/settings.json"));
    }

    #[test]
    fn test_select_all_visible_includes_expanded_children() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());
        state.selections.clear();

        // All visible includes expanded children and subdirectories
        state.select_all_visible();
        assert!(
            state
                .selections
                .contains(&PathBuf::from(".claude/settings.json"))
        );
        assert!(
            state
                .selections
                .contains(&PathBuf::from(".claude/commands"))
        );
        assert!(
            state
                .selections
                .contains(&PathBuf::from(".claude/commands/test.md"))
        );
    }

    #[test]
    fn test_resolve_paths_all_children_selected_emits_directory() {
        let files = make_test_files_with_children();
        let state = SelectionState::new(files, HashSet::new());

        // All files are preselected, so all children selected
        let resolved = state.resolve_selected_paths();

        // Should emit directory path, not individual children
        assert!(resolved.contains(&PathBuf::from(".claude")));
        assert!(!resolved.contains(&PathBuf::from(".claude/settings.json")));
        assert!(!resolved.contains(&PathBuf::from(".claude/commands/test.md")));
        // Non-directory files are still emitted
        assert!(resolved.contains(&PathBuf::from("CLAUDE.md")));
    }

    #[test]
    fn test_resolve_paths_partial_children_emits_individuals() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // Deselect one child
        state
            .selections
            .remove(&PathBuf::from(".claude/commands/test.md"));

        let resolved = state.resolve_selected_paths();

        // Should NOT emit directory path (partial selection)
        assert!(!resolved.contains(&PathBuf::from(".claude")));
        // Should emit individual selected child
        assert!(resolved.contains(&PathBuf::from(".claude/settings.json")));
        // Should NOT emit deselected child
        assert!(!resolved.contains(&PathBuf::from(".claude/commands/test.md")));
    }

    #[test]
    fn test_children_of() {
        let files = make_test_files_with_children();
        let state = SelectionState::new(files, HashSet::new());

        // children_of returns immediate children only
        let children = state.children_of(Path::new(".claude"));
        assert_eq!(children.len(), 2);
        assert!(children.contains(&PathBuf::from(".claude/settings.json")));
        assert!(children.contains(&PathBuf::from(".claude/commands")));
        // test.md is NOT a direct child of .claude
        assert!(!children.contains(&PathBuf::from(".claude/commands/test.md")));

        // children_of for nested directory
        let nested_children = state.children_of(Path::new(".claude/commands"));
        assert_eq!(nested_children.len(), 1);
        assert!(nested_children.contains(&PathBuf::from(".claude/commands/test.md")));
    }

    #[test]
    fn test_descendants_of() {
        let files = make_test_files_with_children();
        let state = SelectionState::new(files, HashSet::new());

        // descendants_of returns all descendants recursively
        let descendants = state.descendants_of(Path::new(".claude"));
        assert_eq!(descendants.len(), 3);
        assert!(descendants.contains(&PathBuf::from(".claude/settings.json")));
        assert!(descendants.contains(&PathBuf::from(".claude/commands")));
        assert!(descendants.contains(&PathBuf::from(".claude/commands/test.md")));
    }

    #[test]
    fn test_is_expandable() {
        let files = make_test_files_with_children();
        let state = SelectionState::new(files, HashSet::new());

        assert!(SelectionState::is_expandable(&state.all_files[1])); // .claude directory
        assert!(SelectionState::is_expandable(&state.all_files[3])); // .claude/commands directory
        assert!(!SelectionState::is_expandable(&state.all_files[0])); // CLAUDE.md file
        assert!(!SelectionState::is_expandable(&state.all_files[2])); // .claude/settings.json file
    }

    #[test]
    fn test_dir_selection_state_enum_equality() {
        assert_eq!(DirSelectionState::None, DirSelectionState::None);
        assert_eq!(DirSelectionState::Partial, DirSelectionState::Partial);
        assert_eq!(DirSelectionState::All, DirSelectionState::All);
        assert_ne!(DirSelectionState::None, DirSelectionState::Partial);
        assert_ne!(DirSelectionState::Partial, DirSelectionState::All);
    }

    #[test]
    fn test_collapse_intermediate_directory_hides_nested_children() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // All expanded initially — 6 items visible
        assert_eq!(state.visible_files().len(), 6);

        // Collapse only the intermediate .claude/commands directory
        state.toggle_expand(Path::new(".claude/commands"));

        let visible = state.visible_files();
        // Should see: CLAUDE.md, .claude, settings.json, commands/ (collapsed), .envrc
        assert_eq!(visible.len(), 5);
        // commands/test.md should be hidden
        assert!(
            visible
                .iter()
                .all(|f| f.path.as_path() != Path::new(".claude/commands/test.md"))
        );
        // commands/ directory itself should still be visible
        assert!(
            visible
                .iter()
                .any(|f| f.path.as_path() == Path::new(".claude/commands"))
        );
    }

    #[test]
    fn test_collapse_parent_hides_all_even_if_child_expanded() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // Both .claude and .claude/commands are expanded
        assert!(state.expanded_dirs.contains(&PathBuf::from(".claude")));
        assert!(
            state
                .expanded_dirs
                .contains(&PathBuf::from(".claude/commands"))
        );

        // Collapse .claude — even though .claude/commands is "expanded",
        // it should be hidden because .claude is collapsed
        state.toggle_expand(Path::new(".claude"));

        let visible = state.visible_files();
        // Only CLAUDE.md, .claude (collapsed), .envrc
        assert_eq!(visible.len(), 3);
        assert!(
            visible
                .iter()
                .all(|f| f.path.as_path() != Path::new(".claude/commands"))
        );
        assert!(
            visible
                .iter()
                .all(|f| f.path.as_path() != Path::new(".claude/commands/test.md"))
        );
    }

    #[test]
    fn test_toggle_nested_directory_selects_its_children() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());
        state.selections.clear();

        // Move cursor to .claude/commands directory (index 3 when fully expanded)
        for _ in 0..3 {
            state.cursor_down();
        }
        assert_eq!(
            state.visible_files()[state.cursor].path,
            PathBuf::from(".claude/commands")
        );

        // Toggle: should select .claude/commands and its child test.md
        state.toggle_current();

        assert!(
            state
                .selections
                .contains(&PathBuf::from(".claude/commands"))
        );
        assert!(
            state
                .selections
                .contains(&PathBuf::from(".claude/commands/test.md"))
        );
        // Should NOT have selected .claude/settings.json (sibling, not child)
        assert!(
            !state
                .selections
                .contains(&PathBuf::from(".claude/settings.json"))
        );
    }

    #[test]
    fn test_collapse_current_on_nested_child_navigates_to_immediate_parent() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // Move cursor to .claude/commands/test.md (index 4 when fully expanded)
        for _ in 0..4 {
            state.cursor_down();
        }
        assert_eq!(
            state.visible_files()[state.cursor].path,
            PathBuf::from(".claude/commands/test.md")
        );

        // Collapse current (child file) should navigate to parent .claude/commands
        state.collapse_current();
        assert_eq!(
            state.visible_files()[state.cursor].path,
            PathBuf::from(".claude/commands")
        );
    }

    // Note: Tests for env var handling are skipped because set_var/remove_var
    // are unsafe in Rust 2024 edition. The is_interactive_returns_false_in_tests
    // test verifies the test detection path works correctly.

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

    #[test]
    fn test_expand_current_on_already_expanded_directory() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // .claude is already expanded; move cursor to it
        state.cursor_down();
        assert_eq!(
            state.visible_files()[state.cursor].path,
            PathBuf::from(".claude")
        );

        let expanded_before = state.expanded_dirs.len();
        state.expand_current(); // no-op since already expanded
        assert_eq!(state.expanded_dirs.len(), expanded_before);
    }

    #[test]
    fn test_collapse_current_on_top_level_file_is_noop() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());

        // Cursor on CLAUDE.md (no parent_dir)
        assert_eq!(state.cursor, 0);
        state.collapse_current();
        // Should not crash, cursor stays at 0
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_dir_selection_state_empty_dir() {
        // Directory with no descendants
        let files = vec![
            DetectedFile {
                path: PathBuf::from("CLAUDE.md"),
                category: FileCategory::AiConfig,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
            DetectedFile {
                path: PathBuf::from(".empty-dir"),
                category: FileCategory::AiConfigDirectory,
                preselected: true,
                depth: 0,
                parent_dir: None,
            },
        ];
        let state = SelectionState::new(files, HashSet::new());

        // Directory with no children returns None
        assert_eq!(
            state.dir_selection_state(Path::new(".empty-dir")),
            DirSelectionState::None
        );
    }

    #[test]
    fn test_descendants_of_nonexistent_dir() {
        let files = make_test_files_with_children();
        let state = SelectionState::new(files, HashSet::new());

        // Non-existent directory has no descendants
        let descendants = state.descendants_of(Path::new("nonexistent-dir"));
        assert!(descendants.is_empty());
    }

    #[test]
    fn test_all_ancestors_expanded_root_level() {
        let files = make_test_files_with_children();
        let state = SelectionState::new(files, HashSet::new());

        // Root-level files (parent_dir: None) always have all ancestors expanded
        assert!(state.all_ancestors_expanded(&state.all_files[0])); // CLAUDE.md
        assert!(state.all_ancestors_expanded(&state.all_files[5])); // .envrc
    }

    #[test]
    fn test_resolve_paths_no_selections() {
        let files = make_test_files_with_children();
        let mut state = SelectionState::new(files, HashSet::new());
        state.selections.clear();

        let resolved = state.resolve_selected_paths();
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_flat_state_new_excludes_disabled_from_preselection() {
        let items = vec![
            SelectableItem {
                id: "a".into(),
                label: "Alpha".into(),
                description: None,
                preselected: true,
                disabled: true,
            },
            SelectableItem {
                id: "b".into(),
                label: "Beta".into(),
                description: None,
                preselected: true,
                disabled: false,
            },
        ];
        let state = FlatSelectionState::new(items);
        assert!(!state.selections.contains("a")); // disabled, even though preselected
        assert!(state.selections.contains("b"));
    }

    #[test]
    fn test_flat_state_visible_items_empty_query() {
        let items = vec![
            SelectableItem {
                id: "a".into(),
                label: "Alpha".into(),
                description: None,
                preselected: false,
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
        assert_eq!(state.visible_items().len(), 2);
    }

    #[test]
    fn test_flat_state_visible_items_case_insensitive() {
        let items = vec![
            SelectableItem {
                id: "a".into(),
                label: "ALPHA".into(),
                description: None,
                preselected: false,
                disabled: false,
            },
            SelectableItem {
                id: "b".into(),
                label: "beta".into(),
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
    fn test_flat_state_toggle_on_and_off() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: false,
        }];
        let mut state = FlatSelectionState::new(items);
        assert!(!state.selections.contains("a"));

        state.toggle_selection(0);
        assert!(state.selections.contains("a"));

        state.toggle_selection(0);
        assert!(!state.selections.contains("a"));
    }

    #[test]
    fn test_flat_state_toggle_out_of_bounds() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: false,
        }];
        let mut state = FlatSelectionState::new(items);
        // Out of bounds should not panic
        state.toggle_selection(5);
        assert!(!state.selections.contains("a"));
    }

    #[test]
    fn test_flat_state_select_all_visible_deselects_when_all_selected() {
        let items = vec![
            SelectableItem {
                id: "a".into(),
                label: "Alpha".into(),
                description: None,
                preselected: false,
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
        let mut state = FlatSelectionState::new(items);
        // Select all
        state.select_all_visible();
        assert!(state.selections.contains("a"));
        assert!(state.selections.contains("b"));

        // Select all again should deselect all (toggle behavior)
        state.select_all_visible();
        assert!(!state.selections.contains("a"));
        assert!(!state.selections.contains("b"));
    }

    #[test]
    fn test_flat_state_clamp_cursor_empty() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: false,
        }];
        let mut state = FlatSelectionState::new(items);
        state.cursor = 5;
        state.search_query = "nonexistent".into();
        state.clamp_cursor();
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_flat_state_clamp_cursor_within_range() {
        let items = vec![
            SelectableItem {
                id: "a".into(),
                label: "Alpha".into(),
                description: None,
                preselected: false,
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
        let mut state = FlatSelectionState::new(items);
        state.cursor = 10;
        state.clamp_cursor();
        assert_eq!(state.cursor, 1); // clamped to len - 1
    }

    #[test]
    fn test_adjust_flat_scroll_cursor_above_viewport() {
        let items: Vec<_> = (0..20)
            .map(|i| SelectableItem {
                id: format!("item{i}"),
                label: format!("Item {i}"),
                description: None,
                preselected: false,
                disabled: false,
            })
            .collect();
        let mut state = FlatSelectionState::new(items);
        state.scroll_offset = 10;
        state.cursor = 5;
        adjust_flat_scroll(&mut state);
        assert_eq!(state.scroll_offset, 5);
    }

    #[test]
    fn test_adjust_flat_scroll_cursor_below_viewport() {
        let items: Vec<_> = (0..20)
            .map(|i| SelectableItem {
                id: format!("item{i}"),
                label: format!("Item {i}"),
                description: None,
                preselected: false,
                disabled: false,
            })
            .collect();
        let mut state = FlatSelectionState::new(items);
        state.scroll_offset = 0;
        state.cursor = 18;
        adjust_flat_scroll(&mut state);
        // MAX_VISIBLE_ITEMS = 15, so scroll_offset = 18 - 15 + 1 = 4
        assert_eq!(state.scroll_offset, 4);
    }

    #[test]
    fn test_handle_flat_search_key_esc_returns_true() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: false,
        }];
        let mut state = FlatSelectionState::new(items);
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());
        let exit_search = handle_flat_search_key(key, &mut state);
        assert!(exit_search);
    }

    #[test]
    fn test_handle_flat_search_key_enter_returns_true() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: false,
        }];
        let mut state = FlatSelectionState::new(items);
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        let exit_search = handle_flat_search_key(key, &mut state);
        assert!(exit_search);
    }

    #[test]
    fn test_handle_flat_search_key_char_appends() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: false,
        }];
        let mut state = FlatSelectionState::new(items);
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty());
        let exit_search = handle_flat_search_key(key, &mut state);
        assert!(!exit_search);
        assert_eq!(state.search_query, "x");
    }

    #[test]
    fn test_handle_flat_search_key_backspace_pops() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: false,
        }];
        let mut state = FlatSelectionState::new(items);
        state.search_query = "ab".into();
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty());
        let exit_search = handle_flat_search_key(key, &mut state);
        assert!(!exit_search);
        assert_eq!(state.search_query, "a");
    }

    #[test]
    fn test_handle_flat_search_key_ctrl_c_clears() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: None,
            preselected: false,
            disabled: false,
        }];
        let mut state = FlatSelectionState::new(items);
        state.search_query = "hello".into();
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let exit_search = handle_flat_search_key(key, &mut state);
        assert!(exit_search);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn test_flat_state_with_descriptions() {
        let items = vec![SelectableItem {
            id: "a".into(),
            label: "Alpha".into(),
            description: Some("already applied".into()),
            preselected: false,
            disabled: true,
        }];
        let state = FlatSelectionState::new(items);
        assert_eq!(state.items[0].description, Some("already applied".into()));
        assert!(state.items[0].disabled);
    }

    #[test]
    fn test_flat_selection_result_fields() {
        let result = FlatSelectionResult {
            selected_ids: vec!["a".into(), "b".into()],
            cancelled: false,
        };
        assert_eq!(result.selected_ids.len(), 2);
        assert!(!result.cancelled);

        let cancelled = FlatSelectionResult {
            selected_ids: Vec::new(),
            cancelled: true,
        };
        assert!(cancelled.cancelled);
        assert!(cancelled.selected_ids.is_empty());
    }

    #[test]
    fn test_flat_selection_config_fields() {
        let config = FlatSelectionConfig {
            prompt: "Choose overlays".into(),
        };
        assert_eq!(config.prompt, "Choose overlays");
    }
}
