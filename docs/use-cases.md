# Use Cases

Real-world scenarios where ctrlb-decompose helps you make sense of logs faster.

---

## Incident Triage

**Problem:** Your API is returning 5xx errors. You have thousands of log lines from the last hour. Where do you start?

**Approach:** Pipe recent logs through ctrlb-decompose to surface error patterns with statistics.

```bash
# From a log file
ctrlb-decompose --context 3 /var/log/app/api.log

# From journalctl
journalctl -u myapp --since "1 hour ago" --no-pager | ctrlb-decompose --context 3
```

**What to look for:**
- ERROR/WARN patterns sorted by severity at the top
- p99 duration spikes indicating slow paths
- IP or host concentration (one backend causing failures?)
- Temporal clustering (all errors in a 2-minute window?)

**Example output:**
```
Pattern #1 [ERROR] (342 occurrences, 8.2%)
  "<TS> ERROR [<*>] Timeout connecting to <*>:<*> after <*>"
  Variables:
    IPv4:     10.0.5.12 (89.2%), 10.0.5.13 (10.8%)
    Duration: mean=5003, p50=5001, p99=5012, min=4998, max=5050
```

One IP accounts for 89% of timeouts. Investigate that host.

---

## Postmortem Summary

**Problem:** After an incident, you need to write a timeline for the postmortem. The raw logs are 500K lines.

**Approach:** Use `--llm` to generate a token-efficient summary, then feed it to an LLM for timeline reconstruction.

```bash
ctrlb-decompose --llm --context 2 incident-logs.log > /tmp/summary.txt

# Then in Claude Code or any LLM chat:
# "Here are the decomposed log patterns from the incident.
#  Write a timeline of what happened:"
# <paste summary>
```

The `--llm` format prioritizes errors/warnings first and includes quantile stats inline, giving the LLM everything it needs without drowning it in raw lines.

---

## CI/CD Log Triage

**Problem:** A CI build failed with 10,000 lines of output. The actual error is buried somewhere.

**Approach:** Pipe the build log through ctrlb-decompose to find the failure patterns.

```bash
# GitHub Actions — download and analyze
gh run view 12345 --log | ctrlb-decompose --top 5 --context 2

# Local build output
cargo build 2>&1 | ctrlb-decompose --top 5 --context 2

# Test output
pytest -v 2>&1 | ctrlb-decompose --top 10 --context 3
```

**What to look for:**
- ERROR patterns are surfaced first
- Repeated test failure patterns cluster together
- Build warnings that might be related to the failure

---

## Monitoring / Alerting

**Problem:** You want to programmatically check if error rates exceed a threshold.

**Approach:** Use `--json` output and parse it.

```bash
# Check if any error pattern exceeds 5% frequency
ctrlb-decompose --json --quiet /var/log/app.log | \
  python3 -c "
import json, sys
data = json.load(sys.stdin)
for p in data['patterns']:
    if p['severity'] == 'error' and p['frequency_pct'] > 5.0:
        print(f\"ALERT: {p['template']} at {p['frequency_pct']}%\")
        sys.exit(1)
"
```

```bash
# In a shell script
ERROR_PCT=$(ctrlb-decompose --json --quiet app.log | \
  jq '[.patterns[] | select(.severity == "error")] | map(.frequency_pct) | add // 0')

if (( $(echo "$ERROR_PCT > 10" | bc -l) )); then
    echo "Error rate too high: ${ERROR_PCT}%"
    exit 1
fi
```

---

## Log Comparison

**Problem:** You deployed a new version and want to see if new error patterns appeared.

**Approach:** Run ctrlb-decompose on before and after logs, compare the templates.

```bash
# Before deploy
ctrlb-decompose --json --quiet before-deploy.log > /tmp/before.json

# After deploy
ctrlb-decompose --json --quiet after-deploy.log > /tmp/after.json

# Compare templates
diff <(jq -r '.patterns[].template' /tmp/before.json | sort) \
     <(jq -r '.patterns[].template' /tmp/after.json | sort)
```

New templates in the "after" output are new log patterns introduced by the deployment. Disappeared templates might indicate removed functionality or fixed bugs.
