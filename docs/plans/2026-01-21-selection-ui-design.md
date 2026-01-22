# Selection UI Design for Overlay Creation

## Overview

Redesign the file selection UI in `repoverlay create` to make it easier to identify selected files, filter by category, search, and bulk select.

## Requirements

- Easier visual identification of selected files (color + symbols + counts)
- Toggle visibility of file categories (AI Config, Gitignored, Untracked)
- Search/filter files by path
- "Select all" with confirmation when filters are active

## Interaction Model

```
┌─────────────────────────────────────────────────────────┐
│ Select files to include in overlay                      │
│                                                         │
│ Categories: [1] AI Config (3) [2] Gitignored (5) [3] UT │
│ Search: _                                               │
│ Selected: 2 AI Config, 0 Gitignored, 0 Untracked        │
├─────────────────────────────────────────────────────────┤
│ [✓] CLAUDE.md                                           │
│ [✓] .claude/settings.json                               │
│ [ ] .claude/commands/test.md                            │
│ [ ] .envrc                                              │
│ [ ] .env.local                                          │
└─────────────────────────────────────────────────────────┘
  Space: toggle | Enter: confirm | a: select all | 1-3: toggle category
```

**Key bindings:**
- `1`, `2`, `3`: Toggle category visibility
- `/` or typing: Start search mode
- `a`: Select all (with confirmation if filters active)
- `Space`: Toggle individual selection
- `Enter`: Confirm and proceed
- `Esc`: Cancel/clear search

## Architecture

### New Module: `src/selection.rs`

```rust
/// Result of the interactive selection process
pub struct SelectionResult {
    pub selected_files: Vec<PathBuf>,
    pub cancelled: bool,
}

/// Configuration for the selection UI
pub struct SelectionConfig {
    pub prompt: String,
    pub default_hidden_categories: HashSet<FileCategory>,
}

/// Run the interactive file selection UI
pub fn select_files(
    files: &[DetectedFile],
    config: SelectionConfig,
) -> Result<SelectionResult>;
```

### Internal State

```rust
struct SelectionState {
    all_files: Vec<DetectedFile>,
    selections: HashSet<PathBuf>,        // persists across filters
    visible_categories: HashSet<FileCategory>,
    search_query: String,
}

impl SelectionState {
    fn visible_files(&self) -> Vec<&DetectedFile>;
    fn toggle_category(&mut self, cat: FileCategory);
    fn set_search(&mut self, query: &str);
    fn toggle_selection(&mut self, path: &Path);
    fn select_all_visible(&mut self);
    fn select_all(&mut self);
    fn selection_counts(&self) -> HashMap<FileCategory, (usize, usize)>;
}
```

## Rendering Approach

Loop-based UI using `dialoguer` with custom header rendering:

1. **Header section**: Category toggles with counts, search indicator, selection summary
2. **File list**: `dialoguer::MultiSelect` with filtered files and colored items
3. **Help line**: Key binding hints

```rust
loop {
    clear_screen();
    render_header(&state);

    match get_input_mode(&state) {
        Mode::Selection => {
            let result = show_multiselect(&state);
            match result {
                Action::Confirm(selections) => break Ok(selections),
                Action::Cancel => break Ok(cancelled()),
                Action::ToggleCategory(cat) => state.toggle_category(cat),
                Action::StartSearch => state.mode = Mode::Search,
                Action::SelectAll => handle_select_all(&mut state),
            }
        }
        Mode::Search => {
            let query = prompt_search(&state.search_query);
            state.set_search(&query);
        }
    }
}
```

## Select All Behavior

**No filters active:**
- Immediately select all files

**Filters active:**
- Prompt with two options showing counts:
  - "Visible files only (8 files)"
  - "All files (23 files)"

## Edge Cases

| Scenario | Behavior |
|----------|----------|
| No files discovered | Skip selection UI, show error |
| All files filtered out | Show "No files match filters" message |
| User cancels (Esc/Ctrl+C) | Return cancelled result |
| Non-TTY (piped input) | Fall back to preselected AI configs |

## Files to Modify

- `src/selection.rs` - NEW (~300 lines)
- `src/lib.rs` - Update `create_overlay` to use selection module (~20 lines)
- `tests/integration/create.rs` - Add non-TTY fallback test

## Testing Strategy

**Unit tests** for `SelectionState`:
- `toggle_category_hides_files`
- `search_filters_by_path`
- `selections_persist_across_filter_changes`
- `select_all_visible_respects_filters`

**Integration tests:**
- Non-TTY fallback behavior

**Manual testing:**
- Full interactive experience
