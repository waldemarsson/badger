//! Hybrid ranking: Reciprocal Rank Fusion (Cormack et al. 2009) of the keyword and
//! semantic result lists, plus the rules for when the semantic leg runs.

use std::collections::HashMap;

/// RRF constant: a document at rank r (1-based) in a list scores 1 / (K + r).
pub const RRF_K: f64 = 60.0;
/// Results taken from each leg before fusing.
pub const DEPTH: usize = 50;
/// The semantic leg needs a query (tags removed) at least this long.
pub const MIN_SEMANTIC_CHARS: usize = 4;
/// Minimum cosine similarity for a semantic hit, applied to a note's best chunk.
/// Calibrated at note level with the real model on the synthetic Swedish/English corpus in
/// `real_model_tests` (markdown notes chunked by the app's chunker, title + heading prefix
/// included; 0.81 lets ~1.5% of unrelated notes pass with the same recall as 0.8075's 2%; see the
/// brief's Spike line). Notes with many chunks get more chances to pass by noise, so a floor
/// calibrated per chunk pair would be too low.
pub const SIMILARITY_FLOOR: f32 = 0.81;

/// The text the semantic leg embeds: the query without `#tag` terms, when it is long
/// enough; `None` skips the semantic leg (short or tag-only queries).
pub fn semantic_query(query: &str) -> Option<String> {
    let text: Vec<&str> = query
        .split_whitespace()
        .filter(|t| crate::note_links::tag_term(t).is_none())
        .collect();
    let text = text.join(" ");
    (text.chars().count() >= MIN_SEMANTIC_CHARS).then_some(text)
}

/// Fuses two ranked id lists (best first) with RRF over their first `DEPTH` entries.
/// Ties go to the better single rank, then to keyword hits, then to the lower id.
pub fn rrf(keyword: &[i64], semantic: &[i64]) -> Vec<i64> {
    // id → (score, best rank, in keyword list)
    let mut scores: HashMap<i64, (f64, usize, bool)> = HashMap::new();
    for (list, is_keyword) in [(keyword, true), (semantic, false)] {
        for (rank, id) in list.iter().take(DEPTH).enumerate() {
            let e = scores.entry(*id).or_insert((0.0, usize::MAX, false));
            e.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
            e.1 = e.1.min(rank);
            e.2 |= is_keyword;
        }
    }
    let mut ids: Vec<(i64, (f64, usize, bool))> = scores.into_iter().collect();
    ids.sort_by(|(a_id, a), (b_id, b)| {
        b.0.total_cmp(&a.0)
            .then(a.1.cmp(&b.1))
            .then(b.2.cmp(&a.2))
            .then(a_id.cmp(b_id))
    });
    ids.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_rewards_agreement_between_legs() {
        // 3 is second in both lists: 2/(62) beats 1/61 for a single first place.
        assert_eq!(rrf(&[1, 3, 4], &[2, 3, 5]), vec![3, 1, 2, 4, 5]);
    }

    #[test]
    fn rrf_ties_prefer_keyword_hits_then_lower_ids() {
        assert_eq!(rrf(&[7], &[3]), vec![7, 3]);
        assert_eq!(rrf(&[], &[9, 4]), vec![9, 4]);
    }

    #[test]
    fn rrf_only_fuses_to_depth() {
        let keyword: Vec<i64> = (0..60).collect();
        let fused = rrf(&keyword, &[]);
        assert_eq!(fused.len(), DEPTH);
        assert_eq!(fused[0], 0);
    }

    #[test]
    fn rrf_scores_match_the_formula() {
        // id 1: 1/61 + 1/63 ≈ 0.03227; id 2: 1/62 + 1/61 ≈ 0.03252 → 2 first.
        assert_eq!(rrf(&[1, 2], &[2, 9, 1]), vec![2, 1, 9]);
    }

    #[test]
    fn semantic_query_needs_four_chars_without_tags() {
        assert_eq!(semantic_query("abc"), None);
        assert_eq!(semantic_query("#budget"), None);
        assert_eq!(semantic_query("#budget #plan abc"), None);
        assert_eq!(semantic_query("  plan  "), Some("plan".into()));
        assert_eq!(
            semantic_query("#acme money plan"),
            Some("money plan".into())
        );
        assert_eq!(semantic_query("åäö!"), Some("åäö!".into()));
    }
}
