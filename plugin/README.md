# ctrlb-decompose Claude Code Plugin

Adds log analysis to Claude Code via [ctrlb-decompose](https://github.com/ctrlb-hq/ctrlb-decompose) — compress millions of log lines into a handful of actionable patterns with typed variable statistics, anomaly detection, and severity scoring.

## Install

In Claude Code:

```
/plugin marketplace add ctrlb-hq/ctrlb-decompose
/plugin install ctrlb-decompose@ctrlb-hq
```

## Usage

Just ask Claude to analyze logs — the skill triggers automatically:

- "Analyze the errors in `/var/log/app.log`"
- "What are the most common patterns in this log file?"
- "Summarize these logs:" followed by pasted log lines

If `ctrlb-decompose` is not installed on your system, Claude will detect your OS
and walk you through installation (Homebrew, apt/rpm package, or binary download).
If those fail, it can build from source using the Rust toolchain.

## What the skill does

1. Checks if `ctrlb-decompose` is installed; installs it if not
2. Runs `ctrlb-decompose --llm` on the log file or pasted text
3. Interprets the output — surfaces errors first, explains anomalies, summarizes variable patterns

## Manual install of ctrlb-decompose

**macOS:**
```bash
brew tap ctrlb-hq/tap && brew install ctrlb-decompose
```

**Debian / Ubuntu:**
```bash
curl -LO https://github.com/ctrlb-hq/ctrlb-decompose/releases/latest/download/ctrlb-decompose_amd64.deb
sudo dpkg -i ctrlb-decompose_amd64.deb
```

**Build from source:**
```bash
git clone https://github.com/ctrlb-hq/ctrlb-decompose.git
cd ctrlb-decompose && cargo build --release
```
