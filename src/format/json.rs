use std::collections::HashMap;

use serde::Serialize;

use crate::label::infer_label;
use crate::scoring::{PatternScore, Severity};
use crate::stats::PatternStore;
use crate::types::{FormatOptions, PatternID};

#[derive(Serialize)]
struct JsonOutput {
    version: String,
    summary: JsonSummary,
    patterns: Vec<JsonPattern>,
}

#[derive(Serialize)]
struct JsonSummary {
    total_lines: u64,
    pattern_count: usize,
    patterns_shown: usize,
    patterns_omitted: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_range: Option<JsonTimeRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_label: Option<String>,
}

#[derive(Serialize)]
struct JsonTimeRange {
    start: String,
    end: String,
}

#[derive(Serialize)]
struct JsonPattern {
    id: usize,
    template: String,
    count: u64,
    frequency_pct: f64,
    score: f64,
    severity: Severity,
    variables: Vec<JsonVariable>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    example_lines: Vec<String>,
}

#[derive(Serialize)]
struct JsonVariable {
    slot: usize,
    #[serde(rename = "type")]
    var_type: String,
    label: String,
    unique_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    numeric: Option<JsonNumeric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_values: Option<Vec<JsonTopValue>>,
}

#[derive(Serialize)]
struct JsonNumeric {
    mean: f64,
    p50: f64,
    p99: f64,
    min: f64,
    max: f64,
}

#[derive(Serialize)]
struct JsonTopValue {
    value: String,
    count: u64,
    pct: f64,
}

pub fn format(store: &PatternStore, opts: &FormatOptions, scores: &HashMap<PatternID, PatternScore>) -> String {
    let patterns = store.sorted_patterns();
    let total_lines = store.global_line_count;
    let top_n = opts.top.min(patterns.len());

    let time_range = match (store.global_first_ts, store.global_last_ts) {
        (Some(first), Some(last)) => Some(JsonTimeRange {
            start: first.to_rfc3339(),
            end: last.to_rfc3339(),
        }),
        _ => None,
    };

    let json_patterns: Vec<JsonPattern> = patterns
        .iter()
        .take(top_n)
        .map(|p| {
            let pct = if total_lines > 0 {
                (p.count as f64 / total_lines as f64 * 1000.0).round() / 10.0
            } else {
                0.0
            };

            let variables: Vec<JsonVariable> = p
                .variables
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let label = infer_label(&p.template, i, v.var_type);
                    let numeric = v.numeric.as_ref().map(|n| JsonNumeric {
                        mean: (n.mean() * 10.0).round() / 10.0,
                        p50: n.quantile(0.5).unwrap_or(0.0),
                        p99: n.quantile(0.99).unwrap_or(0.0),
                        min: n.min,
                        max: n.max,
                    });

                    let top_values = if v.categorical.unique_count() <= 20 {
                        let top = v.categorical.top_k(10);
                        if top.is_empty() {
                            None
                        } else {
                            Some(
                                top.into_iter()
                                    .map(|(val, count, pct)| JsonTopValue {
                                        value: val,
                                        count,
                                        pct: (pct * 10.0).round() / 10.0,
                                    })
                                    .collect(),
                            )
                        }
                    } else {
                        None
                    };

                    JsonVariable {
                        slot: i,
                        var_type: format!("{}", v.var_type),
                        label,
                        unique_count: v.categorical.unique_count(),
                        numeric,
                        top_values,
                    }
                })
                .collect();

            let ps = scores.get(&p.pattern_id);
            JsonPattern {
                id: p.pattern_id,
                template: p.template.clone(),
                count: p.count,
                frequency_pct: pct,
                score: ps.map(|s| (s.score * 10.0).round() / 10.0).unwrap_or(p.count as f64),
                severity: ps.map(|s| s.severity).unwrap_or(Severity::Info),
                variables,
                example_lines: p.example_lines.items().to_vec(),
            }
        })
        .collect();

    let pattern_count = store.patterns.len();
    let output = JsonOutput {
        version: "0.1.0".to_string(),
        summary: JsonSummary {
            total_lines,
            pattern_count,
            patterns_shown: top_n,
            patterns_omitted: pattern_count - top_n,
            time_range,
            source_label: opts.source_label.clone(),
        },
        patterns: json_patterns,
    };

    serde_json::to_string(&output).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
}
