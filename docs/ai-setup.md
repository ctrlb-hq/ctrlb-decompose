# AI Assistant Setup

How to configure AI coding assistants to use ctrlb-decompose for log analysis.

This repo ships config files for Claude Code, Cursor, Codex, and GitHub Copilot. Clone the repo and they work automatically. To set up ctrlb-decompose in **your own project**, follow the guides below.

---

## Prerequisites

Install ctrlb-decompose and make sure it's on your PATH or note the binary location:

```bash
# Homebrew
brew tap ctrlb-hq/tap && brew install ctrlb-decompose

# Or build from source
git clone https://github.com/ctrlb-hq/ctrlb-decompose.git
cd ctrlb-decompose && cargo build --release
# Binary at ./target/release/ctrlb-decompose
```

---

## Claude Code

### CLAUDE.md (recommended)

Add to your project's `CLAUDE.md`:

```markdown
## Log Analysis

When analyzing log files (>100 lines), run ctrlb-decompose first:

\`\`\`bash
ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null
\`\`\`

Key flags: `--llm` (token-efficient), `--json` (structured), `--source-label <name>`, `--sim-threshold <0.0-1.0>`.
Workflow: decompose first, identify patterns, then drill into raw logs.
\`\`\`bash
kubectl logs <pod> | ctrlb-decompose --llm --source-label "pod-name"
\`\`\`
```

### Optional: Hook for large log files

Create `.claude/hooks/check-log-size.sh`:

```bash
#!/bin/bash
# Warn when Claude reads large log files
INPUT=$(cat)
FILEPATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

if [[ "$FILEPATH" == *.log ]]; then
  LINES=$(wc -l < "$FILEPATH" 2>/dev/null || echo 0)
  if (( LINES > 100 )); then
    echo "{\"decision\": \"allow\", \"reason\": \"Large log file ($LINES lines). Consider running: ctrlb-decompose $FILEPATH --llm --context 2\"}" 
  fi
fi
```

Add to `.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Read",
        "hooks": [
          {
            "type": "command",
            "command": "bash .claude/hooks/check-log-size.sh"
          }
        ]
      }
    ]
  }
}
```

---

## Cursor

### .cursorrules (legacy, widely supported)

Create `.cursorrules` in your project root:

```
When working with log files (>100 lines), run ctrlb-decompose first:

    ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null

This compresses log lines into patterns with typed variables, quantile stats, and anomaly detection.
Key flags: --llm, --json, --source-label <name>, --sim-threshold <0.0-1.0>
Works with stdin: kubectl logs pod | ctrlb-decompose --llm
```

### .cursor/rules/ (modern, auto-activating)

Create `.cursor/rules/log-analysis.mdc`:

```markdown
---
description: Use ctrlb-decompose to analyze log files before reading them raw
globs: **/*.log
alwaysApply: false
---

# Log File Analysis

Run ctrlb-decompose first to get a structural summary:

\`\`\`bash
ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null
\`\`\`

Workflow: decompose first, identify patterns, then grep or use --context N for more examples.
```

The `globs: **/*.log` ensures this rule auto-activates when any `.log` file is referenced.

---

## Codex / OpenAI

Create `AGENTS.md` in your project root:

```markdown
# AGENTS.md

## Log Analysis

When working with log files (>100 lines), run ctrlb-decompose first:

\`\`\`bash
ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null
\`\`\`

Key flags: --llm, --json, --source-label <name>, --sim-threshold <0.0-1.0>
Works with stdin: kubectl logs pod | ctrlb-decompose --llm
```

---

## GitHub Copilot

Create `.github/copilot-instructions.md`:

```markdown
When working with log files (>100 lines), run ctrlb-decompose first:

    ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null

Key flags: --llm (token-efficient), --json (structured), --source-label <name>, --sim-threshold <0.0-1.0>
```

---

## Shipped config files in this repo

| File | Tool | Purpose |
|------|------|---------|
| `CLAUDE.md` | Claude Code | Project guidance + log analysis instructions |
| `.cursorrules` | Cursor (legacy) | Project-wide AI instructions |
| `.cursor/rules/log-analysis.mdc` | Cursor (modern) | Auto-activates on `.log` files |
| `AGENTS.md` | Codex / OpenAI | Project instructions for Codex agents |
| `.github/copilot-instructions.md` | GitHub Copilot | Project-level Copilot instructions |
