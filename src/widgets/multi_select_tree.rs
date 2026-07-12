//! A multi-select tree widget built on top of `tui-tree-widget`.
//!
//! Adds multi-selection with tri-state checkboxes to `tui-tree-widget`'s
//! `Tree` and `TreeState`. Generic over the identifier type.

use std::collections::HashSet;
use std::hash::Hash;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::StatefulWidget;
use tui_tree_widget::{Tree, TreeItem, TreeState};

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

/// A node descriptor for building the multi-select tree.
///
/// Unlike `TreeItem` (whose text field is not publicly accessible),
/// this type keeps the display text available so the widget can
/// prepend checkbox symbols during rendering.
pub(crate) struct TreeNode<'a, Id> {
    /// Unique identifier for this node (among siblings).
    pub(crate) id: Id,
    /// Display text for this node.
    pub(crate) text: Line<'a>,
    /// Child nodes.
    pub(crate) children: Vec<Self>,
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

    /// Select a single item.
    pub(crate) fn select(&mut self, id: Id) {
        self.selected.insert(id);
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

/// Configuration for checkbox symbols.
#[derive(Debug, Clone)]
pub(crate) struct CheckboxSymbols {
    pub(crate) checked: &'static str,
    pub(crate) unchecked: &'static str,
    pub(crate) partial: &'static str,
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
pub(crate) struct CheckboxStyles {
    pub(crate) checked: Style,
    pub(crate) unchecked: Style,
    pub(crate) partial: Style,
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
pub(crate) type DescendantsFn<'a, Id> = Box<dyn Fn(&Id) -> Vec<Id> + 'a>;

/// A multi-select tree widget.
///
/// Accepts [`TreeNode`] descriptors and renders them as a tree with
/// tri-state checkboxes prepended to each node's text.
pub(crate) struct MultiSelectTree<'a, Id: Clone + Eq + Hash> {
    /// The tree node descriptors.
    nodes: &'a [TreeNode<'a, Id>],
    /// Highlight style for the focused node.
    highlight_style: Style,
    /// Checkbox symbols.
    checkbox_symbols: CheckboxSymbols,
    /// Checkbox styles.
    checkbox_styles: CheckboxStyles,
    /// Function to get descendant IDs for tri-state computation.
    /// If None, branch nodes show checked/unchecked based on their own ID only.
    descendants_fn: Option<DescendantsFn<'a, Id>>,
}

impl<'a, Id: Clone + Eq + Hash + 'a> MultiSelectTree<'a, Id> {
    /// Create a new multi-select tree from node descriptors.
    pub(crate) fn new(nodes: &'a [TreeNode<'a, Id>]) -> Self {
        Self {
            nodes,
            highlight_style: Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            checkbox_symbols: CheckboxSymbols::default(),
            checkbox_styles: CheckboxStyles::default(),
            descendants_fn: None,
        }
    }

    /// Set the function used to compute descendant IDs for tri-state checkboxes.
    pub(crate) fn descendants_fn(mut self, f: DescendantsFn<'a, Id>) -> Self {
        self.descendants_fn = Some(f);
        self
    }

    /// Build `TreeItem`s with checkbox prefixes from node descriptors.
    fn build_items(
        &self,
        nodes: &'a [TreeNode<'a, Id>],
        state: &MultiSelectTreeState<Id>,
    ) -> Vec<TreeItem<'a, Id>> {
        nodes
            .iter()
            .map(|node| self.build_item(node, state))
            .collect()
    }

    fn build_item(
        &self,
        node: &'a TreeNode<'a, Id>,
        state: &MultiSelectTreeState<Id>,
    ) -> TreeItem<'a, Id> {
        let check = self.resolve_check_state(node, state);
        let (symbol, style) = match check {
            CheckState::Checked => (self.checkbox_symbols.checked, self.checkbox_styles.checked),
            CheckState::Partial => (self.checkbox_symbols.partial, self.checkbox_styles.partial),
            CheckState::Unchecked => (
                self.checkbox_symbols.unchecked,
                self.checkbox_styles.unchecked,
            ),
        };

        // Build text with checkbox prefix
        let mut spans = vec![Span::styled(symbol, style)];
        spans.extend(node.text.spans.iter().cloned());
        let text_with_checkbox = Line::from(spans);

        // Recurse into children
        let children = self.build_items(&node.children, state);

        if children.is_empty() {
            TreeItem::new_leaf(node.id.clone(), text_with_checkbox)
        } else {
            TreeItem::new(node.id.clone(), text_with_checkbox, children)
                .expect("duplicate identifiers in children")
        }
    }

    /// Determine the checkbox state for a node.
    fn resolve_check_state(
        &self,
        node: &TreeNode<'a, Id>,
        state: &MultiSelectTreeState<Id>,
    ) -> CheckState {
        if !node.children.is_empty()
            && let Some(ref descendants_fn) = self.descendants_fn
        {
            let desc = descendants_fn(&node.id);
            return state.check_state(&desc);
        }
        // Leaf nodes or branch nodes without a descendants_fn: binary check
        if state.is_selected(&node.id) {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        }
    }
}

