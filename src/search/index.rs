use crate::models::{ADR, AdrStatus, Trap};
use crate::search::scorer::score_match_v3;
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

// ── Types ───────────────────────────────────────────────────────────────────

/// Distinguishes between ADR and Trap entries in the inverted index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryType {
    Adr,
    Trap,
}

/// Search index with inverted-index pre-filtering for term-based lookup.
///
/// Holds owned copies of ADRs and Traps for scoring and result construction.
/// The `terms` map provides O(1) lookup of which entries contain a given token,
/// avoiding brute-force iteration over all items for queries with common terms.
pub struct SearchIndex {
    adrs: Vec<ADR>,
    traps: Vec<Trap>,
    /// Inverted index: lowercase token → list of (EntryType, array_index) refs.
    pub terms: HashMap<String, Vec<(EntryType, usize)>>,
}

pub struct SearchResult {
    pub score: f64,
    pub item: SearchItem,
}

pub enum SearchItem {
    Adr(ADR),
    Trap(Trap),
}

impl SearchIndex {
    /// Build the index from ADR and Trap slices. Clones all data and
    /// populates the inverted-index `terms` map.
    pub fn build(adrs: &[ADR], traps: &[Trap]) -> Self {
        let owned_adrs = adrs.to_vec();
        let owned_traps = traps.to_vec();
        let mut terms: HashMap<String, Vec<(EntryType, usize)>> = HashMap::new();

        // Index ADRs: title, context, decision, and individual tags.
        for (idx, adr) in adrs.iter().enumerate() {
            let fields = [
                adr.title.as_str(),
                adr.context.as_str(),
                adr.decision.as_str(),
            ];
            let mut seen: HashSet<String> = HashSet::new();
            for field in &fields {
                for token in tokenize_field(field) {
                    if seen.insert(token.clone()) {
                        terms
                            .entry(token)
                            .or_default()
                            .push((EntryType::Adr, idx));
                    }
                }
            }
            // Tags are indexed individually (already lowercase for lookup).
            for tag in &adr.tags {
                let t = tag.to_lowercase();
                if seen.insert(t.clone()) {
                    terms.entry(t).or_default().push((EntryType::Adr, idx));
                }
            }
        }

        // Index Traps: all five fields.
        for (idx, trap) in traps.iter().enumerate() {
            let fields = [
                trap.error_signature.as_str(),
                trap.context.as_str(),
                trap.solution.as_str(),
                trap.root_cause.as_str(),
                trap.prevention.as_str(),
            ];
            let mut seen: HashSet<String> = HashSet::new();
            for field in &fields {
                for token in tokenize_field(field) {
                    if seen.insert(token.clone()) {
                        terms
                            .entry(token)
                            .or_default()
                            .push((EntryType::Trap, idx));
                    }
                }
            }
        }

        SearchIndex {
            adrs: owned_adrs,
            traps: owned_traps,
            terms,
        }
    }

    /// Serialize the inverted-index terms to the `.memguard/search_index.json`
    /// format.  Maps term entries from `(EntryType, usize)` indices to
    /// human-readable IDs using the ADR/Trap arrays.
    pub fn to_index_json(&self, adrs: &[ADR], traps: &[Trap]) -> serde_json::Value {
        let mut terms_json: serde_json::Map<String, serde_json::Value> =
            serde_json::Map::new();

        for (term, entries) in &self.terms {
            let refs: Vec<serde_json::Value> = entries
                .iter()
                .map(|(entry_type, idx)| {
                    let (typ, id) = match entry_type {
                        EntryType::Adr => {
                            let id = adrs.get(*idx).map(|a| a.id.as_str()).unwrap_or("?");
                            ("adr", id)
                        }
                        EntryType::Trap => {
                            let id = traps
                                .get(*idx)
                                .map(|t| t.error_signature.as_str())
                                .unwrap_or("?");
                            ("trap", id)
                        }
                    };
                    serde_json::json!({"type": typ, "id": id})
                })
                .collect();
            terms_json.insert(term.clone(), serde_json::Value::Array(refs));
        }

        serde_json::json!({
            "terms": terms_json,
            "metadata": {
                "version": "v4",
                "rebuilt_at": iso8601_now(),
            },
        })
    }

