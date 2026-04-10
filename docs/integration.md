# Integration Guide

How to integrate ctrlb-decompose into your tools and workflows.

---

## Claude Code / LLM Agents

Use `--llm` mode to generate token-efficient summaries for LLM context injection. This is the most common integration pattern.

### Quick start

```bash
# Analyze a log file and get LLM-ready output
ctrlb-decompose app.log --llm --context 2
```

When in LLM mode, the banner is suppressed automatically. Add `--quiet` to also suppress the progress line on stderr:

```bash
ctrlb-decompose app.log --llm --quiet --context 2 2>/dev/null
```

### Workflow

1. **Decompose first:** Run ctrlb-decompose to get the pattern summary
2. **Identify patterns of interest:** Look at error/warn patterns, p99 spikes
3. **Drill into specifics:** Use `grep` or `--context N` with a higher N to see more examples
4. **Ask the LLM:** Feed the summary as context for questions like "what caused this outage?"

### CLAUDE.md snippet

Add this to your project's CLAUDE.md so Claude Code knows to use it:

```markdown
## Log Analysis

When analyzing large log files (>100 lines), run ctrlb-decompose first:
\`\`\`bash
ctrlb-decompose <file> --llm --context 2 --quiet 2>/dev/null
\`\`\`
```

---

## Common Log Sources

### journalctl / systemd

```bash
# Last hour of a specific service
journalctl -u nginx --since "1 hour ago" --no-pager | ctrlb-decompose --llm

# All errors from all services since boot
journalctl -p err -b --no-pager | ctrlb-decompose --llm --context 2

# Specific time window
journalctl --since "2024-03-10 08:00" --until "2024-03-10 09:00" --no-pager | \
  ctrlb-decompose --llm

# Multiple services
journalctl -u nginx -u postgres -u myapp --since "30 min ago" --no-pager | \
  ctrlb-decompose --llm

# With source label
journalctl -u nginx --since "1 hour ago" --no-pager | \
  ctrlb-decompose --llm --source-label "nginx"
```

### kubectl

```bash
# Single pod
kubectl logs my-pod | ctrlb-decompose --llm

# All pods matching a label
kubectl logs -l app=api --all-containers --since=1h | \
  ctrlb-decompose --llm --source-label "api-pods"

# Previous container (after a crash)
kubectl logs my-pod --previous | ctrlb-decompose --llm --context 3

# Specific container in a multi-container pod
kubectl logs my-pod -c sidecar --since=30m | ctrlb-decompose --llm

# Follow + analyze (useful for live debugging, Ctrl-C when done)
kubectl logs -l app=api --since=5m -f | ctrlb-decompose --llm

# Multiple namespaces — compare
kubectl logs -n prod -l app=api --since=1h | \
  ctrlb-decompose --json --quiet --source-label "prod" > /tmp/prod.json
kubectl logs -n staging -l app=api --since=1h | \
  ctrlb-decompose --json --quiet --source-label "staging" > /tmp/staging.json
```

### docker / docker compose

```bash
# Single container
docker logs my-container --since 1h | ctrlb-decompose --llm

# With tail limit
docker logs my-container --tail 5000 | ctrlb-decompose --llm

# Docker compose — all services
docker compose logs --since 1h | ctrlb-decompose --llm

# Docker compose — specific services
docker compose logs api db --since 30m | ctrlb-decompose --llm

# Docker compose — with source label per service
docker compose logs api --since 1h | \
  ctrlb-decompose --llm --source-label "api"
docker compose logs worker --since 1h | \
  ctrlb-decompose --llm --source-label "worker"
```

### SSH / Remote

```bash
# Tail remote logs
ssh prod-server 'tail -10000 /var/log/app/api.log' | ctrlb-decompose --llm

# Compressed logs
ssh prod-server 'zcat /var/log/app/api.log.1.gz' | ctrlb-decompose --llm

# Multiple hosts — compare
for host in prod-{1..3}; do
  ssh $host 'tail -5000 /var/log/app.log' | \
    ctrlb-decompose --llm --source-label "$host"
  echo "---"
done
```

---

## CI Pipelines

### GitHub Actions

Add a step that summarizes build/test logs on failure:

```yaml
- name: Run tests
  id: tests
  run: |
    cargo test 2>&1 | tee /tmp/test-output.log
  continue-on-error: true

- name: Summarize test output
  if: steps.tests.outcome == 'failure'
  run: |
    ctrlb-decompose --llm --top 10 --context 3 /tmp/test-output.log
```

To post the summary as a PR comment:

```yaml
- name: Post log summary
  if: steps.tests.outcome == 'failure' && github.event_name == 'pull_request'
  run: |
    SUMMARY=$(ctrlb-decompose --llm --top 5 --context 2 --quiet /tmp/test-output.log 2>/dev/null)
    gh pr comment ${{ github.event.pull_request.number }} --body "## Test Failure Summary

    $SUMMARY"
```

---

## JSON Programmatic Use

The `--json` output is structured for machine consumption.

### Output schema

```json
{
  "version": "0.1.0",
  "summary": {
    "total_lines": 50000,
    "pattern_count": 23,
    "patterns_shown": 20,
    "patterns_omitted": 3,
    "source_label": "api-pod",
    "time_range": { "start": "...", "end": "..." }
  },
  "patterns": [
    {
      "id": 1,
      "template": "<TS> ERROR [<*>] Timeout ...",
      "count": 342,
      "frequency_pct": 8.2,
      "score": 57.0,
      "severity": "error",
      "variables": [
        {
          "slot": 0,
          "type": "IPv4",
          "label": "ip",
          "unique_count": 3,
          "numeric": null,
          "top_values": [
            { "value": "10.0.5.12", "count": 305, "pct": 89.2 }
          ]
        }
      ],
      "example_lines": ["..."]
    }
  ]
}
```

### Parsing examples

```bash
# Get all error templates
ctrlb-decompose --json --quiet app.log | jq -r '.patterns[] | select(.severity == "error") | .template'

# Get p99 latency for all duration variables
ctrlb-decompose --json --quiet app.log | \
  jq '.patterns[].variables[] | select(.type == "Duration") | {label, p99: .numeric.p99}'

# Count total error lines
ctrlb-decompose --json --quiet app.log | \
  jq '[.patterns[] | select(.severity == "error") | .count] | add'
```

---

## AI Assistant Setup

This repo ships config files for Claude Code, Cursor, Codex, and GitHub Copilot so log analysis works out of the box. To set up ctrlb-decompose in your own project, see [docs/ai-setup.md](ai-setup.md).
