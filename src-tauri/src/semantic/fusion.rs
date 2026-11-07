//! Hybrid ranking: Reciprocal Rank Fusion (Cormack et al. 2009) of the keyword and
//! semantic result lists, plus the rules for when the semantic leg runs.

use std::collections::HashMap;

/// RRF constant: a document at rank r (1-based) in a list scores 1 / (K + r).
pub const RRF_K: f64 = 60.0;
/// Results taken from each leg before fusing.
pub const DEPTH: usize = 50;
/// The semantic leg needs a query (tags removed) at least this long.
pub const MIN_SEMANTIC_CHARS: usize = 4;
/// Per-query similarity cut for semantic hits, applied to each note's best chunk. e5 scores
/// sit in a narrow band (about 0.74–0.85) that shifts with the query, so no single cosine
/// value separates relevant notes from noise. Instead a note must beat the typical note
/// for this query: score ≥ max(`ABSOLUTE_FLOOR`, median best-chunk score over all indexed
/// notes + `MARGIN`). The median is taken before tag filtering and the depth cut. Calibrated
/// with the real model in `real_model_tests` (synthetic Swedish/English notes chunked by the
/// app's chunker plus off-topic queries; see the brief's Spike line).
///
/// On 400 synthetic notes, 0.045 keeps a relevant note for 82.5% of topic queries (as many as
/// the fixed 0.81) with precision 0.84 (0.68 fixed) and 0.49% of unrelated pairs (1.45%
/// fixed), but 60% of off-topic queries still return a note (3.35 on average; 30% / 0.65
/// fixed). e5 scores are compressed and no rule tried (fixed, median + margin, MAD, sd,
/// percentile, top-N) separates cleanly; this one favours not missing real matches, which
/// is the cheaper error here: semantic-only hits are labelled Related and fused by RRF with
/// ties going to keyword hits.
pub const MARGIN: f32 = 0.045;
/// Safety floor under the relative rule: below this a note never counts as related, however
/// low the median is for the query.
pub const ABSOLUTE_FLOOR: f32 = 0.76;
/// With fewer indexed notes than this the median says little about the query, so
/// `FIXED_FLOOR` applies instead of the relative rule.
pub const MIN_NOTES_FOR_RELATIVE: usize = 8;
/// Fixed cosine floor for small indexes (the earlier global floor: ~1.5% of unrelated notes
/// passed on the synthetic corpus).
pub const FIXED_FLOOR: f32 = 0.81;

/// The minimum best-chunk score for a semantic hit, given every indexed note's best-chunk
/// score for the query (any order).
pub fn similarity_floor(scores: &[f32]) -> f32 {
    if scores.len() < MIN_NOTES_FOR_RELATIVE {
        return FIXED_FLOOR;
    }
    let mut sorted = scores.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mid = sorted.len() / 2;
    let median = if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    };
    ABSOLUTE_FLOOR.max(median + MARGIN)
}

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
    fn similarity_floor_is_median_plus_margin() {
        // Odd count: the middle score.
        let scores = [0.70, 0.74, 0.78, 0.80, 0.82, 0.84, 0.86, 0.79, 0.81];
        assert!((similarity_floor(&scores) - (0.80 + MARGIN)).abs() < 1e-6);
        // Even count: mean of the two middle scores; order doesn't matter.
        let scores = [0.86, 0.80, 0.84, 0.82, 0.83, 0.85, 0.87, 0.81];
        assert!((similarity_floor(&scores) - (0.835 + MARGIN)).abs() < 1e-6);
    }

    #[test]
    fn similarity_floor_never_drops_below_the_absolute_floor() {
        let scores = [0.0; 12];
        assert_eq!(similarity_floor(&scores), ABSOLUTE_FLOOR);
        let low = [0.60, 0.61, 0.62, 0.63, 0.64, 0.65, 0.66, 0.67];
        assert!(0.635 + MARGIN < ABSOLUTE_FLOOR);
        assert_eq!(similarity_floor(&low), ABSOLUTE_FLOOR);
    }

    #[test]
    fn similarity_floor_is_fixed_for_small_indexes() {
        assert_eq!(similarity_floor(&[]), FIXED_FLOOR);
        let seven = [0.0; MIN_NOTES_FOR_RELATIVE - 1];
        assert_eq!(similarity_floor(&seven), FIXED_FLOOR);
        let eight = [0.0; MIN_NOTES_FOR_RELATIVE];
        assert_eq!(similarity_floor(&eight), ABSOLUTE_FLOOR);
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
