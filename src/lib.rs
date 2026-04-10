pub mod anomaly;
pub mod correlation;
pub mod extraction;
pub mod format;
pub mod label;
pub mod scoring;
pub mod stats;
pub mod timestamp;
pub mod types;
#[cfg(feature = "wasm")]
pub mod wasm;

use std::collections::HashMap;

use crate::anomaly::detect_anomalies;
use crate::extraction::drain3::Config;
use crate::extraction::pipeline::ClpDrainPipeline;
use crate::scoring::{compute_scores, PatternScore};
use crate::stats::PatternStore;
use crate::timestamp::{extract_timestamp, strip_timestamp};
use crate::types::{FormatOptions, PatternID};

// ── CLI-only imports and types ──────────────────────────────────────────

#[cfg(feature = "cli")]
use std::fs::File;
#[cfg(feature = "cli")]
use std::io::{self, BufRead, BufReader};
#[cfg(feature = "cli")]
use anyhow::Result;
#[cfg(feature = "cli")]
use clap::Parser;
#[cfg(feature = "cli")]
use crate::format::format_output;
#[cfg(feature = "cli")]
use crate::types::OutputMode;

#[cfg(feature = "cli")]
#[derive(Parser, Debug)]
#[command(
    name = "ctrlb-decompose",
    version,
    about = "Compress raw log lines into structural patterns"
)]
pub struct Args {
    /// Log file to process (reads from stdin if omitted or "-")
    pub file: Option<String>,

    /// Output in human-readable format (default)
    #[arg(long)]
    pub human: bool,

    /// Output in LLM-optimized format (compact, token-efficient)
    #[arg(long)]
    pub llm: bool,

    /// Output in JSON format
    #[arg(long)]
    pub json: bool,

    /// Show top N patterns (default: 20)
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Include N example raw lines per pattern (default: 0)
    #[arg(long, default_value_t = 0)]
    pub context: usize,

    /// Disable terminal colors
    #[arg(long)]
    pub no_color: bool,

    /// Suppress the header/footer banners
    #[arg(long)]
    pub no_banner: bool,

    /// Suppress progress output on stderr
    #[arg(short, long)]
    pub quiet: bool,

    /// Label for the log source (shown in output header)
    #[arg(long)]
    pub source_label: Option<String>,
}

#[cfg(feature = "cli")]
impl Args {
    pub fn output_mode(&self) -> OutputMode {
        if self.json {
            OutputMode::Json
        } else if self.llm {
            OutputMode::Llm
        } else {
            OutputMode::Human
        }
    }

    pub fn to_format_options(&self) -> FormatOptions {
        FormatOptions {
            top: self.top,
            context: if self.llm && self.context == 0 { 2 } else { self.context },
            no_color: self.no_color,
            no_banner: self.no_banner || self.llm,
            output_mode: self.output_mode(),
            source_label: self.source_label.clone(),
        }
    }
}

#[cfg(feature = "cli")]
pub fn run(args: Args) -> Result<()> {
    let mut reader: Box<dyn BufRead> = match args.file.as_deref() {
        None | Some("-") => Box::new(BufReader::new(io::stdin())),
        Some(path) => Box::new(BufReader::new(File::open(path)?)),
    };

    let opts = args.to_format_options();

    let mut pipeline = ClpDrainPipeline::new(Config::default());
    let mut store = PatternStore::new(opts.context);
    let mut line_number: u64 = 0;

    let mut raw_buf = Vec::new();
    loop {
        raw_buf.clear();
        let bytes_read = reader.read_until(b'\n', &mut raw_buf)?;
        if bytes_read == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&raw_buf).trim_end_matches('\n').trim_end_matches('\r').to_string();
        if line.is_empty() {
            continue;
        }
        line_number += 1;

        let ts_match = extract_timestamp(&line);
        let stripped = match &ts_match {
            Some(ts) => strip_timestamp(&line, ts),
            None => line.clone(),
        };

        let parsed = pipeline.process_line(&stripped);

        store.accumulate(
            parsed.pattern_id,
            &parsed.display_template,
            &parsed.variables,
            ts_match.map(|ts| ts.datetime),
            &line,
            line_number,
        );
    }

    if !args.quiet {
        eprintln!(
            "Processed {} lines -> {} patterns",
            store.global_line_count,
            store.patterns.len()
        );
    }

    store.finalize();

    let anomalies = detect_anomalies(&store);
    let scores = compute_scores(&store, &anomalies);

    let output = format_output(&store, &opts, &scores);
    print!("{}", output);

    if !args.quiet && !opts.no_banner {
        eprintln!(
            "\nPowered by CtrlB \u{00b7} Search 5TB of logs in 614ms \u{2192} ctrlb.ai"
        );
    }

    Ok(())
}

// ── Core processing (no I/O, works in CLI and WASM) ─────────────────────

pub struct AnalysisOutput {
    pub store: PatternStore,
    pub scores: HashMap<PatternID, PatternScore>,
}

/// Process log text and return analysis results.
/// This is the WASM-friendly entry point — no filesystem, no stdin.
pub fn process_log_text(input: &str, opts: &FormatOptions) -> AnalysisOutput {
    let mut pipeline = ClpDrainPipeline::new(Config::default());
    let mut store = PatternStore::new(opts.context);
    let mut line_number: u64 = 0;

    for line in input.lines() {
        if line.is_empty() {
            continue;
        }
        line_number += 1;

        let ts_match = extract_timestamp(line);
        let stripped = match &ts_match {
            Some(ts) => strip_timestamp(line, ts),
            None => line.to_string(),
        };

        let parsed = pipeline.process_line(&stripped);

        store.accumulate(
            parsed.pattern_id,
            &parsed.display_template,
            &parsed.variables,
            ts_match.map(|ts| ts.datetime),
            line,
            line_number,
        );
    }

    store.finalize();

    let anomalies = detect_anomalies(&store);
    let scores = compute_scores(&store, &anomalies);

    AnalysisOutput { store, scores }
}
