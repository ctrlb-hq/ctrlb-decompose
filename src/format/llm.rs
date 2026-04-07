use std::collections::HashMap;

use crate::label::infer_label;
use crate::scoring::{PatternScore, Severity};
use crate::stats::{PatternStats, PatternStore};
use crate::types::{FormatOptions, PatternID};

pub fn format(
    store: &PatternStore,
    opts: &FormatOptions,
    scores: &HashMap<PatternID, PatternScore>,
) -> String {
    let mut out = String::new();

    let patterns = store.sorted_patterns();
    let total_lines = store.global_line_count;
    let pattern_count = store.patterns.len();

    // Header
    let time_range = match (store.global_first_ts, store.global_last_ts) {
        (Some(first), Some(last)) => format!(
            ", time span: {}-{} UTC",
            first.format("%H:%M:%S"),
            last.format("%H:%M:%S")
        ),
        _ => String::new(),
    };
    out.push_str(&format!(
        "## Log Analysis: {} lines -> {} patterns{}\n\n",
        total_lines, pattern_count, time_range
    ));

    // Partition patterns into critical vs high-volume.
    // Critical: only patterns with keyword severity >= Warn (ERROR, WARN, etc.).
    // Anomaly detection is left to the LLM — we just surface the data.
    let mut critical: Vec<&PatternStats> = Vec::new();
    let mut high_volume: Vec<&PatternStats> = Vec::new();

    for &p in &patterns {
        let is_critical = scores
            .get(&p.pattern_id)
            .map(|s| matches!(s.severity, Severity::Error | Severity::Warn))
            .unwrap_or(false);

        if is_critical {
            critical.push(p);
        } else {
            high_volume.push(p);
        }
    }

    // Sort critical by score descending
    critical.sort_by(|a, b| {
        let sa = scores.get(&a.pattern_id).map(|s| s.score).unwrap_or(0.0);
        let sb = scores.get(&b.pattern_id).map(|s| s.score).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    // High-volume already sorted by count desc (from sorted_patterns), just take top 5
    let high_volume_limit = 5;

    if critical.is_empty() {
        // Fallback: no critical patterns, show flat list sorted by count (current behavior)
        let top_n = opts.top.min(patterns.len());
        let shown_lines: u64 = patterns.iter().take(top_n).map(|p| p.count).sum();
        let shown_pct = if total_lines > 0 {
            shown_lines as f64 / total_lines as f64 * 100.0
        } else {
            0.0
        };

        format_section_header(&mut out, pattern_count, top_n, shown_pct);

        for (rank, pattern) in patterns.iter().take(top_n).enumerate() {
            format_pattern_full(&mut out, rank, pattern, total_lines, store, scores);
        }
    } else {
        // Two-section layout
        let critical_n = opts.top.min(critical.len());

        out.push_str(&format!(
            "### Critical Patterns (errors, warnings): {}\n",
            critical_n
        ));

        for (rank, pattern) in critical.iter().take(critical_n).enumerate() {
            format_pattern_full(&mut out, rank, pattern, total_lines, store, scores);
        }

        let hv_n = high_volume_limit.min(high_volume.len());
        if hv_n > 0 {
            out.push_str(&format!(
                "\n### High-Volume Patterns (top {} by count):\n",
                hv_n
            ));
            for (rank, pattern) in high_volume.iter().take(hv_n).enumerate() {
                let pct = if total_lines > 0 {
                    pattern.count as f64 / total_lines as f64 * 100.0
                } else {
                    0.0
                };
                let labeled_template = build_labeled_template_for(pattern);
                out.push_str(&format!(
                    "{}. [{:.1}%] \"{}\"    ({} lines)\n",
                    rank + 1,
                    pct,
                    labeled_template,
                    pattern.count
                ));
                // Always show 1 example for context
                if let Some(ex) = pattern.example_lines.items().first() {
                    out.push_str(&format!("   e.g. {}\n", truncate_long_tokens(ex)));
                }
            }
        }
    }

    out
}

fn format_section_header(out: &mut String, pattern_count: usize, top_n: usize, shown_pct: f64) {
    if pattern_count > top_n {
        let omitted = pattern_count - top_n;
        out.push_str(&format!(
            "### Patterns (showing {} of {}, {:.1}% of lines; {} more patterns omitted):\n",
            top_n, pattern_count, shown_pct, omitted
        ));
    } else {
        out.push_str(&format!(
            "### Patterns ({}, {:.1}% of lines):\n",
            pattern_count, shown_pct
        ));
    }
}

fn format_pattern_full(
    out: &mut String,
    rank: usize,
    pattern: &PatternStats,
    total_lines: u64,
    _store: &PatternStore,
    scores: &HashMap<PatternID, PatternScore>,
) {
    let pct = if total_lines > 0 {
        pattern.count as f64 / total_lines as f64 * 100.0
    } else {
        0.0
    };

    let severity_tag = scores
        .get(&pattern.pattern_id)
        .and_then(|s| match s.severity {
            Severity::Error => Some("ERROR, "),
            Severity::Warn => Some("WARN, "),
            _ => None,
        })
        .unwrap_or("");

    let labeled_template = build_labeled_template_for(pattern);

    out.push_str(&format!(
        "{}. [{}{:.1}%, {} lines] {}\n",
        rank + 1,
        severity_tag,
        pct,
        pattern.count,
        labeled_template
    ));

    // Variable summaries
    let mut var_parts = Vec::new();
    for (i, var) in pattern.variables.iter().enumerate() {
        let label = infer_label(&pattern.template, i, var.var_type);
        if let Some(ref numeric) = var.numeric {
            let mean = numeric.mean();
            let p50 = numeric.quantile(0.5).unwrap_or(0.0);
            let p99 = numeric.quantile(0.99).unwrap_or(0.0);
            var_parts.push(format!(
                "{}: mean={:.0} p50={:.0} p99={:.0} min={:.0} max={:.0}",
                label, mean, p50, p99, numeric.min, numeric.max
            ));
        } else {
            let top = var.categorical.top_k(5);
            let unique = var.categorical.unique_count();
            if !top.is_empty() && unique <= 20 {
                let vals: Vec<String> = top
                    .iter()
                    .map(|(v, _, pct)| format!("{}({:.0}%)", truncate_token(v), pct))
                    .collect();
                var_parts.push(format!("{}: {}", label, vals.join(" ")));
            } else {
                var_parts.push(format!("{}: {} unique", label, unique));
            }
        }
    }
    if !var_parts.is_empty() {
        out.push_str(&format!("   {}\n", var_parts.join(" | ")));
    }

    // Example raw lines (truncate long tokens to save LLM context)
    let examples = pattern.example_lines.items();
    if !examples.is_empty() {
        for ex in examples {
            out.push_str(&format!("   e.g. {}\n", truncate_long_tokens(ex)));
        }
    }
}

fn build_labeled_template_for(pattern: &PatternStats) -> String {
    let labels: Vec<String> = pattern
        .variables
        .iter()
        .enumerate()
        .map(|(i, v)| infer_label(&pattern.template, i, v.var_type))
        .collect();
    build_labeled_template(&pattern.template, &labels)
}

fn build_labeled_template(template: &str, labels: &[String]) -> String {
    let mut result = String::new();
    let mut label_idx = 0;

    for token in template.split_whitespace() {
        if !result.is_empty() {
            result.push(' ');
        }
        if token == "<*>" {
            if label_idx < labels.len() {
                result.push('{');
                result.push_str(&labels[label_idx]);
                result.push('}');
                label_idx += 1;
            } else {
                result.push_str("<*>");
            }
        } else {
            result.push_str(token);
        }
    }

    result
}

const MAX_TOKEN_LEN: usize = 50;

/// Truncate a single string if longer than MAX_TOKEN_LEN.
fn truncate_token(s: &str) -> String {
    if s.len() > MAX_TOKEN_LEN {
        let mut head = 20;
        while !s.is_char_boundary(head) {
            head -= 1;
        }
        let mut tail = s.len() - 10;
        while !s.is_char_boundary(tail) {
            tail += 1;
        }
        format!("{}...{}", &s[..head], &s[tail..])
    } else {
        s.to_string()
    }
}

/// Replace any whitespace-delimited token longer than MAX_TOKEN_LEN with
/// its first 20 chars + "..." + last 10 chars. This strips auth tokens,
/// base64 blobs, and long hex IDs that waste LLM context.
fn truncate_long_tokens(line: &str) -> String {
    line.split_whitespace()
        .map(|token| truncate_token(token))
        .collect::<Vec<_>>()
        .join(" ")
}
