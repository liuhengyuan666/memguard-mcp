use std::collections::HashSet;

pub const JACCARD_FUZZY_THRESHOLD: f64 = 0.25;
pub const FUZZY_WEIGHT_FACTOR: f64 = 0.25;

/// Truncate a string to `max_len` characters, appending "…" if cut.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s.chars().take(max_len).collect::<String>())
    }
}

/// Check if `word` appears as a whole token in `text`, case-insensitive.
/// Splits on non-alphanumeric boundaries, handling snake_case, kebab-case,
/// and camelCase sub-words (e.g. "LoginScreen" splits into ["Login", "Screen"]).
#[inline]
pub fn contains_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|w| {
            let wl = w.to_lowercase();
            wl == word || wl.starts_with(word)
        })
}

/// Compute Jaccard similarity between two strings using character n-grams.
/// Returns [0.0, 1.0]. n=3 (trigram) is the default.
pub fn ngram_jaccard(a: &str, b: &str, n: usize) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a.len() < n || b.len() < n {
        let al = a.to_lowercase();
        let bl = b.to_lowercase();
        return if al.contains(&bl) || bl.contains(&al) { 0.5 } else { 0.0 };
    }

    let ngrams_a: HashSet<&[u8]> = a.as_bytes().windows(n).collect();
    let ngrams_b: HashSet<&[u8]> = b.as_bytes().windows(n).collect();

    let intersection = ngrams_a.intersection(&ngrams_b).count();
    let union = ngrams_a.len() + ngrams_b.len() - intersection;

    if union == 0 {
        0.0
    } else {
        // Use Dice coefficient for stronger recall on short tokens.
        2.0 * intersection as f64 / (ngrams_a.len() + ngrams_b.len()) as f64
    }
}

/// Hybrid scoring: exact word-boundary (precision) + trigram Jaccard (recall).
/// Score = sum(weight * exact_hit) + sum(weight * 0.25 * jaccard if jaccard > threshold)
pub fn score_match_v3(query: &str, fields: &[(f64, &str)]) -> f64 {
    if query.is_empty() {
        return 0.0;
    }

    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();
    let mut total: f64 = 0.0;

    for &(weight, field) in fields {
        let field_lower = field.to_lowercase();

        for token in &tokens {
            // 1. Exact word-boundary match (high precision)
            if contains_word(&field_lower, token) {
                total += weight;
                continue; // do not double-count fuzzy for exact hits
            }

            // 2. Fuzzy n-gram match (high recall)
            let jaccard = ngram_jaccard(token, &field_lower, 3);
            if jaccard > JACCARD_FUZZY_THRESHOLD {
                total += weight * FUZZY_WEIGHT_FACTOR * jaccard;
            }
        }

        // 3. Exact phrase bonus (full query appears as substring)
        if tokens.len() >= 2 && field_lower.contains(&query_lower) {
            total += weight * 0.5;
        }
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Scoring tests (score_match_v3) ──────────────────────────────────────

    #[test]
    fn test_score_match_exact() {
        let s = score_match_v3("auth token", &[(1.0, "Authentication token validation")]);
        assert_eq!(s, 2.0);
    }

    #[test]
    fn test_score_match_no_match() {
        let s = score_match_v3("database", &[(1.0, "HTTP routing")]);
        assert_eq!(s, 0.0);
    }

    #[test]
    fn test_score_match_partial() {
        let s = score_match_v3(
            "login page",
            &[(1.0, "Implement user authentication and login flow")],
        );
        assert_eq!(s, 1.0); // "login" matches
    }

    // ── Word-boundary tests ─────────────────────────────────────────────────

    #[test]
    fn test_contains_word_exact() {
        assert!(contains_word("Authentication token validation", "auth"));
    }

    #[test]
    fn test_contains_word_no_false_positive() {
        assert!(!contains_word("partition", "art"));
    }

    // ── N-gram Jaccard tests ────────────────────────────────────────────────

    #[test]
    fn test_ngram_jaccard_fuzzy_match() {
        let j = ngram_jaccard("auth", "authentication", 3);
        assert!(j > 0.25, "expected fuzzy match > 0.25, got {}", j);
    }

    #[test]
    fn test_ngram_jaccard_no_false_match() {
        let j = ngram_jaccard("database", "authentication", 3);
        assert_eq!(j, 0.0);
    }

    // ── Scoring priority tests ──────────────────────────────────────────────

    #[test]
    fn test_score_v3_exact_ranks_above_fuzzy() {
        let exact = score_match_v3("auth", &[(10.0, "auth module")]);
        let fuzzy = score_match_v3("auth", &[(10.0, "unauthorized access")]);
        assert!(
            exact > fuzzy,
            "exact match ({}) should score higher than fuzzy match ({})",
            exact,
            fuzzy
        );
    }

    #[test]
    fn test_score_v3_phrase_bonus() {
        let with_phrase = score_match_v3("auth token", &[(10.0, "auth token validation")]);
        let without_phrase = score_match_v3("auth token", &[(10.0, "auth and token validation")]);
        assert!(
            with_phrase > without_phrase,
            "phrase bonus should make contiguous match score higher"
        );
    }

    // ── Truncate tests ──────────────────────────────────────────────────────

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let result = truncate("hello world this is long", 10);
        assert!(result.ends_with('…'));
        assert!(result.len() <= 13); // 10 chars + "…" (3 bytes)
    }
}
