# ctrlb-decompose

**Compress raw log lines into structural patterns with statistics, anomalies, and correlations.**

Turn millions of noisy log lines into a handful of actionable patterns — with typed variables, quantile stats, anomaly flags, and severity scoring. Runs as a CLI, in the browser via WASM, or as a Rust library.

```
$ cat server.log | ctrlb-decompose

┌────────────────────────────────────────────────────────────────────┐
│ ctrlb-decompose: 1,247,831 lines -> 43 patterns (99.9% reduction) │
└────────────────────────────────────────────────────────────────────┘

#1  [ERROR]  ██████████████████████  18,402 (1.5%)
    <TS> ERROR [<*>] Connection to <ip> timed out after <duration>

    ip          IPv4    unique=12     top: 10.0.1.15 (34%), 10.0.1.22 (28%)
    duration    Duration               p50=120ms  p99=4.8s

#2  [INFO]   ████████████████████    904,221 (72.5%)
    <TS> INFO  [<*>] Request from <ip> completed in <duration> status=<status>

    ip          IPv4    unique=1,847  top: 10.0.1.15 (12%), 10.0.1.22 (8%)
    duration    Duration               p50=23ms   p99=312ms
    status      Enum    unique=3      values: 200 (91%), 404 (6%), 500 (3%)
```

> Website coming soon.

---

## How It Works

ctrlb-decompose uses a **two-stage normalization and clustering pipeline** that processes logs in a single streaming pass with minimal memory footprint.

```
                         ┌──────────────────────────────────────────────┐
                         │            ctrlb-decompose pipeline          │
                         └──────────────────────────────────────────────┘

  Raw Log Lines
       │
       ▼
┌──────────────┐    Strip & parse timestamps (ISO 8601, Apache,
│  Timestamp   │    syslog, Unix epoch, etc.) into normalized
│  Extraction  │    <TS> markers with DateTime values.
└──────┬───────┘
       │
       ▼
┌──────────────┐    Replace integers, floats, IPs, and strings
│     CLP      │    with compact placeholder bytes. Structurally
│   Encoding   │    identical lines now produce the same "logtype."
└──────┬───────┘
       │
       ▼
┌──────────────┐    Tree-based similarity clustering (Drain3) groups
│   Drain3     │    logtypes into patterns. Differing tokens become
│  Clustering  │    <*> wildcards. Incremental — no second pass needed.
└──────┬───────┘
       │
       ▼
┌──────────────┐    Merge CLP-decoded values with Drain3 wildcard
│   Variable   │    positions. Classify each variable into semantic
│  Extraction  │    types: IPv4, UUID, Duration, Enum, Integer, etc.
│  & Typing    │
└──────┬───────┘
       │
       ▼
┌──────────────┐    DDSketch quantiles (p50/p99), HyperLogLog
│  Statistics  │    cardinality estimation, top-k values, temporal
│ Accumulation │    bucketing, and reservoir-sampled example lines.
└──────┬───────┘
       │
       ▼
┌──────────────┐    Frequency spikes, error cascades, low-cardinality
│   Anomaly    │    flags, bimodal distributions, and clustered
│  Detection   │    numeric detection.
└──────┬───────┘
       │
       ▼
┌──────────────┐    Keyword-based severity (ERROR > WARN > INFO > DEBUG),
│   Scoring    │    temporal co-occurrence, shared variable correlation,
│ & Correlation│    and error cascade detection across patterns.
└──────┬───────┘
       │
       ▼
┌──────────────┐
│    Output    │──── Human (ANSI terminal) / LLM (compact markdown) / JSON
└──────────────┘
```

### Stage 1 — CLP Encoding

