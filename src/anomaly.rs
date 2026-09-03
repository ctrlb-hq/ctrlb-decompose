use serde::Serialize;

use crate::stats::{PatternStats, PatternStore};
use crate::types::PatternID;

#[derive(Debug, Clone, Serialize)]
pub struct AnomalyFlags {
    pub pattern_id: PatternID,
    pub anomalies: Vec<Anomaly>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum Anomaly {
    FrequencySpike {
        description: String,
        severity: f64,
    },
    LowCardinality {
        var_slot: usize,
        unique_count: u64,
        description: String,
    },
    ClusteredNumeric {
        var_slot: usize,
        description: String,
    },
    HighErrorRate {
        description: String,
    },
    BimodalDistribution {
        var_slot: usize,
        description: String,
    },
}

impl Anomaly {
    pub fn severity(&self) -> f64 {
        match self {
            Anomaly::FrequencySpike { severity, .. } => *severity,
            Anomaly::HighErrorRate { .. } => 0.9,
            Anomaly::LowCardinality { .. } => 0.6,
            Anomaly::ClusteredNumeric { .. } => 0.5,
            Anomaly::BimodalDistribution { .. } => 0.4,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Anomaly::FrequencySpike { description, .. }
            | Anomaly::LowCardinality { description, .. }
            | Anomaly::ClusteredNumeric { description, .. }
            | Anomaly::HighErrorRate { description, .. }
            | Anomaly::BimodalDistribution { description, .. } => description,
        }
    }
}

pub fn detect_anomalies(store: &PatternStore) -> Vec<AnomalyFlags> {
    let mut results = Vec::new();

    for pattern in store.patterns.values() {
        let mut anomalies = Vec::new();

        if let Some(spike) = detect_frequency_spike(store, pattern) {
            anomalies.push(spike);
        }

        if let Some(err) = detect_error_pattern(pattern) {
            anomalies.push(err);
        }

        for (i, var) in pattern.variables.iter().enumerate() {
            // Low cardinality
            if pattern.count > 100 && var.categorical.unique_count() < 5 {
                anomalies.push(Anomaly::LowCardinality {
                    var_slot: i,
                    unique_count: var.categorical.unique_count(),
                    description: format!(
                        "targets only {} unique values",
                        var.categorical.unique_count()
                    ),
                });
            }

            if let Some(ref numeric) = var.numeric {
                // Clustered numeric
                if numeric.count > 50 {
                    let mean = numeric.mean();
                    if mean > 0.0 && (numeric.max - numeric.min) / mean < 0.05 {
                        anomalies.push(Anomaly::ClusteredNumeric {
                            var_slot: i,
                            description: format!(
                                "all values near {:.0} (possible fixed value/timeout)",
                                mean
                            ),
                        });
                    }
                }

                // Bimodal distribution
                if numeric.count > 50 {
                    let p25 = numeric.quantile(0.25).unwrap_or(0.0);
                    let p75 = numeric.quantile(0.75).unwrap_or(0.0);
                    if p25 > 0.0 && p75 / p25 > 5.0 {
                        anomalies.push(Anomaly::BimodalDistribution {
                            var_slot: i,
                            description: format!("bimodal: p25={:.0}, p75={:.0}", p25, p75),
                        });
                    }
                }
            }
        }

        if !anomalies.is_empty() {
            results.push(AnomalyFlags {
                pattern_id: pattern.pattern_id,
                anomalies,
            });
        }
    }

    results.sort_by(|a, b| {
        let max_a = a
            .anomalies
            .iter()
            .map(|a| a.severity())
            .fold(0.0f64, f64::max);
        let max_b = b
            .anomalies
            .iter()
            .map(|a| a.severity())
            .fold(0.0f64, f64::max);
        max_b
            .partial_cmp(&max_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results
}

fn detect_frequency_spike(store: &PatternStore, pattern: &PatternStats) -> Option<Anomaly> {
    let buckets = store.time_bucket_vector(pattern);
    if buckets.len() < 5 {
        return None;
    }

    let total: u64 = buckets.iter().sum();
    if total == 0 {
        return None;
    }

    // Check last 10% vs first 90%
    let split = (buckets.len() as f64 * 0.9) as usize;
    let first_90: u64 = buckets[..split].iter().sum();
    let last_10: u64 = buckets[split..].iter().sum();

    let first_rate = first_90 as f64 / split as f64;
    let last_rate = last_10 as f64 / (buckets.len() - split) as f64;

    if first_rate > 0.0 {
        let ratio = last_rate / first_rate;
        if ratio > 3.0 {
            return Some(Anomaly::FrequencySpike {
                description: format!("{:.0}x increase in last 10% of input", ratio),
                severity: (ratio / 10.0).min(1.0),
            });
        }
    }

    // Check initial burst (first 10% vs rest)
    let first_split = (buckets.len() as f64 * 0.1).max(1.0) as usize;
    let first_10: u64 = buckets[..first_split].iter().sum();
    let rest: u64 = buckets[first_split..].iter().sum();
    let first_10_rate = first_10 as f64 / first_split as f64;
    let rest_rate = rest as f64 / (buckets.len() - first_split) as f64;

    if rest_rate > 0.0 {
        let ratio = first_10_rate / rest_rate;
        if ratio > 3.0 {
            return Some(Anomaly::FrequencySpike {
                description: format!("{:.0}x burst in first 10% of input", ratio),
                severity: (ratio / 10.0).min(1.0),
            });
        }
    }

    None
}

fn detect_error_pattern(pattern: &PatternStats) -> Option<Anomaly> {
    // Only scan the first 100 chars — log levels appear in the first ~50-80 chars.
    // Scanning further causes false positives when structured data (JSON, Ruby
    // hashes) contains field names like "error" or "error_type".
    let mut prefix_end = pattern.template.len().min(100);
    while !pattern.template.is_char_boundary(prefix_end) {
        prefix_end -= 1;
    }
    let prefix_upper = pattern.template[..prefix_end].to_uppercase();
    let error_keywords = ["ERROR", "FATAL", "CRIT", "PANIC"];

    // Use word-boundary-aware matching: tokenize on whitespace + delimiters
    // so that e.g. "[ERROR]" matches but "error_handler" does not.
    let delimiters = |c: char| -> bool {
        c.is_whitespace()
            || matches!(
                c,
                '[' | ']'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '='
                    | ':'
                    | ';'
                    | ','
                    | '.'
                    | '"'
                    | '\''
                    | '<'
                    | '>'
            )
    };

    let tokens: Vec<&str> = prefix_upper.split(delimiters).filter(|s| !s.is_empty()).collect();

    if error_keywords.iter().any(|kw| tokens.contains(kw)) {
        return Some(Anomaly::HighErrorRate {
            description: "error-level log pattern".to_string(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::drain3::TypedVariable;
    use crate::stats::PatternStore;
    use crate::types::VarType;

    /// Helper: create a PatternStore and accumulate `lines` under pattern_id=1
    /// with the given template, no timestamps, no variables.
    fn store_with_lines(template: &str, lines: &[&str]) -> PatternStore {
        let mut store = PatternStore::new(2);
        for (i, line) in lines.iter().enumerate() {
            store.accumulate(1, template, &[], None, line, (i + 1) as u64);
        }
        store
    }

    #[test]
    fn test_detect_error_pattern_matches() {
        let lines: Vec<&str> = (0..5)
            .map(|_| "ERROR: connection failed to 10.0.0.1")
            .collect();
        let store = store_with_lines("ERROR: connection failed to <*>", &lines);
        let anomalies = detect_anomalies(&store);
        assert!(
            anomalies.iter().any(|af| af
                .anomalies
                .iter()
                .any(|a| matches!(a, Anomaly::HighErrorRate { .. }))),
            "expected HighErrorRate for template containing ERROR"
        );
    }

    #[test]
    fn test_detect_error_pattern_with_brackets() {
        let lines: Vec<&str> = (0..5)
            .map(|_| "[FATAL] process crashed")
            .collect();
        let store = store_with_lines("[FATAL] process crashed", &lines);
        let anomalies = detect_anomalies(&store);
        assert!(
            anomalies.iter().any(|af| af
                .anomalies
                .iter()
                .any(|a| matches!(a, Anomaly::HighErrorRate { .. }))),
            "expected HighErrorRate for template containing [FATAL]"
        );
    }

    #[test]
    fn test_no_false_positive_error_in_payload() {
        let lines: Vec<&str> = (0..5)
            .map(|_| "INFO processing via error_handler module")
            .collect();
        let store =
            store_with_lines("INFO processing via error_handler module", &lines);
        let anomalies = detect_anomalies(&store);
        assert!(
            !anomalies.iter().any(|af| af
                .anomalies
                .iter()
                .any(|a| matches!(a, Anomaly::HighErrorRate { .. }))),
            "error_handler should NOT trigger HighErrorRate (compound word)"
        );
    }

    #[test]
    fn test_no_error_in_info_pattern() {
        let lines: Vec<&str> = (0..5)
            .map(|_| "INFO request completed successfully")
            .collect();
        let store =
            store_with_lines("INFO request completed successfully", &lines);
        let anomalies = detect_anomalies(&store);
        assert!(
            !anomalies.iter().any(|af| af
                .anomalies
                .iter()
                .any(|a| matches!(a, Anomaly::HighErrorRate { .. }))),
            "INFO-only template should not produce HighErrorRate"
        );
    }

    #[test]
    fn test_low_cardinality_detection() {
        let mut store = PatternStore::new(2);
        let values = ["alpha", "beta", "gamma"];
        for i in 0..200 {
            let val = values[i % 3];
            let var = TypedVariable {
                raw: val.to_string(),
                var_type: VarType::String,
            };
            store.accumulate(
                1,
                "GET /api/<*>",
                &[var],
                None,
                &format!("GET /api/{}", val),
                (i + 1) as u64,
            );
        }
        let anomalies = detect_anomalies(&store);
        assert!(
            anomalies.iter().any(|af| af
                .anomalies
                .iter()
                .any(|a| matches!(a, Anomaly::LowCardinality { .. }))),
            "200 lines with 3 unique values should trigger LowCardinality"
        );
    }

    #[test]
    fn test_anomaly_severity_ordering() {
        let high_error = Anomaly::HighErrorRate {
            description: "error-level log pattern".to_string(),
        };
        let low_card = Anomaly::LowCardinality {
            var_slot: 0,
            unique_count: 3,
            description: "targets only 3 unique values".to_string(),
        };
        assert!(
            high_error.severity() > low_card.severity(),
            "HighErrorRate severity ({}) should be greater than LowCardinality severity ({})",
            high_error.severity(),
            low_card.severity()
        );
    }

    #[test]
    fn test_empty_store_no_anomalies() {
        let store = PatternStore::new(2);
        let anomalies = detect_anomalies(&store);
        assert!(
            anomalies.is_empty(),
            "empty PatternStore should produce no anomalies"
        );
    }
}
