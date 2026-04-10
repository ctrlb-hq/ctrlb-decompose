---
name: analyze-logs
description: >
  Use this skill when the user asks to: "analyze logs", "analyze this log file",
  "what's in my logs", "investigate errors in logs", "find patterns in log file",
  "summarize logs", "detect anomalies in logs", "parse log file", "understand log
  output", "what are the most common errors", or shares a log file path or pastes
  log content for review.
version: 1.0.0
---

# ctrlb-decompose Log Analysis

## Overview

Analyze log files or raw log text using [ctrlb-decompose](https://github.com/ctrlb-hq/ctrlb-decompose) — a CLI that compresses millions of log lines into a handful of typed patterns with variable statistics, anomaly detection, and severity scoring. Output is optimized for LLM consumption via `--llm`.

## Workflow

1. Gate large pastes — redirect to file path if volume is high
2. Check if `ctrlb-decompose` is installed; install if not
3. Run analysis on the file or pasted content
4. Interpret results — lead with problems, surface anomalies, suggest next steps

---

## Large Paste Policy

If the user signals they are about to paste log content (e.g. "here are my logs", "let me paste some logs"), ask first:

> About how many lines are you sharing? For more than ~100 lines, save to a file and share the path — it keeps context clean and ctrlb-decompose handles files directly:
> ```bash
> ctrlb-decompose --llm --top 20 /path/to/your.log
> ```

Proceed with pasted content only if the user insists or the volume is clearly small (a handful of lines).

**After processing pasted content:** do not quote, repeat, or reference individual raw log lines. Work only from ctrlb-decompose output — the raw lines are consumed.

---

## Step 1 — Verify Installation

```bash
which ctrlb-decompose
```

If found → skip to Step 2.

If not found → detect the OS:

```bash
uname -s
```

### macOS (Darwin)

```bash
which brew
```

**Homebrew found** — install ctrlb-decompose:
```bash
brew tap ctrlb-hq/tap && brew install ctrlb-decompose
```

**Homebrew not found** — install Homebrew first:
```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```
Then install ctrlb-decompose:
```bash
brew tap ctrlb-hq/tap && brew install ctrlb-decompose
```

### Linux

```bash
cat /etc/os-release
```

| Distro | Install command |
|--------|----------------|
| Debian / Ubuntu (`ID=debian`, `ID=ubuntu`, or `ID_LIKE` contains `debian`) | `curl -LO https://github.com/ctrlb-hq/ctrlb-decompose/releases/latest/download/ctrlb-decompose_amd64.deb && sudo dpkg -i ctrlb-decompose_amd64.deb` |
| RHEL / Fedora / CentOS (`ID` is `rhel`, `fedora`, or `centos`) | `curl -LO https://github.com/ctrlb-hq/ctrlb-decompose/releases/latest/download/ctrlb-decompose_amd64.rpm && sudo rpm -i ctrlb-decompose_amd64.rpm` |
| Other Linux | `curl -LO https://github.com/ctrlb-hq/ctrlb-decompose/releases/latest/download/ctrlb-decompose-linux-x86_64 && chmod +x ctrlb-decompose-linux-x86_64 && sudo mv ctrlb-decompose-linux-x86_64 /usr/local/bin/ctrlb-decompose` |

### Fallback — Build from Source

If any installation method fails (wrong architecture, missing binary, package manager error):

```bash
which git && which cargo
```

If `cargo` is missing, install Rust:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && source "$HOME/.cargo/env"
```

Clone and build:
```bash
git clone https://github.com/ctrlb-hq/ctrlb-decompose.git /tmp/ctrlb-decompose-build \
  && cd /tmp/ctrlb-decompose-build \
  && cargo build --release \
  && sudo cp target/release/ctrlb-decompose /usr/local/bin/ctrlb-decompose \
  && cd / && rm -rf /tmp/ctrlb-decompose-build
```

Confirm with `which ctrlb-decompose`, then continue to Step 2.

---

## Step 2 — Run Analysis

**File path provided:**
```bash
ctrlb-decompose --llm --top 20 "$FILE_PATH"
```

**Pasted log text:** Use the Write tool to save content to `/tmp/ctrlb_input.log` (handles multi-line text, special characters, and quotes correctly), then:
```bash
ctrlb-decompose --llm --top 20 /tmp/ctrlb_input.log && rm /tmp/ctrlb_input.log
```

Use `--top 20` by default. Increase to `--top 50` if the user wants more patterns. For all available flags and options, run `ctrlb-decompose --help`.

---

## Step 3 — Interpret Results

Present findings in this priority order:

| Priority | Focus | What to cover |
|----------|-------|---------------|
| 1st | **Problems** | ERROR, FATAL, WARN patterns — frequency, rate, associated variables |
| 2nd | **Anomalies** | Frequency spikes, high error rates, unexpectedly low-cardinality variables |
| 3rd | **Variable summaries** | Most common IPs, slowest durations (p99), notable enum distributions |
| 4th | **Next steps** | Which patterns to drill into, what filters or time ranges to investigate |

Keep the response grounded in ctrlb-decompose output. Do not speculate about root causes beyond what the pattern data supports.