    /// Search with inverted-index pre-filtering.
    ///
    /// 1. Tokenize query (split on whitespace, lowercase).
    /// 2. Look up each token in `terms` to get candidate (type, index) pairs.
    /// 3. Union candidates (OR logic).
    /// 4. Score only candidates via `score_match_v3`.
    /// 5. Fallback to brute-force if no token matches or index is empty.
    pub fn search(&self, query: &str, limit: usize, include_stale: bool) -> Vec<SearchResult> {
        let query_lower = query.to_lowercase();
        let tokens: Vec<&str> = query_lower.split_whitespace().collect();

        // ── Collect candidates from inverted index ──────────────────────
        let mut candidate_set: HashSet<(EntryType, usize)> = HashSet::new();
        let mut any_index_match = false;

        for token in &tokens {
            if let Some(entries) = self.terms.get(*token) {
                any_index_match = true;
                for &(entry_type, idx) in entries {
                    candidate_set.insert((entry_type, idx));
                }
            }
        }

        // Fallback: if no tokens matched the index, brute-force all items.
        if !any_index_match {
            for idx in 0..self.adrs.len() {
                candidate_set.insert((EntryType::Adr, idx));
            }
            for idx in 0..self.traps.len() {
                candidate_set.insert((EntryType::Trap, idx));
            }
        }

        // ── Score candidates ────────────────────────────────────────────
        let mut results: Vec<SearchResult> = Vec::new();

        for (entry_type, idx) in candidate_set {
            match entry_type {
                EntryType::Adr => {
                    if idx >= self.adrs.len() {
                        continue;
                    }
                    let adr = &self.adrs[idx];

                    // Stale filtering (search-time).
                    if !include_stale
                        && !matches!(adr.status, AdrStatus::Accepted | AdrStatus::Proposed)
                    {
                        continue;
                    }

                    let score = score_match_v3(
                        query,
                        &[
                            (10.0, &adr.title),
                            (2.0, &adr.context),
                            (4.0, &adr.decision),
                            (6.0, &adr.tags.join(" ")),
                        ],
                    );
                    if score > 0.0 {
                        let final_score =
                            if !matches!(adr.status, AdrStatus::Accepted | AdrStatus::Proposed) {
                                score * 0.3
                            } else {
                                score
                            };
                        results.push(SearchResult {
                            score: final_score,
                            item: SearchItem::Adr(adr.clone()),
                        });
                    }
                }
                EntryType::Trap => {
                    if idx >= self.traps.len() {
                        continue;
                    }
                    let trap = &self.traps[idx];

                    let score = score_match_v3(
                        query,
                        &[
                            (15.0, &trap.error_signature),
                            (3.0, &trap.context),
                            (6.0, &trap.solution),
                            (8.0, &trap.root_cause),
                            (5.0, &trap.prevention),
                        ],
                    );
                    if score > 0.0 {
                        results.push(SearchResult {
                            score,
                            item: SearchItem::Trap(trap.clone()),
                        });
                    }
                }
            }
        }

        // Sort by score descending, take top `limit`.
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        results
    }
}

// ── Serialization helper ────────────────────────────────────────────────────

