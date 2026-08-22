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
    if denom < 1e-10 { 0.0 } else { cov / denom }
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
        let r = pearson_correlation(trimmed_a, shifted_b);
        if r > 0.7 {
            return Some(lag);
        }
    }
    None
}
