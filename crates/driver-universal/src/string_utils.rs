//! String similarity utilities for CLI error suggestions.
//!
//! Extracted from `manifest-check` so future tooling (the upcoming
//! `manifest-wizard`, schema validators, or any other CLI that surfaces
//! a typo'd name from a manifest) can produce "did you mean?" hints
//! without copy-pasting a Levenshtein implementation.
//!
//! Kept intentionally small: if a second caller needs fancier matching
//! (Jaro-Winkler, token-aware), pull in a crate rather than growing
//! this module.

/// Return the candidate with the smallest Levenshtein distance from
/// `target`, but only if that distance is within
/// `max(3, target_len / 3)` — otherwise `None`.
///
/// Ties are broken by iteration order of `candidates`.
pub fn closest_within_threshold<'a, I>(target: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let threshold = 3usize.max(target.chars().count() / 3);
    candidates
        .into_iter()
        .map(|c| (c, edit_distance(target, c)))
        .filter(|(_, d)| *d <= threshold)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c)
}

/// Classic Levenshtein edit distance (insertion, deletion, and
/// substitution each cost 1). Works over Unicode scalar values, not
/// bytes, so non-ASCII input is handled correctly.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_identical() {
        assert_eq!(edit_distance("position", "position"), 0);
    }

    #[test]
    fn edit_distance_single_substitution() {
        assert_eq!(edit_distance("positon", "position"), 1);
    }

    #[test]
    fn edit_distance_empty() {
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("", ""), 0);
    }

    #[test]
    fn edit_distance_unicode_scalars() {
        // 'é' is one scalar value — one substitution from 'e'.
        assert_eq!(edit_distance("café", "cafe"), 1);
    }

    #[test]
    fn closest_picks_nearest_within_threshold() {
        let cands = ["position", "status", "velocity"];
        assert_eq!(
            closest_within_threshold("positon", cands.iter().copied()),
            Some("position")
        );
        assert_eq!(
            closest_within_threshold("stauts", cands.iter().copied()),
            Some("status")
        );
    }

    #[test]
    fn closest_returns_none_when_far() {
        let cands = ["position", "status"];
        assert_eq!(
            closest_within_threshold("xyzzy", cands.iter().copied()),
            None
        );
    }

    #[test]
    fn closest_handles_empty_candidate_list() {
        let cands: [&str; 0] = [];
        assert_eq!(closest_within_threshold("anything", cands), None);
    }
}
