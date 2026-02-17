# Flat List Selector — Unified Overlay/Remove Selection UX

## Problem

Three different selection UIs exist with inconsistent look and interaction:

| UI | Library | Used by |
|----|---------|---------|
| Overlay picker | `dialoguer` MultiSelect | `apply` |
| File picker | Custom crossterm (~2300 lines) | `create`, `edit --interactive` |
| Remove picker | Manual `stdin::read_line` | `remove` |

The overlay picker and remove picker should use the same crossterm-based UI as the file picker for visual consistency.

## Design

### New data structures

```rust
pub struct SelectableItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub preselected: bool,
    pub disabled: bool,
}

pub struct FlatSelectionConfig {
    pub prompt: String,
    pub multi_select: bool,
}

pub struct FlatSelectionResult {
    pub selected_ids: Vec<String>,
    pub cancelled: bool,
}
```

### Visual layout

```
Select overlays to apply:

  / to search

  [✓] claude-overlay          (already applied)
  [ ] biome-overlay
> [ ] prettier-overlay
  [ ] eslint-overlay

  ↑↓ move  Space toggle  Enter confirm  / search  Esc cancel
```

- Disabled items render dimmed with "(already applied)" suffix
- Same keybindings as file picker: ↑↓/j/k, Space, Enter, Esc, /
- Same scrolling (15 visible, overflow indicators)
- No categories, no tree, no expand/collapse

### Shared rendering helpers

Extract from existing `selection.rs`:
- Checkbox glyphs (`[✓]`, `[ ]`, dimmed for disabled)
- Help bar rendering
- Scroll indicators (`↑ N more above` / `↓ N more below`)
- Cursor/highlight rendering
- Search input rendering

### Apply command changes

- Query current overlay state to identify already-applied overlays
- Pass `disabled: true` for those items
- Replace `dialoguer::MultiSelect` with `select_flat()`

### Remove command changes

- Build `SelectableItem` list from applied overlays
- Replace manual `stdin::read_line` menu with `select_flat()`
- Now supports multi-select (remove several overlays at once)

### What stays unchanged

- File picker (`select_files()`) and its tree-based UI remain a separate code path
- `is_interactive()` already public, shared as-is
