# AGENTS.md

This file provides guidance to Codex and other AI agents working with code in this repository.

## Project

ctrlb-decompose compresses raw log lines into structural patterns with typed variables, quantile statistics, anomaly detection, and temporal correlations. It runs as a CLI tool, a Rust library, or a WASM module.

## Build & Test

```bash
cargo build --release
cargo test --locked
cargo clippy
```

## Log Analysis

When working with log files (>100 lines), run ctrlb-decompose first to get a structural summary before reading raw logs:

```bash
./target/release/ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null
```

Key flags:
- `--llm` — Token-efficient markdown output (banner auto-suppressed, default context=2)
- `--json` — Structured JSON for programmatic use
- `--source-label <name>` — Tag output with source name
- `--sim-threshold <0.0-1.0>` — Tune clustering (default 0.5, lower = fewer patterns)
- `--top <N>` — Top N patterns (default 20)
- `--context <N>` — Example lines per pattern

Works with stdin: `kubectl logs pod | ./target/release/ctrlb-decompose --llm`

Workflow: decompose first, identify patterns, then drill into raw logs with grep or higher `--context`.

## Architecture

Two-stage normalization + clustering pipeline (single-pass, streaming):

1. Timestamp extraction (`src/timestamp.rs`)
2. CLP encoding (`src/extraction/clp/`) — normalizes variables into typed placeholders
3. Drain3 clustering (`src/extraction/drain3.rs`) — tree-based prefix clustering with LRU eviction
4. Variable classification — semantic types: IPv4, UUID, Duration, HexID, Integer, Float, Enum, String
5. Statistics (`src/stats.rs`) — DDSketch quantiles, HyperLogLog cardinality, top-k, reservoir sampling
6. Anomaly detection (`src/anomaly.rs`) — frequency spikes, error cascades, bimodal distributions
7. Scoring & correlation (`src/scoring.rs`, `src/correlation.rs`)
8. Output formatting (`src/format/`) — human, llm, json

Entry points:
- CLI: `main.rs` -> `lib.rs::run(args)`
- Library: `lib.rs::process_log_text(input, opts)`
- WASM: `wasm.rs::analyze_logs(input, format, top_n, context_lines)`
