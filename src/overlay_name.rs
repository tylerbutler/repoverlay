//! Newtype wrapper for normalized overlay names.
//!
//! Ensures overlay names are consistently formatted for state lookups and display.

use std::fmt;

use anyhow::bail;

/// A normalized overlay name, as stored in `.ccl` file stems.
///
/// This newtype prevents accidental comparison between overlay names
/// and other string types (e.g., full three-part paths like `"org/repo/name"`).
///
/// An `OverlayName` must be a simple name (no path separators).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OverlayName(String);

impl OverlayName {
    /// Create a new `OverlayName` from a string that is already known to be valid.
    ///
    /// Use this only when the name comes from a trusted source (e.g., file stems
    /// from directory listing). For user-provided input, use [`try_new`](Self::try_new).
    pub(crate) fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        debug_assert!(
            !name.contains('/') && !name.contains('\\'),
            "OverlayName must not contain path separators: {name}"
        );
        Self(name)
    }

    /// Create a new `OverlayName` from user-provided input, validating that it
    /// contains no path separators.
    ///
    /// Returns an error if the name contains `/` or `\`.
    pub(crate) fn try_new(name: impl Into<String>) -> anyhow::Result<Self> {
        let name = name.into();
        if name.contains('/') || name.contains('\\') {
            bail!("Overlay name must not contain path separators: {name}");
        }
        Ok(Self(name))
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
    fn try_new_rejects_forward_slash() {
        let result = OverlayName::try_new("org/repo/name");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("path separators"),
            "expected path separator error, got: {msg}"
        );
    }

    #[test]
    fn try_new_rejects_backslash() {
        let result = OverlayName::try_new(r"org\repo\name");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("path separators"),
            "expected path separator error, got: {msg}"
        );
    }

    #[test]
    fn try_new_accepts_valid_name() {
        let result = OverlayName::try_new("my-overlay");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_str(), "my-overlay");
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
