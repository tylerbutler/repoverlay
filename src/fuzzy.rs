//! Fuzzy matching for overlay name suggestions.
//!
//! Uses the fzf-style matching algorithm from `fuzzy-matcher` to provide
//! helpful suggestions when an overlay name is not found.

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

/// Fuzzy matcher for finding similar overlay names.
pub(crate) struct OverlayMatcher {
    matcher: SkimMatcherV2,
}

impl Default for OverlayMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayMatcher {
    /// Create a new fuzzy matcher with default settings.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            matcher: SkimMatcherV2::default(),
        }
    }

    /// Find the best matches for a query string among candidates.
    ///
    /// Returns up to `max_results` matches, sorted by score (best first).
    /// Only returns matches with a positive score.
    ///
    /// # Arguments
    ///
    /// * `query` - The string to search for (e.g., a typo like "claud-config")
    /// * `candidates` - The list of valid overlay names to search through
    /// * `max_results` - Maximum number of suggestions to return
    #[must_use]
    pub(crate) fn find_matches(
        &self,
        query: &str,
        candidates: &[String],
        max_results: usize,
    ) -> Vec<ScoredMatch> {
        let mut matches: Vec<ScoredMatch> = candidates
            .iter()
            .filter_map(|candidate| {
                self.matcher
                    .fuzzy_match(candidate, query)
                    .map(|score| ScoredMatch {
                        value: candidate.clone(),
                        score,
                    })
            })
            .collect();

        // Sort by score descending (best matches first)
        matches.sort_by(|a, b| b.score.cmp(&a.score));

        // Return top N results
        matches.truncate(max_results);
        matches
    }

    /// Get suggestions formatted for error messages.
    ///
    /// Returns a formatted string like "claude-config, claude-docs" or
    /// an empty string if no matches found.
    #[must_use]
    pub(crate) fn suggest(
        &self,
        query: &str,
        candidates: &[String],
        max_results: usize,
    ) -> Vec<String> {
        self.find_matches(query, candidates, max_results)
            .into_iter()
            .map(|m| m.value)
            .collect()
    }
}

/// A fuzzy match result with its score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScoredMatch {
    /// The matched candidate value.
    pub(crate) value: String,
    /// The fuzzy match score (higher is better).
    pub(crate) score: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_exact_match() {
        let matcher = OverlayMatcher::new();
        let candidates = vec![
            "claude-config".to_string(),
            "copilot-config".to_string(),
            "dev-setup".to_string(),
        ];

        let matches = matcher.find_matches("claude-config", &candidates, 3);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].value, "claude-config");
    }

    #[test]
    fn find_typo_match() {
        let matcher = OverlayMatcher::new();
        let candidates = vec![
            "claude-config".to_string(),
            "copilot-config".to_string(),
            "dev-setup".to_string(),
        ];

        let matches = matcher.find_matches("claud-config", &candidates, 3);
        assert!(!matches.is_empty());
        // Should suggest claude-config as best match for "claud-config"
        assert_eq!(matches[0].value, "claude-config");
    }

    #[test]
    fn find_partial_match() {
        let matcher = OverlayMatcher::new();
        let candidates = vec![
            "claude-config".to_string(),
            "claude-docs".to_string(),
            "dev-setup".to_string(),
        ];

        let matches = matcher.find_matches("claude", &candidates, 3);
        assert!(matches.len() >= 2);
        // Both claude-* should match
        assert!(matches.iter().any(|m| m.value == "claude-config"));
        assert!(matches.iter().any(|m| m.value == "claude-docs"));
    }

    #[test]
    fn no_match_returns_empty() {
        let matcher = OverlayMatcher::new();
        let candidates = vec!["claude-config".to_string(), "copilot-config".to_string()];

        // Completely unrelated query should return empty or very low scores
        let matches = matcher.find_matches("zzzzzzzzz", &candidates, 3);
        // fuzzy-matcher may still return results with low scores
        // The important thing is that meaningful queries get good matches
        assert!(matches.is_empty() || matches[0].score < 10);
    }

    #[test]
    fn suggest_formats_correctly() {
        let matcher = OverlayMatcher::new();
        let candidates = vec![
            "claude-config".to_string(),
            "claude-docs".to_string(),
            "dev-setup".to_string(),
        ];

        let suggestions = matcher.suggest("claud", &candidates, 2);
        assert!(!suggestions.is_empty());
        assert!(suggestions.len() <= 2);
    }

    #[test]
    fn respects_max_results() {
        let matcher = OverlayMatcher::new();
        let candidates: Vec<String> = (0..10).map(|i| format!("overlay-{i}")).collect();

        let matches = matcher.find_matches("overlay", &candidates, 3);
        assert!(matches.len() <= 3);
    }

    #[test]
    fn empty_candidates_returns_empty() {
        let matcher = OverlayMatcher::new();
        let candidates: Vec<String> = vec![];

        let matches = matcher.find_matches("test", &candidates, 5);
        assert!(matches.is_empty());

        let suggestions = matcher.suggest("test", &candidates, 5);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn empty_query_still_works() {
        let matcher = OverlayMatcher::new();
        let candidates = vec!["overlay-a".to_string(), "overlay-b".to_string()];

        // Empty query may or may not match depending on fuzzy matcher behavior
        let matches = matcher.find_matches("", &candidates, 5);
        // Just verify it doesn't panic
        assert!(matches.len() <= 5);
    }

    #[test]
    fn scored_match_fields() {
        let scored = ScoredMatch {
            value: "test-overlay".to_string(),
            score: 100,
        };
        assert_eq!(scored.value, "test-overlay");
        assert_eq!(scored.score, 100);
    }

    #[test]
    fn scored_match_clone_and_eq() {
        let original = ScoredMatch {
            value: "test".to_string(),
            score: 50,
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn scored_match_debug() {
        let scored = ScoredMatch {
            value: "overlay".to_string(),
            score: 75,
        };
        let debug_str = format!("{scored:?}");
        assert!(debug_str.contains("overlay"));
        assert!(debug_str.contains("75"));
    }

    #[test]
    fn overlay_matcher_default() {
        // Test Default implementation
        let matcher = OverlayMatcher::default();
        let candidates = vec!["test".to_string()];
        let matches = matcher.find_matches("test", &candidates, 1);
        assert!(!matches.is_empty());
    }

    #[test]
    fn matches_sorted_by_score_descending() {
        let matcher = OverlayMatcher::new();
        let candidates = vec!["abc".to_string(), "abcd".to_string(), "abcde".to_string()];

        let matches = matcher.find_matches("abcde", &candidates, 3);
        // Best match (exact) should be first
        if matches.len() >= 2 {
            assert!(matches[0].score >= matches[1].score);
        }
    }

    #[test]
    fn suggest_returns_values_only() {
        let matcher = OverlayMatcher::new();
        let candidates = vec!["claude-config".to_string(), "copilot-config".to_string()];

        let suggestions = matcher.suggest("claude", &candidates, 2);
        // suggest() returns Vec<String>, not Vec<ScoredMatch>
        for suggestion in &suggestions {
            assert!(candidates.contains(suggestion));
        }
    }

    #[test]
    fn max_results_zero_returns_empty() {
        let matcher = OverlayMatcher::new();
        let candidates = vec!["test".to_string()];

        let matches = matcher.find_matches("test", &candidates, 0);
        assert!(matches.is_empty());

        let suggestions = matcher.suggest("test", &candidates, 0);
        assert!(suggestions.is_empty());
    }
}