impl<'a, Id: Clone + Eq + Hash + 'a> StatefulWidget for MultiSelectTree<'a, Id> {
    type State = MultiSelectTreeState<Id>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let items = self.build_items(self.nodes, state);

        let tree = Tree::new(&items)
            .expect("duplicate identifiers")
            .highlight_style(self.highlight_style);

        StatefulWidget::render(tree, area, buf, &mut state.tree);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::buffer_to_string;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_nodes() -> Vec<TreeNode<'static, String>> {
        vec![
            TreeNode {
                id: "root".to_string(),
                text: Line::from("root/"),
                children: vec![
                    TreeNode {
                        id: "child-a".to_string(),
                        text: Line::from("a.txt"),
                        children: vec![],
                    },
                    TreeNode {
                        id: "child-b".to_string(),
                        text: Line::from("b.txt"),
                        children: vec![],
                    },
                ],
            },
            TreeNode {
                id: "lone".to_string(),
                text: Line::from("lone.txt"),
                children: vec![],
            },
        ]
    }

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
    fn clear_selection() {
        let mut state = MultiSelectTreeState::<String>::default();
        state.select("a".to_string());
        state.select("b".to_string());
        state.clear_selection();
        assert!(!state.is_selected(&"a".to_string()));
        assert!(!state.is_selected(&"b".to_string()));
    }

    #[test]
    fn renders_tree_with_checkboxes() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let nodes = make_nodes();
        let mut state = MultiSelectTreeState::<String>::default();
        state.select("child-a".to_string());
        state.tree.select(vec!["root".to_string()]);
        state.tree.open(vec!["root".to_string()]);

        terminal
            .draw(|frame| {
                let widget = MultiSelectTree::new(&nodes);
                frame.render_stateful_widget(widget, frame.area(), &mut state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        insta::assert_snapshot!(buffer_to_string(&buffer));
    }

    fn make_deep_nodes() -> Vec<TreeNode<'static, String>> {
        vec![TreeNode {
            id: "root".to_string(),
            text: Line::from("root/"),
            children: vec![
                TreeNode {
                    id: "sub".to_string(),
                    text: Line::from("sub/"),
                    children: vec![
                        TreeNode {
                            id: "deep-a".to_string(),
                            text: Line::from("a.txt"),
                            children: vec![],
                        },
                        TreeNode {
                            id: "deep-b".to_string(),
                            text: Line::from("b.txt"),
                            children: vec![],
                        },
                    ],
                },
                TreeNode {
                    id: "child-c".to_string(),
                    text: Line::from("c.txt"),
                    children: vec![],
                },
            ],
        }]
    }

    #[test]
    fn snapshot_tree_tristate_partial() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let nodes = make_nodes();
        let mut state = MultiSelectTreeState::<String>::default();
        state.select("child-a".to_string());
        state.tree.select(vec!["root".to_string()]);
        state.tree.open(vec!["root".to_string()]);

        terminal
            .draw(|frame| {
                let widget = MultiSelectTree::new(&nodes).descendants_fn(Box::new(
                    |id: &String| -> Vec<String> {
                        if id == "root" {
                            vec!["child-a".to_string(), "child-b".to_string()]
                        } else {
                            vec![]
                        }
                    },
                ));
                frame.render_stateful_widget(widget, frame.area(), &mut state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        insta::assert_snapshot!(buffer_to_string(&buffer));
    }

    #[test]
    fn snapshot_tree_tristate_all_checked() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let nodes = make_nodes();
        let mut state = MultiSelectTreeState::<String>::default();
        state.select("child-a".to_string());
        state.select("child-b".to_string());
        state.tree.select(vec!["root".to_string()]);
        state.tree.open(vec!["root".to_string()]);

        terminal
            .draw(|frame| {
                let widget = MultiSelectTree::new(&nodes).descendants_fn(Box::new(
                    |id: &String| -> Vec<String> {
                        if id == "root" {
                            vec!["child-a".to_string(), "child-b".to_string()]
                        } else {
                            vec![]
                        }
                    },
                ));
                frame.render_stateful_widget(widget, frame.area(), &mut state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        insta::assert_snapshot!(buffer_to_string(&buffer));
    }

    #[test]
    fn snapshot_tree_all_unchecked() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let nodes = make_nodes();
        let mut state = MultiSelectTreeState::<String>::default();
        state.tree.select(vec!["root".to_string()]);
        state.tree.open(vec!["root".to_string()]);

        terminal
            .draw(|frame| {
                let widget = MultiSelectTree::new(&nodes);
                frame.render_stateful_widget(widget, frame.area(), &mut state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        insta::assert_snapshot!(buffer_to_string(&buffer));
    }

    #[test]
    fn snapshot_tree_deep_nesting() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let nodes = make_deep_nodes();
        let mut state = MultiSelectTreeState::<String>::default();
        state.select("deep-a".to_string());
        state.tree.select(vec!["root".to_string()]);
        state.tree.open(vec!["root".to_string()]);
        state.tree.open(vec!["root".to_string(), "sub".to_string()]);

        terminal
            .draw(|frame| {
                let widget = MultiSelectTree::new(&nodes).descendants_fn(Box::new(
                    |id: &String| -> Vec<String> {
                        match id.as_str() {
                            "root" => vec![
                                "sub".to_string(),
                                "deep-a".to_string(),
                                "deep-b".to_string(),
                                "child-c".to_string(),
                            ],
                            "sub" => vec!["deep-a".to_string(), "deep-b".to_string()],
                            _ => vec![],
                        }
                    },
                ));
                frame.render_stateful_widget(widget, frame.area(), &mut state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        insta::assert_snapshot!(buffer_to_string(&buffer));
    }

    #[test]
    fn snapshot_tree_collapsed_parent() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let nodes = make_nodes();
        let mut state = MultiSelectTreeState::<String>::default();
        state.select("child-a".to_string());
        state.tree.select(vec!["root".to_string()]);
        // Don't open root — children should be hidden

        terminal
            .draw(|frame| {
                let widget = MultiSelectTree::new(&nodes);
                frame.render_stateful_widget(widget, frame.area(), &mut state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        insta::assert_snapshot!(buffer_to_string(&buffer));
    }
}
