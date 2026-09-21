# ctrlb-decompose — Architecture & Operational Guidelines

## Overview
`ctrlb-decompose` is a single-pass streaming log pattern extraction engine written in Rust. It compresses raw, high-volume logs into structured templates with typed variable statistics (quantiles, cardinality, distributions), severity scoring, anomaly detection, and temporal correlation.

---

## Processing Pipeline

```
Raw Log Stream
     │
     ▼
[ 1. Timestamp Extraction & Normalization ] (src/timestamp.rs)
     │  - Regex matching for Apache bracket, ISO-8601, RFC3339, Syslog, Common Log, Unix Epoch.
     │  - Replaces timestamp with `<TS>` marker.
     ▼
[ 2. CLP Pre-Encoding ] (src/extraction/clp/)
     │  - CLP (Compressed Log Processor) byte-level tokenization.
     │  - Replaces numeric and structured values with compact byte placeholders:
     │    `\x11` (Integer), `\x13` (Float), `\x12` (Dictionary Variable).
     │  - Delimiter rules: `!(c == '+' || c == '-' || c == '.' || c.is_ascii_digit() || c.is_ascii_uppercase() || c == '_' || c.is_ascii_lowercase())`
     │  - Variable token rules: contains decimal digits, preceded by '=', or multi-digit hex.
     ▼
[ 3. Drain3 Tree Clustering ] (src/extraction/drain3.rs)
     │  - Fixed-depth prefix tree partitioned by token length.
     │  - Compares token sequences using similarity threshold (`sim_th = 0.4` default).
     │  - Produces template strings containing `<*>` wildcards for variable positions.
     ▼
[ 4. Variable Reconstruction & Typing ] (src/extraction/pipeline.rs)
     │  - Reconciles Drain3 `<*>` wildcards with CLP byte placeholders.
     │  - Classifies extracted variables into `VarType`:
     │    `UUID`, `IPv4`, `IPv6`, `Duration`, `Timestamp`, `Float`, `Integer`, `HexID`, `Enum`, `String`.
     ▼
[ 5. Streaming Statistics ] (src/stats.rs)
     │  - `NumericStats`: Tracks distributions with `DDSketch` (p50, p99, min, max, mean).
     │  - `CategoricalStats`: Exact counts up to 10,000 items (`CARDINALITY_CAP`); falls back to `HyperLogLogPlus`.
     │    * CRITICAL: `hll.count()` must be evaluated lazily during output generation, NEVER per log line.
     │  - `BoundedVec`: Reservoir sampling for example raw lines.
     ▼
[ 6. Scoring, Anomaly Detection, & Correlation ]
     │  - `src/scoring.rs`: Priority rank ($count \times keyword\_weight \times anomaly\_multiplier$).
     │    Prefix scanning bounded to first 100 chars (ERROR=10.0, WARN=5.0, DEBUG=0.1, INFO=1.0).
     │  - `src/anomaly.rs`: Frequency spikes, low cardinality invariants, timeout clusters, bimodal latency.
     │  - `src/correlation.rs`: Pearson correlation across time buckets, shared variables, error cascade lags.
     ▼
[ 7. Output Formatters ] (src/format/)
        - `human`: ANSI colored terminal view with reduction statistics and severity tags.
        - `llm`: Token-efficient markdown partitioned into Critical Patterns and High-Volume Patterns.
        - `json`: Structured machine-readable schema with summary and variable metadata.
```

---

## Key Invariants & Gotchas

1. **HyperLogLog Performance Hot Path**:
   Never invoke `HyperLogLogPlus::count()` inside `CategoricalStats::update()`. `count()` scans $2^{16} = 65,536$ registers on every call. Always evaluate cardinality lazily inside `unique_count()`.

2. **CLP Variable Semantics**:
   Tokens containing only alphabetic characters without digits, without `=` prefix, and not matching hex are treated as schema logtype constants by CLP, NOT dictionary variables.

3. **Deterministic Type Voting**:
   When resolving variable types across multiple observations in `VarSlotStats`, avoid non-deterministic `HashMap::iter().max_by_key()` without deterministic tie-breaking.

4. **Formatting Partitioning in LLM Mode**:
   When critical patterns (ERROR/WARN) are detected, `format/llm.rs` outputs `### Critical Patterns (errors, warnings):` followed by `### High-Volume Patterns`. Flat `### Patterns` is only used when no critical patterns exist.