/// Return an ISO 8601 UTC timestamp string for the current system time.
fn iso8601_now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (y, m, d) = days_to_ymd(days_since_epoch as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    days += 719468; // shift epoch to 0000-03-01
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ── Tokenization ───────────────────────────────────────────────────────────—

/// Tokenize a field string for inverted-index insertion.
///
/// Splits on whitespace, lowercases, and strips leading/trailing
/// non-alphanumeric characters (e.g. punctuation).  Matches the query
/// tokenization in `score_match_v3` (split_whitespace + to_lowercase).
fn tokenize_field(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|t| {
            t.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Build a test ADR with the given id/title/status and fixed fields.
    fn adr(id: &str, title: &str, status: AdrStatus) -> ADR {
        ADR {
            id: id.into(),
            title: title.into(),
            status,
            context: format!("Context for {}.", id),
            decision: format!("Decision for {}.", id),
            tags: vec![],
        }
    }

    /// Build a test Trap with the given signature.
    fn trap(sig: &str) -> Trap {
        Trap {
            error_signature: sig.into(),
            context: format!("Ctx {}.", sig),
            solution: format!("Sol {}.", sig),
            root_cause: format!("Rc {}.", sig),
            prevention: format!("Prev {}.", sig),
        }
    }

    /// Helper: brute-force search that iterates ALL items, same scoring as
    /// `SearchIndex::search()`.  Returns (score, item_id_string).
    fn brute_force(
        adrs: &[ADR],
        traps: &[Trap],
        query: &str,
        include_stale: bool,
    ) -> Vec<(f64, String)> {
        let mut results: Vec<(f64, String)> = Vec::new();

        for adr in adrs {
            if !include_stale
                && !matches!(adr.status, AdrStatus::Accepted | AdrStatus::Proposed)
            {
                continue;
            }
            let score = score_match_v3(
                query,
                &[
                    (10.0, &adr.title),
                    (2.0, &adr.context),
                    (4.0, &adr.decision),
                    (6.0, &adr.tags.join(" ")),
                ],
            );
            if score > 0.0 {
                let final_score =
                    if !matches!(adr.status, AdrStatus::Accepted | AdrStatus::Proposed) {
                        score * 0.3
                    } else {
                        score
                    };
                results.push((final_score, adr.id.clone()));
            }
        }

        for t in traps {
            let score = score_match_v3(
                query,
                &[
                    (15.0, &t.error_signature),
                    (3.0, &t.context),
                    (6.0, &t.solution),
                    (8.0, &t.root_cause),
                    (5.0, &t.prevention),
                ],
            );
            if score > 0.0 {
                results.push((score, t.error_signature.clone()));
            }
        }

        results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Run indexed search and return (score, item_id_string) tuples.
    fn indexed_search(
        index: &SearchIndex,
        query: &str,
        limit: usize,
        include_stale: bool,
    ) -> Vec<(f64, String)> {
        index
            .search(query, limit, include_stale)
            .into_iter()
            .map(|r| match r.item {
                SearchItem::Adr(a) => (r.score, a.id),
                SearchItem::Trap(t) => (r.score, t.error_signature),
            })
            .collect()
    }

    // ── Tests ───────────────────────────────────────────────────────────

    #[test]
    fn test_inverted_index_correctness_matches_brute_force() {
        let adrs = vec![
            adr("ADR-001", "Use Vue 3 for frontend framework", AdrStatus::Accepted),
            adr("ADR-002", "Use Serde for serialization layer", AdrStatus::Accepted),
            adr(
                "ADR-003",
                "Deprecated: old router approach",
                AdrStatus::Superseded,
            ),
            adr("ADR-004", "Adopt PostgreSQL as primary database", AdrStatus::Accepted),
        ];

        let traps = vec![
            trap("NPE in auth handler"),
            trap("Race condition in state cache"),
        ];

        let index = SearchIndex::build(&adrs, &traps);
        assert!(!index.terms.is_empty(), "index should have terms");

        // Test queries with common terms (should use inverted index).
        let queries = vec![
            ("vue3 frontend", false),
            ("serde", false),
            ("router", true),  // needs include_stale to find Superseded ADR-003
            ("database", false),
            ("auth handler", false),
            ("race condition", false),
        ];

        for (q, include_stale) in &queries {
            let bf = brute_force(&adrs, &traps, q, *include_stale);
            let ix = indexed_search(&index, q, 20, *include_stale);
            assert_eq!(
                bf.len(),
                ix.len(),
                "query='{}' include_stale={}: result count mismatch (bf={}, ix={})",
                q,
                include_stale,
                bf.len(),
                ix.len()
            );
            for (i, ((bf_score, bf_id), (ix_score, ix_id))) in
                bf.iter().zip(ix.iter()).enumerate()
            {
                assert_eq!(
                    bf_id, ix_id,
                    "query='{}' include_stale={}: result {} id mismatch (bf={}, ix={})",
                    q, include_stale, i, bf_id, ix_id
                );
                assert!(
                    (bf_score - ix_score).abs() < 1e-9,
                    "query='{}' include_stale={}: result {} score mismatch (bf={}, ix={})",
                    q,
                    include_stale,
                    i,
                    bf_score,
                    ix_score
                );
            }
        }
    }

    #[test]
    fn test_inverted_index_no_match_returns_empty() {
        let adrs = vec![adr("ADR-001", "Vue frontend", AdrStatus::Accepted)];
        let traps = vec![trap("Timeout error")];
        let index = SearchIndex::build(&adrs, &traps);

        let results = index.search("zzzz_nonexistent_xyz", 10, false);
        assert!(results.is_empty(), "non-matching query should return empty");
    }

    #[test]
    fn test_inverted_index_fallback_when_index_empty() {
        // Index with no terms (empty data).
        let index = SearchIndex::build(&[], &[]);
        assert!(index.terms.is_empty());

        let results = index.search("anything", 10, false);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rebuild_performance_500_items() {
        use std::time::Instant;

        let mut adrs = Vec::with_capacity(300);
        for i in 0..300 {
            let words = [
                "auth", "token", "database", "cache", "router", "api", "graphql",
                "rest", "websocket", "serialization", "validation", "logging",
                "metrics", "tracing", "security", "encryption", "compression",
                "streaming", "batching", "queueing", "scheduling", "monitoring",
                "vue3", "react", "angular", "svelte", "solid", "preact",
                "rust", "typescript", "golang", "python", "zig", "elixir",
                "postgresql", "sqlite", "redis", "mongodb", "elasticsearch",
                "kafka", "rabbitmq", "nats", "grpc", "protobuf", "avro",
            ];
            let t1 = words[i % words.len()];
            let t2 = words[(i * 7 + 3) % words.len()];
            let t3 = words[(i * 13 + 5) % words.len()];
            adrs.push(ADR {
                id: format!("ADR-{:03}", i),
                title: format!("Use {} with {} for module {}", t1, t2, i),
                status: if i % 5 == 0 {
                    AdrStatus::Superseded
                } else {
                    AdrStatus::Accepted
                },
                context: format!(
                    "Context for {}: need {}, {}, and {} support.",
                    t1, t2, t3, words[(i * 3) % words.len()]
                ),
                decision: format!(
                    "Decision: adopt {} with {} integration and {} backend.",
                    t1, t2, t3
                ),
                tags: vec![t1.to_string(), t2.to_string()],
            });
        }

        let mut traps = Vec::with_capacity(200);
        for i in 0..200 {
            let w = [
                "null", "pointer", "timeout", "deadlock", "race", "leak",
                "overflow", "corruption", "panic", "unwrap", "unwrap_err",
                "index", "bounds", "borrow", "lifetime",
            ];
            let w1 = w[i % w.len()];
            let w2 = w[(i * 5 + 2) % w.len()];
            traps.push(Trap {
                error_signature: format!("{} {} error in module {}", w1, w2, i),
                context: format!("Occurred when processing request {}.", i),
                solution: format!("Fix by adding proper {} guard.", w1),
                root_cause: format!("Missing {} check in {} handler.", w1, w2),
                prevention: format!("Add lint rule for {} patterns.", w1),
            });
        }

        let start = Instant::now();
        let index = SearchIndex::build(&adrs, &traps);
        let build_elapsed = start.elapsed();

        assert!(
            build_elapsed.as_millis() < 200,
            "build took {}ms, expected < 200ms for 500 items",
            build_elapsed.as_millis()
        );
        assert!(!index.terms.is_empty(), "index should have terms");

        // Verify a search also completes quickly.
        let start2 = Instant::now();
        let results = index.search("auth token database", 10, false);
        let search_elapsed = start2.elapsed();

        assert!(
            search_elapsed.as_millis() < 50,
            "search took {}ms, expected < 50ms with inverted index",
            search_elapsed.as_millis()
        );
        assert!(
            !results.is_empty(),
            "should find results for common terms in 500 items"
        );
    }
}
