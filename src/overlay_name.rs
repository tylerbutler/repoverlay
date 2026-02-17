//! Newtype wrapper for normalized overlay names.

use std::fmt;

/// A normalized overlay name, as stored in `.ccl` file stems.
///
/// This newtype prevents accidental comparison between overlay names
/// and other string types (e.g., full three-part paths like `"org/repo/name"`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OverlayName(String);

impl OverlayName {
    /// Create a new `OverlayName` from a string.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Get the underlying string slice.
    pub fn as_str(&self) -> &str {
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

impl From<String> for OverlayName {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for OverlayName {
    fn from(s: &str) -> Self {
        Self(s.to_string())
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
}
