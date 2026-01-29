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

/// Format a number in a human-readable way (e.g., 1.2K, 3.5M).
pub fn humanize_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Result of the interactive selection process.
pub struct SelectionResult {
    /// Files that were selected by the user.
    pub selected_files: Vec<PathBuf>,
    /// Whether the selection was cancelled.
    pub cancelled: bool,
}

/// Configuration for the selection UI.
pub struct SelectionConfig {
    /// Prompt text shown at the top.
    pub prompt: String,
    /// Categories to hide by default.
    pub default_hidden_categories: HashSet<FileCategory>,
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

/// Internal state for the selection UI.
struct SelectionState {
    /// All files available for selection.
    all_files: Vec<DetectedFile>,
    /// Currently selected file paths (persists across filter changes).
    selections: HashSet<PathBuf>,
    /// Currently visible categories.
    visible_categories: HashSet<FileCategory>,
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
        let mut visible = HashSet::new();
        visible.insert(FileCategory::AiConfig);
        visible.insert(FileCategory::AiConfigDirectory);
        visible.insert(FileCategory::Gitignored);
        visible.insert(FileCategory::Untracked);

        for cat in hidden_categories {
            visible.remove(&cat);
        }

        // Pre-select files that are marked as preselected
        let selections: HashSet<PathBuf> = files
            .iter()
            .filter(|f| f.preselected)
            .map(|f| f.path.clone())
            .collect();

        Self {
            all_files: files,
            selections,
            visible_categories: visible,
            search_query: String::new(),
            mode: Mode::Selection,
            cursor: 0,
            scroll_offset: 0,
        }
    }

    /// Get files that are currently visible (match category filter and search).
    fn visible_files(&self) -> Vec<&DetectedFile> {
        self.all_files
            .iter()
            .filter(|f| self.visible_categories.contains(&f.category))
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

    /// Check if any filters are active.
    fn has_active_filters(&self) -> bool {
        !self.search_query.is_empty() || self.visible_categories.len() < 4 // Not all categories visible
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
    fn toggle_current(&mut self) {
        let visible = self.visible_files();
        if let Some(file) = visible.get(self.cursor) {
            let path = file.path.clone();
            self.toggle_selection(&path);
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

        for cat in &[
            FileCategory::AiConfig,
            FileCategory::AiConfigDirectory,
            FileCategory::Gitignored,
            FileCategory::Untracked,
        ] {
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
    fn adjust_scroll(&mut self) {
        let max_visible = 15; // Max files to show at once
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
}

/// Run the interactive file selection UI.
///
/// Returns the selected files, or a cancelled result if the user aborts.
///
/// # Non-TTY Fallback
///
/// If stdin is not a TTY (e.g., piped input), this function falls back to
/// returning all preselected files (AI configs) without showing the UI.
pub fn select_files(
    files: &[DetectedFile],
    config: SelectionConfig,
) -> anyhow::Result<SelectionResult> {
    // Non-TTY fallback: return preselected files
    if !atty_is_interactive() {
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

    let mut state = SelectionState::new(files.to_vec(), config.default_hidden_categories);

    // Enter raw mode for keyboard input
    terminal::enable_raw_mode()?;
    let result = run_selection_loop(&mut state, &config.prompt);
    terminal::disable_raw_mode()?;

    // Restore terminal state for subsequent prompts
    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::Show,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::All),
    )?;
    // Print newline to ensure clean state for dialoguer
    println!();
    stdout.flush()?;

    result
}

/// Check if the terminal is interactive.
///
/// Returns false in these cases:
/// - stdin or stdout is not a TTY
/// - Running in a CI environment (CI env var is set)
/// - Running as a cargo test binary (executable in target/*/deps/)
/// - TERM is unset or "dumb"
/// - REPOVERLAY_NON_INTERACTIVE env var is set
fn atty_is_interactive() -> bool {
    use std::io::IsTerminal;

    // Explicit non-interactive override
    if std::env::var("REPOVERLAY_NON_INTERACTIVE").is_ok() {
        return false;
    }

    // CI environments are never interactive
    if std::env::var("CI").is_ok() {
        return false;
    }

    // Detect cargo test environment by checking executable path
    // Test binaries live in target/debug/deps/ or target/release/deps/
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.to_string_lossy();
        if exe_str.contains("target") && exe_str.contains("deps") {
            return false;
        }
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
                            selected_files: state.selections.iter().cloned().collect(),
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
        KeyCode::Char('3') => state.toggle_category(FileCategory::Gitignored),
        KeyCode::Char('4') => state.toggle_category(FileCategory::Untracked),

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

    // Gitignored
    let gi_visible = state.visible_categories.contains(&FileCategory::Gitignored);
    let (gi_sel, gi_total) = counts.get(&FileCategory::Gitignored).unwrap_or(&(0, 0));
    render_category_toggle(
        stdout,
        "3",
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
        "4",
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
    let max_visible = 15;

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

        // Checkbox
        if is_selected {
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
            FileCategory::Gitignored => Color::Yellow,
            FileCategory::Untracked => Color::Blue,
        };

        // File path (highlight search match if any)
        // Add trailing slash for directories
        let path_str = if file.category == FileCategory::AiConfigDirectory {
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
        render_key_hint(stdout, "Ctrl+C", "clear")?;
        Ok(())
    } else {
        render_key_hint(stdout, "↑↓", "move")?;
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
            },
            DetectedFile {
                path: PathBuf::from(".claude/settings.json"),
                category: FileCategory::AiConfig,
                preselected: true,
            },
            DetectedFile {
                path: PathBuf::from(".envrc"),
                category: FileCategory::Gitignored,
                preselected: false,
            },
            DetectedFile {
                path: PathBuf::from(".env.local"),
                category: FileCategory::Gitignored,
                preselected: false,
            },
            DetectedFile {
                path: PathBuf::from("scratch.txt"),
                category: FileCategory::Untracked,
                preselected: false,
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
        assert_eq!(humanize_count(999999), "1000.0K");
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
            });
        }

        let mut state = SelectionState::new(files, HashSet::new());

        // Initially at top
        assert_eq!(state.cursor, 0);
        assert_eq!(state.scroll_offset, 0);

        // Move down past visible area (max_visible = 15)
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
            },
            DetectedFile {
                path: PathBuf::from(".claude"),
                category: FileCategory::AiConfigDirectory,
                preselected: true,
            },
            DetectedFile {
                path: PathBuf::from(".cursor"),
                category: FileCategory::AiConfigDirectory,
                preselected: true,
            },
            DetectedFile {
                path: PathBuf::from(".envrc"),
                category: FileCategory::Gitignored,
                preselected: false,
            },
            DetectedFile {
                path: PathBuf::from("notes.txt"),
                category: FileCategory::Untracked,
                preselected: false,
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
        assert_eq!(state.visible_categories.len(), 4);
    }
}
