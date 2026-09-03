This repository contains ctrlb-decompose, a CLI tool that compresses raw log lines into structural patterns with typed variables, quantile statistics, and anomaly detection.

## Log Analysis

When working with log files (>100 lines), run ctrlb-decompose first:

```bash
./target/release/ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null
```

Key flags:
- `--llm` — Token-efficient markdown (errors first, quantile stats inline)
- `--json` — Structured JSON for programmatic use
- `--source-label <name>` — Tag output with source name
- `--sim-threshold <0.0-1.0>` — Tune clustering granularity (default 0.5)

Works with stdin: `kubectl logs pod | ./target/release/ctrlb-decompose --llm`

## Build & Test

```bash
cargo build --release
cargo test --locked
cargo clippy
```
