use wasm_bindgen::prelude::*;

use crate::format::format_output;
use crate::types::{FormatOptions, OutputMode};

/// Analyze log lines and return formatted output.
///
/// - `input`: raw log text (newline-delimited)
/// - `format`: one of "human", "llm", "json"
/// - `top_n`: number of top patterns to show
/// - `context_lines`: number of example lines per pattern
#[wasm_bindgen]
pub fn analyze_logs(input: &str, format: &str, top_n: u32, context_lines: u32) -> String {
    let output_mode = match format {
        "json" => OutputMode::Json,
        "llm" => OutputMode::Llm,
        _ => OutputMode::Human,
    };

    let context = if output_mode == OutputMode::Llm && context_lines == 0 {
        2
    } else {
        context_lines as usize
    };

    let opts = FormatOptions {
        top: top_n as usize,
        context,
        no_color: true,
        no_banner: false,
        output_mode,
        source_label: None,
    };

    let result = crate::process_log_text(input, &opts);
    format_output(&result.store, &opts, &result.scores)
}

/// Analyze log lines and return JSON output.
#[wasm_bindgen]
pub fn analyze_logs_json(input: &str, top_n: u32, context_lines: u32) -> String {
    analyze_logs(input, "json", top_n, context_lines)
}
