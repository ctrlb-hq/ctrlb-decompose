use std::collections::HashSet;

use serde::Serialize;

use crate::stats::{PatternStats, PatternStore};
use crate::types::PatternID;

#[derive(Debug, Clone, Serialize)]
pub struct Correlation {
    pub pattern_a: PatternID,
    pub pattern_b: PatternID,
    pub correlation_type: CorrelationType,
    pub description: String,
    pub strength: f64,
}

#[derive(Debug, Clone, Serialize)]
pub enum CorrelationType {
    TemporalCooccurrence,
    SharedVariable,
    ErrorCascade,
}

pub fn find_correlations(store: &PatternStore) -> Vec<Correlation> {
    let mut results = Vec::new();
    let top_patterns = store.sorted_patterns();
    let top_patterns: Vec<_> = top_patterns.into_iter().take(20).collect();

    // Temporal co-occurrence
    for i in 0..top_patterns.len() {
        let vec_a = store.time_bucket_vector(top_patterns[i]);
        if vec_a.len() < 3 {
            continue;
        }
        for j in (i + 1)..top_patterns.len() {
            let vec_b = store.time_bucket_vector(top_patterns[j]);
            if vec_b.len() < 3 {
                continue;
            }

            let r = pearson_correlation(&vec_a, &vec_b);
            if r.abs() > 0.7 {
                results.push(Correlation {
                    pattern_a: top_patterns[i].pattern_id,
                    pattern_b: top_patterns[j].pattern_id,
                    correlation_type: CorrelationType::TemporalCooccurrence,
                    description: format!("spike together (r={:.2})", r),
                    strength: r.abs(),
                });
            }
        }
    }

    // Shared variable detection
    for i in 0..top_patterns.len() {
        for j in (i + 1)..top_patterns.len() {
            if let Some(desc) = detect_shared_variables(top_patterns[i], top_patterns[j]) {
                results.push(Correlation {
                    pattern_a: top_patterns[i].pattern_id,
                    pattern_b: top_patterns[j].pattern_id,
                    correlation_type: CorrelationType::SharedVariable,
                    description: desc,
                    strength: 0.8,
                });
            }
        }
    }

    // Error cascade detection
    for i in 0..top_patterns.len() {
        let is_error_i = is_error_pattern(top_patterns[i]);
        for j in 0..top_patterns.len() {
            if i == j {
                continue;
            }
            let is_error_j = is_error_pattern(top_patterns[j]);

            // Non-error A's spike precedes error B's spike
            if !is_error_i && is_error_j {
                let vec_a = store.time_bucket_vector(top_patterns[i]);
                let vec_b = store.time_bucket_vector(top_patterns[j]);

                if let Some(lag) = detect_lag_correlation(&vec_a, &vec_b, 1, 3) {
                    results.push(Correlation {
                        pattern_a: top_patterns[i].pattern_id,
                        pattern_b: top_patterns[j].pattern_id,
                        correlation_type: CorrelationType::ErrorCascade,
                        description: format!("precedes error by ~{}min", lag),
                        strength: 0.9,
                    });
                }
            }
        }
    }

    results.sort_by(|a, b| {
        b.strength
            .partial_cmp(&a.strength)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

fn is_error_pattern(p: &PatternStats) -> bool {
    let upper = p.template.to_uppercase();
    upper.contains("ERROR") || upper.contains("FATAL") || upper.contains("PANIC")
}

fn pearson_correlation(a: &[u64], b: &[u64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 2 {
        return 0.0;
    }

    let mean_a: f64 = a[..n].iter().map(|&x| x as f64).sum::<f64>() / n as f64;
    let mean_b: f64 = b[..n].iter().map(|&x| x as f64).sum::<f64>() / n as f64;

    let mut cov = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;

    for i in 0..n {
        let da = a[i] as f64 - mean_a;
        let db = b[i] as f64 - mean_b;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }

    let denom = (var_a * var_b).sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        cov / denom
    }
}

fn detect_shared_variables(a: &PatternStats, b: &PatternStats) -> Option<String> {
    for var_a in &a.variables {
        for var_b in &b.variables {
            if var_a.var_type != var_b.var_type {
                continue;
            }

            let top_a: HashSet<String> = var_a
                .categorical
                .top_k(10)
                .into_iter()
                .map(|(v, _, _)| v)
                .collect();
            let top_b: HashSet<String> = var_b
                .categorical
                .top_k(10)
                .into_iter()
                .map(|(v, _, _)| v)
                .collect();

            if top_a.is_empty() || top_b.is_empty() {
                continue;
            }

            let overlap = top_a.intersection(&top_b).count();
            let min_size = top_a.len().min(top_b.len());

            if min_size > 0 && overlap as f64 / min_size as f64 > 0.5 {
                return Some(format!(
                    "shared {} values ({} overlap)",
                    var_a.var_type, overlap
                ));
            }
        }
    }
    None
}

fn detect_lag_correlation(a: &[u64], b: &[u64], min_lag: usize, max_lag: usize) -> Option<usize> {
    let n = a.len().min(b.len());
    if n < max_lag + 3 {
        return None;
    }

    for lag in min_lag..=max_lag {
        let shifted_b = &b[lag..n];
        let trimmed_a = &a[..n - lag];
        let r = pearson_correlation(
            &trimmed_a.iter().copied().collect::<Vec<_>>(),
            &shifted_b.iter().copied().collect::<Vec<_>>(),
        );
        if r > 0.7 {
            return Some(lag);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{BoundedVec, PatternStats, PatternStore};
    use std::collections::HashMap;

    // --- Pearson correlation tests ---

    #[test]
    fn test_pearson_correlation_perfect_positive() {
        let a = vec![1, 2, 3, 4, 5];
        let b = vec![2, 4, 6, 8, 10];
        let r = pearson_correlation(&a, &b);
        assert!((r - 1.0).abs() < 1e-9, "expected r ≈ 1.0, got {}", r);
    }

    #[test]
    fn test_pearson_correlation_perfect_negative() {
        let a = vec![1, 2, 3, 4, 5];
        let b = vec![10, 8, 6, 4, 2];
        let r = pearson_correlation(&a, &b);
        assert!((r - (-1.0)).abs() < 1e-9, "expected r ≈ -1.0, got {}", r);
    }

    #[test]
    fn test_pearson_correlation_zero() {
        // Constant series has zero variance, should return 0.0
        let a = vec![5, 5, 5, 5, 5];
        let b = vec![1, 2, 3, 4, 5];
        let r = pearson_correlation(&a, &b);
        assert!(r.abs() < 1e-9, "expected r ≈ 0.0, got {}", r);
    }

    #[test]
    fn test_pearson_correlation_too_short() {
        let a = vec![1];
        let b = vec![2];
        let r = pearson_correlation(&a, &b);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn test_pearson_correlation_empty() {
        let a: Vec<u64> = vec![];
        let b: Vec<u64> = vec![];
        let r = pearson_correlation(&a, &b);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn test_pearson_correlation_different_lengths() {
        // Uses min(len) = 3; first 3 elements are perfectly positively correlated
        let a = vec![1, 2, 3, 4, 5, 6, 7];
        let b = vec![2, 4, 6];
        let r = pearson_correlation(&a, &b);
        assert!((r - 1.0).abs() < 1e-9, "expected r ≈ 1.0, got {}", r);
    }

    // --- Lag correlation tests ---

    #[test]
    fn test_detect_lag_correlation_basic() {
        // Pattern a spikes, pattern b spikes one step later
        let a = vec![0, 10, 0, 0, 10, 0, 0, 10, 0, 0];
        let b = vec![0, 0, 10, 0, 0, 10, 0, 0, 10, 0];
        let lag = detect_lag_correlation(&a, &b, 1, 3);
        assert_eq!(lag, Some(1), "expected lag of 1");
    }

    #[test]
    fn test_detect_lag_correlation_too_short() {
        let a = vec![1, 2, 3];
        let b = vec![1, 2, 3];
        // max_lag=3, need n >= max_lag + 3 = 6, but n=3 so returns None
        let lag = detect_lag_correlation(&a, &b, 1, 3);
        assert_eq!(lag, None);
    }

    // --- is_error_pattern tests ---

    fn make_pattern_stats(template: &str) -> PatternStats {
        PatternStats {
            pattern_id: 0,
            template: template.to_string(),
            count: 1,
            first_seen_line: 1,
            last_seen_line: 1,
            first_ts: None,
            last_ts: None,
            variables: Vec::new(),
            time_buckets: HashMap::new(),
            example_lines: BoundedVec::new(0),
        }
    }

    #[test]
    fn test_is_error_pattern() {
        assert!(is_error_pattern(&make_pattern_stats("ERROR: something failed")));
        assert!(is_error_pattern(&make_pattern_stats("FATAL crash")));
        assert!(is_error_pattern(&make_pattern_stats("PANIC at the disco")));
        assert!(!is_error_pattern(&make_pattern_stats("INFO request completed")));
    }

    // --- find_correlations tests ---

    #[test]
    fn test_find_correlations_empty_store() {
        let store = PatternStore::new(0);
        let results = find_correlations(&store);
        assert!(results.is_empty(), "expected no correlations from empty store");
    }
}
