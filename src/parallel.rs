use anyhow::Result;
use rayon::prelude::*;
use std::fs::File;

use crate::AnalysisOutput;
use crate::anomaly::detect_anomalies;
use crate::extraction::drain3::Config;
use crate::extraction::pipeline::ClpDrainPipeline;
use crate::scoring::compute_scores;
use crate::stats::PatternStore;
use crate::timestamp::{extract_timestamp, strip_timestamp};
use crate::types::FormatOptions;

/// Find newline-aligned byte boundaries to split a buffer into `num_chunks` independent slices.
pub fn find_chunk_boundaries(data: &[u8], num_chunks: usize) -> Vec<(usize, usize)> {
    if data.is_empty() || num_chunks <= 1 {
        return vec![(0, data.len())];
    }

    let chunk_size = data.len() / num_chunks;
    let mut boundaries = Vec::with_capacity(num_chunks);
    let mut start = 0;

    for _ in 0..num_chunks - 1 {
        let target_end = start + chunk_size;
        if target_end >= data.len() {
            break;
        }

        // Find the next newline character to align the boundary
        let end = match data[target_end..].iter().position(|&b| b == b'\n') {
            Some(pos) => target_end + pos + 1,
            None => data.len(),
        };

        if start < end {
            boundaries.push((start, end));
            start = end;
        }
        if start >= data.len() {
            break;
        }
    }

    if start < data.len() {
        boundaries.push((start, data.len()));
    }

    boundaries
}

/// Process a log file in parallel using memory-mapped chunking and Rayon map-reduce.
pub fn process_file_parallel(path: &str, opts: &FormatOptions) -> Result<AnalysisOutput> {
    let file = File::open(path)?;
    // SAFETY: We only read from the mapped memory and assume the file is not concurrently truncated.
    let mmap = unsafe { memmap2::MmapOptions::new().map(&file)? };

    if mmap.is_empty() {
        let store = PatternStore::new(opts.context);
        return Ok(AnalysisOutput {
            store,
            scores: std::collections::HashMap::new(),
        });
    }

    // Determine chunk count based on available CPU cores and file size
    let num_threads = rayon::current_num_threads().max(1);
    // Use 2-4 chunks per thread for dynamic load balancing across variable-length lines
    let num_chunks = (num_threads * 2).clamp(1, 64);
    let boundaries = find_chunk_boundaries(&mmap, num_chunks);

    let context_lines = opts.context;

    // Parallel Map: Process each chunk with an isolated pipeline and store
    let mut global_store = boundaries
        .into_par_iter()
        .map(|(start, end)| {
            let chunk_bytes = &mmap[start..end];
            let chunk_str = match std::str::from_utf8(chunk_bytes) {
                Ok(s) => s,
                Err(_) => String::from_utf8_lossy(chunk_bytes).into_owned().leak(),
            };

            let mut pipeline = ClpDrainPipeline::new(Config::default());
            let mut store = PatternStore::new(context_lines);
            let mut line_number: u64 = 0;

            for line in chunk_str.lines() {
                let trimmed = line.trim_end_matches('\r');
                if trimmed.is_empty() {
                    continue;
                }
                line_number += 1;

                let ts_match = extract_timestamp(trimmed);
                let stripped = match &ts_match {
                    Some(ts) => strip_timestamp(trimmed, ts),
                    None => trimmed.to_string(),
                };

                let parsed = pipeline.process_line(&stripped);

                store.accumulate(
                    parsed.pattern_id,
                    &parsed.display_template,
                    &parsed.variables,
                    ts_match.map(|ts| ts.datetime),
                    trimmed,
                    line_number,
                );
            }

            store
        })
        // Parallel Reduce: Merge chunk stores into a unified global store
        .reduce(
            || PatternStore::new(context_lines),
            |mut acc, store| {
                acc.merge(store);
                acc
            },
        );

    global_store.finalize();

    let anomalies = detect_anomalies(&global_store);
    let scores = compute_scores(&global_store, &anomalies);

    Ok(AnalysisOutput {
        store: global_store,
        scores,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_chunk_boundaries_newline_aligned() {
        let text = b"line 1\nline 2\nline 3\nline 4\nline 5\n";
        let boundaries = find_chunk_boundaries(text, 2);

        assert!(!boundaries.is_empty());
        // Verify every chunk boundary ends with a newline byte
        for &(start, end) in &boundaries {
            assert_eq!(text[end - 1], b'\n');
            assert!(start < end);
        }
        // Verify all bytes are covered continuously
        assert_eq!(boundaries[0].0, 0);
        assert_eq!(boundaries.last().unwrap().1, text.len());
    }
}
