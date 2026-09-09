//! Fuzzy scoring + did-you-mean.

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use strsim::jaro_winkler;

/// Score below which we consider a spec "not found" even if some fuzzy
/// matches came back. Calibrated so `firefxo` → `firefox` passes but
/// `asdfghjkl` against random packages doesn't.
pub const EXACTISH: i64 = 60;

/// Rank a candidate name against the user's spec.
pub fn score(spec: &str, candidate: &str) -> i64 {
    // Exact substring gets a strong boost (what pacman -Ssq already did),
    // then skim's fuzzy score for ordering.
    let matcher = SkimMatcherV2::default();
    let mut score = matcher.fuzzy_match(candidate, spec).unwrap_or(0);
    let spec_lc = spec.to_lowercase();
    let cand_lc = candidate.to_lowercase();
    if cand_lc.contains(&spec_lc) {
        score += 100;
    }
    if cand_lc == spec_lc {
        score += 1000;
    }
    score
}

/// did-you-mean: jaro-winkler similarity over the pool of known names.
pub fn near_misses(spec: &str, pool: &[String], limit: usize) -> Vec<String> {
    let mut scored: Vec<(f64, &String)> = pool
        .iter()
        .map(|name| {
            (
                jaro_winkler(&spec.to_lowercase(), &name.to_lowercase()),
                name,
            )
        })
        .filter(|(sim, _)| *sim >= 0.75)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, name)| name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typo_corrects_to_firefox() {
        let pool = vec![
            "firefox".to_string(),
            "filelight".to_string(),
            "firejail".to_string(),
            "zsh".to_string(),
        ];
        let misses = near_misses("firefxo", &pool, 5);
        assert_eq!(misses.first().map(|s| s.as_str()), Some("firefox"));
    }

    #[test]
    fn garbage_matches_nothing() {
        let pool = vec!["firefox".to_string(), "ripgrep".to_string()];
        assert!(near_misses("qqqzzz", &pool, 5).is_empty());
    }

    #[test]
    fn exact_beats_fuzzy() {
        assert!(score("firefox", "firefox") > score("firefox", "firefox-developer-edition"));
    }
}
