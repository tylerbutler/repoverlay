//! Input reference parsing for repoverlay.
//!
//! Parses user input strings into structured source references for resolution.

use std::path::PathBuf;

use crate::github::GitHubSource;

/// Parsed source reference from user input.
///
/// This enum represents all valid input formats for overlay sources,
/// enabling the resolution layer to handle each type appropriately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceReference {
    /// GitHub URL (e.g., `https://github.com/owner/repo`)
    GitHubUrl(String),

    /// Local filesystem path (e.g., `./overlay` or `/path/to/overlay`)
    LocalPath {
        path: PathBuf,
        /// True if the path was ambiguous (no `./` prefix) and a deprecation
        /// warning should be shown.
        needs_prefix_warning: bool,
    },

    /// Three-part reference: `owner/repo/overlay`
    /// Resolves directly to a specific overlay.
    ThreePart {
        owner: String,
        repo: String,
        overlay: String,
    },

    /// Two-part reference: `owner/repo`
    /// Triggers browse mode to select an overlay interactively.
    TwoPart { owner: String, repo: String },

    /// One-part reference: `username`
    /// Expands to `username/repo-overlays` (convention), then browse mode.
    OnePart { username: String },
}

impl SourceReference {
    /// Parse user input into a structured source reference.
    ///
    /// Resolution order:
    /// 1. GitHub URL (`https://github.com/...`)
    /// 2. Explicit local path (`./path`, `/path`, or `~/path`)
    /// 3. Three-part reference (`owner/repo/overlay`)
    /// 4. Two-part reference (`owner/repo`) - Phase B
    /// 5. One-part reference (`username`) - Phase C
    /// 6. Ambiguous local path (existing directory without `./` prefix)
    ///
    /// # Ambiguous Local Paths
    ///
    /// If input doesn't match any structured format but exists as a local path,
    /// it's treated as a local path with `needs_prefix_warning = true` to indicate
    /// that users should use `./` prefix in the future.
    #[must_use]
    pub(crate) fn parse(input: &str) -> Self {
        // 1. Check for GitHub URL
        if GitHubSource::is_github_url(input) {
            return Self::GitHubUrl(input.to_string());
        }

        // 2. Check for explicit local path indicators
        if input.starts_with("./") || input.starts_with('/') {
            return Self::LocalPath {
                path: PathBuf::from(input),
                needs_prefix_warning: false,
            };
        }

        // Handle tilde expansion for home directory
        if input == "~" {
            if let Some(home) = dirs::home_dir() {
                return Self::LocalPath {
                    path: home,
                    needs_prefix_warning: false,
                };
            }
        } else if let Some(rest) = input.strip_prefix("~/")
            && let Some(home) = dirs::home_dir()
        {
            return Self::LocalPath {
                path: home.join(rest),
                needs_prefix_warning: false,
            };
        }

        // 3. Check for local path existence BEFORE structured reference parsing.
        // This preserves backward compatibility: existing local paths take precedence.
        // TODO: In a future version, require `./` prefix for local paths to avoid ambiguity.
        let path = PathBuf::from(input);
        if path.exists() {
            return Self::LocalPath {
                path,
                needs_prefix_warning: true,
            };
        }

        // 4. Parse by slash count for structured references
        let parts: Vec<&str> = input.split('/').collect();
        match parts.len() {
            3 => {
                // Three-part: owner/repo/overlay
                // Validate: must not contain URL schemes or empty parts
                if !input.contains("://") && parts.iter().all(|p| !p.is_empty()) {
                    return Self::ThreePart {
                        owner: parts[0].to_string(),
                        repo: parts[1].to_string(),
                        overlay: parts[2].to_string(),
                    };
                }
            }
            2 => {
                // Two-part: owner/repo (Phase B)
                if parts.iter().all(|p| !p.is_empty()) {
                    return Self::TwoPart {
                        owner: parts[0].to_string(),
                        repo: parts[1].to_string(),
                    };
                }
            }
            1 => {
                // One-part: username (Phase C)
                if !input.is_empty() {
                    return Self::OnePart {
                        username: input.to_string(),
                    };
                }
            }
            _ => {
                // 4+ parts: not a valid structured reference
            }
        }

        // 6. Final fallback: treat as local path (may not exist yet)
        // This handles cases like "some-overlay" that don't exist locally
        // and aren't valid structured references
        Self::LocalPath {
            path: PathBuf::from(input),
            needs_prefix_warning: false,
        }
    }

