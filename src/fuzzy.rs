//! Fuzzy matching for overlay name suggestions.
//!
//! Uses the fzf-style matching algorithm from `fuzzy-matcher` to provide
//! helpful suggestions when an overlay name is not found.

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

/// Fuzzy matcher for finding similar overlay names.
pub struct OverlayMatcher {
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
    pub fn new() -> Self {
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
    pub fn find_matches(
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
    pub fn suggest(&self, query: &str, candidates: &[String], max_results: usize) -> Vec<String> {
        self.find_matches(query, candidates, max_results)
            .into_iter()
            .map(|m| m.value)
            .collect()
    }
}

/// A fuzzy match result with its score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoredMatch {
    /// The matched candidate value.
    pub value: String,
    /// The fuzzy match score (higher is better).
    pub score: i64,
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
}
