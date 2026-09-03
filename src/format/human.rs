use std::collections::HashMap;

#[cfg(feature = "cli")]
use colored::Colorize;

use crate::scoring::{PatternScore, Severity};
use crate::stats::PatternStore;
use crate::types::{FormatOptions, PatternID};

pub fn format(store: &PatternStore, opts: &FormatOptions, scores: &HashMap<PatternID, PatternScore>) -> String {
    let mut out = String::new();

    #[cfg(feature = "cli")]
    if opts.no_color {
        colored::control::set_override(false);
    }

    let patterns = store.sorted_patterns();
    let pattern_count = store.patterns.len();
    let total_lines = store.global_line_count;
    let reduction = if total_lines > 0 {
        (1.0 - pattern_count as f64 / total_lines as f64) * 100.0
    } else {
        0.0
    };

    // Header
    if !opts.no_banner {
        let label_part = match &opts.source_label {
            Some(label) => format!(" ({})", label),
            None => String::new(),
        };
        let header = format!(
            " ctrlb-decompose{}: {} lines -> {} patterns ({:.1}% reduction) ",
            label_part,
            format_count(total_lines),
            pattern_count,
            reduction
        );
        let width = header.len() + 2;
        let border = "\u{2500}".repeat(width);

        out.push_str(&format!("\u{250c}{}\u{2510}\n", border));
        out.push_str(&format!("\u{2502}{}\u{2502}\n", header));
        out.push_str(&format!("\u{2514}{}\u{2518}\n", border));

        if let (Some(first), Some(last)) = (store.global_first_ts, store.global_last_ts) {
            out.push_str(&format!(
                "  Time range: {} -> {}\n",
                first.format("%H:%M:%S UTC"),
                last.format("%H:%M:%S UTC")
            ));
        }
        out.push('\n');
    }

    // Patterns
    let top_n = opts.top.min(patterns.len());
    let shown_lines: u64 = patterns.iter().take(top_n).map(|p| p.count).sum();
    let shown_pct = if total_lines > 0 {
        shown_lines as f64 / total_lines as f64 * 100.0
    } else {
        0.0
    };

    if pattern_count > top_n {
        out.push_str(&format!(
            "Showing {} of {} patterns ({:.1}% of lines). Use --top {} to show all.\n\n",
            top_n, pattern_count, shown_pct, pattern_count
        ));
    }

    for (rank, pattern) in patterns.iter().take(top_n).enumerate() {
        let pct = if total_lines > 0 {
            pattern.count as f64 / total_lines as f64 * 100.0
        } else {
            0.0
        };

        let severity_tag = scores
            .get(&pattern.pattern_id)
            .map(|s| match s.severity {
                Severity::Error => {
                    #[cfg(feature = "cli")]
                    if !opts.no_color {
                        return format!(" {}", "[ERROR]".red().bold());
                    }
                    " [ERROR]".to_string()
                }
                Severity::Warn => {
                    #[cfg(feature = "cli")]
                    if !opts.no_color {
                        return format!(" {}", "[WARN]".yellow().bold());
                    }
                    " [WARN]".to_string()
                }
                _ => String::new(),
            })
            .unwrap_or_default();

        out.push_str(&format!(
            "Pattern #{}{} ({} occurrences, {:.1}%)\n",
            rank + 1,
            severity_tag,
            format_count(pattern.count),
            pct
        ));
        out.push_str(&format!("  \"{}\"\n", pattern.template));

        // Variables
        if !pattern.variables.is_empty() {
            out.push_str("  Variables:\n");
            for var in &pattern.variables {
                let unique = var.categorical.unique_count();
                out.push_str(&format!("    {:10}", format!("{}:", var.var_type)));

                if let Some(ref numeric) = var.numeric {
                    let mean = numeric.mean();
                    let p50 = numeric.quantile(0.5).unwrap_or(0.0);
                    let p99 = numeric.quantile(0.99).unwrap_or(0.0);
                    out.push_str(&format!(
                        "mean={:.0}, p50={:.0}, p99={:.0}, min={:.0}, max={:.0}\n",
                        mean, p50, p99, numeric.min, numeric.max
                    ));
                } else {
                    let top = var.categorical.top_k(5);
                    if !top.is_empty() && unique <= 20 {
                        let parts: Vec<String> = top
                            .iter()
                            .map(|(v, _, pct)| format!("{} ({:.1}%)", v, pct))
                            .collect();
                        out.push_str(&format!("{}\n", parts.join(", ")));
                    } else {
                        out.push_str(&format!("{} unique values\n", format_count(unique)));
                    }
                }
            }
        }

        // Example lines
        if opts.context > 0 && !pattern.example_lines.items().is_empty() {
            out.push_str("  Examples:\n");
            for example in pattern.example_lines.items() {
                out.push_str(&format!("    | {}\n", example));
            }
        }

        out.push('\n');
    }

    out
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        let s = n.to_string();
        let mut result = String::new();
        for (i, c) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                result.push(',');
            }
            result.push(c);
        }
        result.chars().rev().collect()
    } else {
        n.to_string()
    }
}
