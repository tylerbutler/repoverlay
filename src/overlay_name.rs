//! Newtype wrapper for normalized overlay names.
//!
//! Ensures overlay names are consistently formatted for state lookups and display.

use std::fmt;

/// A normalized overlay name, as stored in `.ccl` file stems.
///
/// This newtype prevents accidental comparison between overlay names
/// and other string types (e.g., full three-part paths like `"org/repo/name"`).
///
/// An `OverlayName` must be a simple name (no path separators).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OverlayName(String);

impl OverlayName {
    /// Create a new `OverlayName` from a string.
    ///
    /// The name must be a simple overlay name (e.g., `"my-overlay"`),
    /// not a path like `"org/repo/name"`.
    pub(crate) fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        debug_assert!(
            !name.contains('/'),
            "OverlayName must not contain path separators: {name}"
        );
        Self(name)
    }

    /// Get the underlying string slice.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OverlayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for OverlayName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for OverlayName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for OverlayName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl crate::selection::ToSelectableItem for OverlayName {
    fn to_selectable_item(&self, target: &std::path::Path) -> crate::selection::SelectableItem {
        let description = crate::load_overlay_state(target, self.as_str())
            .ok()
            .map(|state| {
                format!(
                    "last updated {}",
                    crate::state::format_relative_time(&state.applied_at)
                )
            });
        crate::selection::SelectableItem {
            id: self.to_string(),
            label: self.to_string(),
            description,
            preselected: false,
            disabled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_name_display() {
        let name = OverlayName::new("my-overlay");
        assert_eq!(name.to_string(), "my-overlay");
    }

    #[test]
    fn overlay_name_equality() {
        let a = OverlayName::new("foo");
        let b = OverlayName::new("foo");
        let c = OverlayName::new("bar");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn overlay_name_as_str() {
        let name = OverlayName::new("test");
        assert_eq!(name.as_str(), "test");
    }

    #[test]
    fn overlay_name_eq_str() {
        let name = OverlayName::new("foo");
        assert!(name == "foo");
        assert!(name != "bar");
    }

    #[test]
    #[should_panic(expected = "must not contain path separators")]
    fn overlay_name_rejects_paths_in_debug() {
        let _ = OverlayName::new("org/repo/name");
    }

    #[test]
    fn overlay_name_ordering() {
        let a = OverlayName::new("alpha");
        let b = OverlayName::new("beta");
        let c = OverlayName::new("gamma");

        assert!(a < b);
        assert!(b < c);
        assert!(a < c);

        let mut names = vec![c.clone(), a.clone(), b.clone()];
        names.sort();
        assert_eq!(names, vec![a, b, c]);
    }

    #[test]
    fn overlay_name_as_ref_str() {
        let name = OverlayName::new("my-overlay");
        let s: &str = name.as_ref();
        assert_eq!(s, "my-overlay");
    }
}