    /// Check if this reference requires a deprecation warning about local path syntax.
    #[must_use]
    #[allow(dead_code)] // Available for future use
    pub(crate) const fn needs_local_path_warning(&self) -> bool {
        matches!(
            self,
            Self::LocalPath {
                needs_prefix_warning: true,
                ..
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_github_url() {
        let result = SourceReference::parse("https://github.com/owner/repo");
        assert_eq!(
            result,
            SourceReference::GitHubUrl("https://github.com/owner/repo".to_string())
        );
    }

    #[test]
    fn parse_github_url_with_tree() {
        let result =
            SourceReference::parse("https://github.com/owner/repo/tree/main/overlays/test");
        assert!(matches!(result, SourceReference::GitHubUrl(_)));
    }

    #[test]
    fn parse_explicit_local_path_dot_slash() {
        let result = SourceReference::parse("./my-overlay");
        assert_eq!(
            result,
            SourceReference::LocalPath {
                path: PathBuf::from("./my-overlay"),
                needs_prefix_warning: false,
            }
        );
    }

    #[test]
    fn parse_explicit_local_path_absolute() {
        let result = SourceReference::parse("/absolute/path/to/overlay");
        assert_eq!(
            result,
            SourceReference::LocalPath {
                path: PathBuf::from("/absolute/path/to/overlay"),
                needs_prefix_warning: false,
            }
        );
    }

    #[test]
    fn parse_three_part_reference() {
        let result = SourceReference::parse("microsoft/FluidFramework/claude-config");
        assert_eq!(
            result,
            SourceReference::ThreePart {
                owner: "microsoft".to_string(),
                repo: "FluidFramework".to_string(),
                overlay: "claude-config".to_string(),
            }
        );
    }

    #[test]
    fn parse_two_part_reference() {
        let result = SourceReference::parse("owner/repo");
        assert_eq!(
            result,
            SourceReference::TwoPart {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
            }
        );
    }

    #[test]
    fn parse_one_part_nonexistent() {
        // When path doesn't exist, treat as username (Phase C)
        let result = SourceReference::parse("someusername");
        assert_eq!(
            result,
            SourceReference::OnePart {
                username: "someusername".to_string(),
            }
        );
    }

    #[test]
    fn parse_three_part_empty_parts_invalid() {
        // Empty parts should not match three-part
        let result = SourceReference::parse("owner//overlay");
        // Falls through to local path
        assert!(matches!(result, SourceReference::LocalPath { .. }));
    }

    #[test]
    fn parse_url_like_not_github() {
        // URLs that aren't GitHub should not be parsed as three-part
        let result = SourceReference::parse("https://example.com/a/b/c");
        // Not a GitHub URL, has slashes but contains "://" so not three-part
        assert!(matches!(result, SourceReference::LocalPath { .. }));
    }

    #[test]
    fn parse_bare_tilde() {
        // Bare tilde should expand to home directory
        let result = SourceReference::parse("~");
        if let SourceReference::LocalPath {
            path,
            needs_prefix_warning,
        } = result
        {
            // Should expand to home dir (if available)
            if let Some(home) = dirs::home_dir() {
                assert_eq!(path, home);
            }
            assert!(!needs_prefix_warning);
        } else {
            panic!("Expected LocalPath for bare tilde");
        }
    }

    #[test]
    fn parse_github_url_case_insensitive() {
        // GitHub URLs should be recognized regardless of case
        let result = SourceReference::parse("https://GitHub.com/owner/repo");
        assert!(matches!(result, SourceReference::GitHubUrl(_)));

        let result = SourceReference::parse("HTTPS://GITHUB.COM/owner/repo");
        assert!(matches!(result, SourceReference::GitHubUrl(_)));
    }

    #[test]
    fn parse_empty_input() {
        // Empty input should not panic
        let result = SourceReference::parse("");
        // Empty string doesn't match structured formats, falls to local path
        assert!(matches!(result, SourceReference::LocalPath { .. }));
    }

    #[test]
    fn parse_tilde_with_path() {
        // ~/some/path should expand tilde
        let result = SourceReference::parse("~/overlays/test");
        if let SourceReference::LocalPath {
            path,
            needs_prefix_warning,
        } = result
        {
            if let Some(home) = dirs::home_dir() {
                assert_eq!(path, home.join("overlays/test"));
            }
            assert!(!needs_prefix_warning);
        } else {
            panic!("Expected LocalPath for tilde path");
        }
    }

    #[test]
    fn parse_four_parts_invalid() {
        // Four or more parts should not match structured formats
        let result = SourceReference::parse("a/b/c/d");
        assert!(matches!(result, SourceReference::LocalPath { .. }));

        let result = SourceReference::parse("a/b/c/d/e/f");
        assert!(matches!(result, SourceReference::LocalPath { .. }));
    }

    #[test]
    fn parse_two_part_empty_parts_invalid() {
        // Empty parts should not match two-part
        let result = SourceReference::parse("/repo");
        // Starts with / so it's an absolute path
        assert!(matches!(result, SourceReference::LocalPath { .. }));

        let result = SourceReference::parse("owner/");
        // Has empty second part
        assert!(matches!(result, SourceReference::LocalPath { .. }));
    }

    #[test]
    fn parse_with_whitespace() {
        // Whitespace in input is preserved (not trimmed)
        // " owner/repo " splits into [" owner", "repo "] - 2 non-empty parts
        // so it matches as TwoPart with spaces in the values
        let result = SourceReference::parse(" owner/repo ");
        if let SourceReference::TwoPart { owner, repo } = result {
            assert_eq!(owner, " owner");
            assert_eq!(repo, "repo ");
        } else {
            panic!("Expected TwoPart with whitespace preserved");
        }
    }

    #[test]
    fn needs_local_path_warning_returns_false_for_explicit_paths() {
        let reference = SourceReference::LocalPath {
            path: PathBuf::from("./test"),
            needs_prefix_warning: false,
        };
        assert!(!reference.needs_local_path_warning());
    }

    #[test]
    fn needs_local_path_warning_returns_true_for_ambiguous_paths() {
        let reference = SourceReference::LocalPath {
            path: PathBuf::from("test"),
            needs_prefix_warning: true,
        };
        assert!(reference.needs_local_path_warning());
    }

    #[test]
    fn needs_local_path_warning_returns_false_for_non_local_path() {
        let reference = SourceReference::GitHubUrl("https://github.com/a/b".to_string());
        assert!(!reference.needs_local_path_warning());

        let reference = SourceReference::ThreePart {
            owner: "a".to_string(),
            repo: "b".to_string(),
            overlay: "c".to_string(),
        };
        assert!(!reference.needs_local_path_warning());
    }

    #[test]
    fn parse_http_github_url() {
        // HTTP (not HTTPS) should also work
        let result = SourceReference::parse("http://github.com/owner/repo");
        assert!(matches!(result, SourceReference::GitHubUrl(_)));
    }

    #[test]
    fn source_reference_clone_and_eq() {
        // Test Clone and PartialEq derives
        let original = SourceReference::ThreePart {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            overlay: "overlay".to_string(),
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn source_reference_debug() {
        // Test Debug derive
        let reference = SourceReference::OnePart {
            username: "user".to_string(),
        };
        let debug_str = format!("{reference:?}");
        assert!(debug_str.contains("OnePart"));
        assert!(debug_str.contains("user"));
    }
}