[CLP (Compact Log Pattern)](https://www.cs.toronto.edu/~zzhao/clp/) encoding normalizes variable tokens into typed placeholders, so structurally identical lines produce identical logtypes regardless of the actual values:

```
Input:   "Request from 10.0.1.15 completed in 45ms status=200"
Logtype: "Request from <dict> completed in <float>ms status=<int>"
```

### Stage 2 — Drain3 Clustering

The Drain algorithm builds a prefix tree over logtypes and groups them by token similarity (configurable threshold, default 0.4). Where tokens diverge, the template gains a `<*>` wildcard. This runs incrementally — each line is processed once with no second pass.

### Variable Classification

Extracted variables are classified into semantic types for richer analysis:

| Type | Example | Detection |
|------|---------|-----------|
| `IPv4` / `IPv6` | `10.0.1.15` | CIDR pattern match |
| `UUID` | `550e8400-e29b-...` | 8-4-4-4-12 hex format |
| `Duration` | `45ms`, `3.2s` | Numeric + time unit suffix |
| `HexID` | `0x1a2b3c` | 4+ hex digits |
| `Integer` | `200` | Parses as i64 |
| `Float` | `3.14` | Contains `.`, parses as f64 |
| `Enum` | `ERROR` | Low cardinality (<=20 unique, top-3 >= 80%) |
| `Timestamp` | `2024-01-15T14:22:01Z` | RFC 3339 pattern |
| `String` | anything else | Fallback |

### Memory Efficiency

- **Drain3 clusters**: O(k) with LRU eviction (default 10k max)
- **Quantiles**: DDSketch — fixed ~200 bytes per numeric slot, no raw value storage
- **Cardinality**: HyperLogLog++ — ~200 bytes per high-cardinality variable
- **Examples**: Reservoir sampling — bounded buffer per pattern

---

## Installation

### macOS (Homebrew)

```bash
brew tap ctrlb-hq/tap
brew install ctrlb-decompose
```

### Debian / Ubuntu

```bash
curl -LO https://github.com/ctrlb-hq/ctrlb-decompose/releases/download/v0.1.0/ctrlb-decompose_0.1.0-1_amd64.deb
sudo dpkg -i ctrlb-decompose_0.1.0-1_amd64.deb
```

### Build from source

```bash
git clone https://github.com/ctrlb-hq/ctrlb-decompose.git
cd ctrlb-decompose
cargo build --release
# Binary at target/release/ctrlb-decompose
```

---

## Usage

```bash
# Pipe from stdin
cat /var/log/syslog | ctrlb-decompose

# Read from file
ctrlb-decompose server.log

# LLM-optimized output (compact, token-efficient)
ctrlb-decompose --llm app.log

# JSON output
ctrlb-decompose --json app.log

# Top 10 patterns with 3 example lines each
ctrlb-decompose --top 10 --context 3 app.log
```

### Options

```
ctrlb-decompose [OPTIONS] [FILE]

Arguments:
  [FILE]          Log file path (reads stdin if omitted or "-")

Options:
      --human         Human-readable output with colors (default)
      --llm           LLM-optimized compact markdown
      --json          Structured JSON output
      --top <N>       Show top N patterns (default: 20)
      --context <N>   Example lines per pattern (default: 0)
      --no-color      Disable ANSI colors
      --no-banner     Suppress header/footer
  -q, --quiet         Suppress progress messages
  -h, --help          Show help
  -V, --version       Show version
```

---

## Output Formats

| Format | Flag | Best for |
|--------|------|----------|
| **Human** | `--human` (default) | Terminal investigation — colored, visual bars |
| **LLM** | `--llm` | Feeding into LLMs — compact, token-efficient markdown |
| **JSON** | `--json` | Programmatic consumption — structured, machine-readable |

---

## Use Cases

ctrlb-decompose is useful for:

- **Incident triage** — Surface error patterns, p99 spikes, and host concentration in thousands of log lines. See [docs/use-cases.md](docs/use-cases.md#incident-triage).
- **Postmortem summary** — Generate LLM-friendly summaries for automated timeline reconstruction. See [docs/use-cases.md](docs/use-cases.md#postmortem-summary).
- **CI/CD log triage** — Find failure patterns buried in verbose build output. See [docs/use-cases.md](docs/use-cases.md#cicd-log-triage).
- **Monitoring** — Use `--json` for programmatic alerting thresholds. See [docs/use-cases.md](docs/use-cases.md#monitoring--alerting).
- **Log comparison** — Diff patterns before and after a deployment. See [docs/use-cases.md](docs/use-cases.md#log-comparison).

## Integration

ctrlb-decompose pipes into any workflow:

```bash
# Kubernetes pods
kubectl logs -l app=api --since=1h | ctrlb-decompose --llm --source-label "api"

# systemd services
journalctl -u nginx --since "1 hour ago" --no-pager | ctrlb-decompose --llm

# Docker Compose
docker compose logs api db --since 30m | ctrlb-decompose --llm

# CI failure summary
cargo test 2>&1 | ctrlb-decompose --llm --top 5 --context 2
```

See [docs/integration.md](docs/integration.md) for detailed recipes including Claude Code setup, GitHub Actions, SSH remote analysis, and JSON programmatic use.

---

## License

[MIT](LICENSE)
