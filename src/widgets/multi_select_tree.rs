//! A multi-select tree widget built on top of `tui-tree-widget`.
//!
//! Adds multi-selection with tri-state checkboxes to `tui-tree-widget`'s
//! `Tree` and `TreeState`. Generic over the identifier type.

use std::collections::HashSet;
use std::hash::Hash;

use tui_tree_widget::TreeState;

/// Selection state of a node based on its descendants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckState {
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
pub(crate) struct MultiSelectTreeState<Id: Clone + Eq + Hash> {
    /// The underlying tree navigation/expand state.
    pub(crate) tree: TreeState<Id>,
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
    pub(crate) fn is_selected(&self, id: &Id) -> bool {
        self.selected.contains(id)
    }

    /// Toggle selection of a single item.
    pub(crate) fn toggle(&mut self, id: &Id) {
        if self.selected.contains(id) {
            self.selected.remove(id);
        } else {
            self.selected.insert(id.clone());
        }
    }

    /// Select a single item.
    pub(crate) fn select(&mut self, id: Id) {
        self.selected.insert(id);
    }

    /// Deselect a single item.
    pub(crate) fn deselect(&mut self, id: &Id) {
        self.selected.remove(id);
    }

    /// Select multiple items.
    pub(crate) fn select_many(&mut self, ids: impl IntoIterator<Item = Id>) {
        self.selected.extend(ids);
    }

    /// Deselect multiple items.
    pub(crate) fn deselect_many<'a>(&mut self, ids: impl IntoIterator<Item = &'a Id>)
    where
        Id: 'a,
    {
        for id in ids {
            self.selected.remove(id);
        }
    }

    /// Get the number of selected items.
    pub(crate) fn selected_count(&self) -> usize {
        self.selected.len()
    }

    /// Get an iterator over selected identifiers.
    pub(crate) fn selected_ids(&self) -> impl Iterator<Item = &Id> {
        self.selected.iter()
    }

    /// Clear all selections.
    pub(crate) fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Check the selection state of a node given its descendants.
    ///
    /// The caller provides the list of descendant IDs (not including the
    /// node itself). Returns `CheckState` based on how many are selected.
    pub(crate) fn check_state(&self, descendant_ids: &[Id]) -> CheckState {
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

#[cfg(test)]
mod tests {
    use super::*;

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
