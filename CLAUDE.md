# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ctrlb-decompose compresses raw log lines into structural patterns with typed variables, quantile statistics, anomaly detection, and temporal correlations. It runs as a CLI tool, a Rust library, or a WASM module in the browser.

## Build & Test Commands

```bash
# Build
cargo build
cargo build --release

# Test
cargo test --locked
cargo test <test_name>             # Run a single test

# Lint
cargo clippy

# Build without default features (library-only, no CLI)
cargo build --no-default-features

# WASM build
wasm-pack build --target web --out-dir web/pkg -- --no-default-features --features wasm
```

## Architecture

**Two-stage normalization + clustering pipeline** (single-pass, streaming):

1. **Timestamp extraction** (`src/timestamp.rs`) — regex-based, stripped before further processing
2. **CLP encoding** (`src/extraction/clp/`) — normalizes variables (ints, floats, IPs, hex) into typed placeholders
3. **Drain3 clustering** (`src/extraction/drain3.rs`) — tree-based prefix clustering on logtypes with LRU eviction
4. **Variable classification** (`src/extraction/drain3.rs`) — merges CLP-decoded values with Drain3 wildcards, classifies into semantic types (IPv4, UUID, Duration, HexID, Integer, Float, Enum, String, etc.)
5. **Statistics** (`src/stats.rs`) — DDSketch quantiles (~200 bytes/slot), HyperLogLog++ cardinality, top-k, temporal bucketing, reservoir-sampled examples
6. **Anomaly detection** (`src/anomaly.rs`) — frequency spikes, error cascades, bimodal distributions, low cardinality
7. **Scoring & correlation** (`src/scoring.rs`, `src/correlation.rs`) — keyword severity, Pearson temporal co-occurrence, shared variables
8. **Output formatting** (`src/format/`) — human (ANSI terminal), llm (compact markdown), json (structured)

**Entry points:**
- CLI: `main.rs` → `lib.rs::run(args)`
- Library: `lib.rs::process_log_text(input, opts) -> AnalysisOutput`
- WASM: `wasm.rs::analyze_logs(input, format, top_n, context_lines) -> String`

## Feature Gates

- `cli` (default) — includes `clap` and `colored` for terminal use
- `wasm` — includes `wasm-bindgen` and `serde-wasm-bindgen` for browser use
- The core library is WASM-safe (no stdin/filesystem deps)
- Crate type is `["cdylib", "rlib"]` for dual WASM + library output

## Key Design Decisions

- **Single-pass streaming**: no second pass over data; all stats accumulated incrementally
- **Memory-bounded**: Drain3 LRU (default 10k clusters), DDSketch fixed-size quantiles, HyperLogLog++ fixed-size cardinality, reservoir sampling for examples
- **Lazy regex compilation**: `once_cell::sync::Lazy` for all regex patterns
- **Minimum Rust version**: 1.94.0

## Testing

Tests are in `tests/integration.rs` using the fixture at `tests/fixtures/sample.log`. Snapshot testing uses `insta`. Benchmarks use `criterion` in `benches/pipeline.rs`.

## CI

- `ci.yml` — runs on push to main and PRs: `cargo test`, `clippy`, feature matrix (no-default-features, wasm target)
- `release.yml` — triggered by `v*.*.*` tags: cargo-dist multi-platform binaries + homebrew
- `wasm-deploy.yml` — deploys WASM build to GitHub Pages on push to main

## Using ctrlb-decompose for log analysis

When working with large log files (>100 lines), use ctrlb-decompose to get a structured summary before reading raw logs. This saves context window and surfaces errors/anomalies immediately.

```bash
# Analyze a log file (LLM-optimized output, 2 example lines per pattern)
./target/release/ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null

# Pipe from any command
kubectl logs <pod> | ./target/release/ctrlb-decompose --llm --context 2 --quiet --source-label "pod-name" 2>/dev/null
journalctl -n 5000 | ./target/release/ctrlb-decompose --llm --context 2 --quiet 2>/dev/null

# JSON output for programmatic use
./target/release/ctrlb-decompose <file> --json --quiet 2>/dev/null

# Tune clustering granularity (lower = more aggressive merging, higher = more patterns)
./target/release/ctrlb-decompose <file> --llm --sim-threshold 0.6
```

In LLM mode, the banner is suppressed automatically. The `--quiet` flag suppresses the progress line on stderr.

**Workflow**: Run `--llm` first to identify patterns of interest, then use `--context N` with higher N or grep for specific patterns in the raw file.
