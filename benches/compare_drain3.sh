#!/usr/bin/env bash
#
# compare_drain3.sh - Benchmark ctrlb-decompose (Rust) against Drain3 (Python)
#
# Generates log files at several scales, runs both tools, and prints a
# markdown comparison table with wall-clock time, peak RSS, and throughput.
#
# Requirements: cargo, python3, drain3 (pip), /usr/bin/time (GNU), bc
#
set -euo pipefail

# ── Configuration ────────────────────────────────────────────────────────────

SCALES=(1000 10000 100000 1000000)
SEED=42
BENCH_DIR="/tmp/ctrlb-bench"
TIME_BIN="/usr/bin/time"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

CTRLB_BIN="$PROJECT_DIR/target/release/ctrlb-decompose"
GENLOG_BIN="$PROJECT_DIR/target/release/examples/generate_logs"
DRAIN3_SCRIPT="$PROJECT_DIR/benches/drain3_baseline.py"

# ── Dependency checks ────────────────────────────────────────────────────────

check_deps() {
    local missing=()

    if ! command -v cargo &>/dev/null; then
        missing+=("cargo")
    fi

    if ! command -v python3 &>/dev/null; then
        missing+=("python3")
    fi

    if ! command -v bc &>/dev/null; then
        missing+=("bc")
    fi

    if [[ ! -x "$TIME_BIN" ]]; then
        missing+=("/usr/bin/time (GNU time)")
    fi

    if python3 -c "import drain3" 2>/dev/null; then
        :
    else
        missing+=("drain3 (pip install drain3)")
    fi

    if (( ${#missing[@]} > 0 )); then
        echo "ERROR: missing dependencies:" >&2
        for dep in "${missing[@]}"; do
            echo "  - $dep" >&2
        done
        exit 1
    fi
}

# ── Build ─────────────────────────────────────────────────────────────────────

build() {
    echo "==> Building ctrlb-decompose and log generator..."
    (cd "$PROJECT_DIR" && cargo build --release --example generate_logs 2>&1)
    (cd "$PROJECT_DIR" && cargo build --release 2>&1)

    if [[ ! -x "$CTRLB_BIN" ]]; then
        echo "ERROR: $CTRLB_BIN not found after build" >&2
        exit 1
    fi
    if [[ ! -x "$GENLOG_BIN" ]]; then
        echo "ERROR: $GENLOG_BIN not found after build" >&2
        exit 1
    fi
}

# ── Log generation ────────────────────────────────────────────────────────────

generate_logs() {
    mkdir -p "$BENCH_DIR"
    for n in "${SCALES[@]}"; do
        local logfile="$BENCH_DIR/bench-${n}.log"
        if [[ -f "$logfile" ]]; then
            echo "==> Reusing existing $logfile"
        else
            echo "==> Generating $logfile ($n lines)..."
            "$GENLOG_BIN" --lines "$n" --seed "$SEED" > "$logfile"
        fi
    done
}

# ── Time parsing helpers ──────────────────────────────────────────────────────

# Parse GNU time "Elapsed (wall clock)" value to seconds.
# Formats: "M:SS.ss" or "H:MM:SS.ss" or "H:MM:SS"
parse_wall_clock() {
    local raw="$1"
    # Strip any leading/trailing whitespace
    raw="$(echo "$raw" | xargs)"

    local parts
    IFS=':' read -ra parts <<< "$raw"

    if (( ${#parts[@]} == 2 )); then
        # M:SS.ss
        local minutes="${parts[0]}"
        local seconds="${parts[1]}"
        echo "$(echo "$minutes * 60 + $seconds" | bc -l)"
    elif (( ${#parts[@]} == 3 )); then
        # H:MM:SS.ss
        local hours="${parts[0]}"
        local minutes="${parts[1]}"
        local seconds="${parts[2]}"
        echo "$(echo "$hours * 3600 + $minutes * 60 + $seconds" | bc -l)"
    else
        echo "0"
    fi
}

# Extract wall clock time string from GNU time -v output file
extract_wall_clock() {
    local timefile="$1"
    grep "Elapsed (wall clock)" "$timefile" | sed 's/.*): //'
}

# Extract max RSS in KB from GNU time -v output file
extract_rss_kb() {
    local timefile="$1"
    grep "Maximum resident set size" "$timefile" | awk '{print $NF}'
}

# ── Benchmark runner ──────────────────────────────────────────────────────────

# Run a single benchmark, capture time metrics.
# Usage: run_bench <label> <tmpfile> <command...>
# Sets global vars: BENCH_WALL_SECS, BENCH_RSS_KB
run_bench() {
    local label="$1"
    local tmpfile="$2"
    shift 2

    echo "    Running $label..."
    "$TIME_BIN" -v "$@" > /dev/null 2> "$tmpfile"

    local wall_raw
    wall_raw="$(extract_wall_clock "$tmpfile")"
    BENCH_WALL_SECS="$(parse_wall_clock "$wall_raw")"
    BENCH_RSS_KB="$(extract_rss_kb "$tmpfile")"
}

# ── Main ──────────────────────────────────────────────────────────────────────

main() {
    check_deps
    build
    generate_logs

    # Result arrays (indexed by position in SCALES)
    declare -a ctrlb_times ctrlb_rss drain3_times drain3_rss

    local tmpfile
    tmpfile="$(mktemp)"
    trap "rm -f '$tmpfile'" EXIT

    for i in "${!SCALES[@]}"; do
        local n="${SCALES[$i]}"
        local logfile="$BENCH_DIR/bench-${n}.log"
        echo "==> Benchmarking $n lines..."

        # ctrlb-decompose
        run_bench "ctrlb-decompose" "$tmpfile" \
            "$CTRLB_BIN" --json --quiet --no-banner "$logfile"
        ctrlb_times[$i]="$BENCH_WALL_SECS"
        ctrlb_rss[$i]="$BENCH_RSS_KB"

        # Drain3
        run_bench "drain3 (python)" "$tmpfile" \
            bash -c "python3 '$DRAIN3_SCRIPT' < '$logfile'"
        drain3_times[$i]="$BENCH_WALL_SECS"
        drain3_rss[$i]="$BENCH_RSS_KB"
    done

    # ── Print results ────────────────────────────────────────────────────────

    echo ""
    echo "## ctrlb-decompose vs Drain3 (Python) Benchmark"
    echo ""
    echo "Seed: $SEED | Scales: ${SCALES[*]}"
    echo ""

    # Table header
    printf "| %-10s | %-12s | %-12s | %-8s | %-12s | %-12s | %-14s | %-14s |\n" \
        "Lines" "ctrlb (s)" "drain3 (s)" "Speedup" "ctrlb RSS" "drain3 RSS" "ctrlb lines/s" "drain3 lines/s"
    printf "|%s|%s|%s|%s|%s|%s|%s|%s|\n" \
        "------------" "--------------" "--------------" "----------" "--------------" "--------------" "----------------" "----------------"

    for i in "${!SCALES[@]}"; do
        local n="${SCALES[$i]}"
        local ct="${ctrlb_times[$i]}"
        local dt="${drain3_times[$i]}"
        local cr="${ctrlb_rss[$i]}"
        local dr="${drain3_rss[$i]}"

        # Speedup: drain3_time / ctrlb_time
        local speedup
        if [[ "$(echo "$ct > 0" | bc -l)" == "1" ]]; then
            speedup="$(echo "scale=1; $dt / $ct" | bc -l)"
        else
            speedup="N/A"
        fi

        # RSS in MB (from KB)
        local cr_mb dr_mb
        cr_mb="$(echo "scale=1; $cr / 1024" | bc -l)"
        dr_mb="$(echo "scale=1; $dr / 1024" | bc -l)"

        # Throughput: lines/s
        local ct_lps dt_lps
        if [[ "$(echo "$ct > 0" | bc -l)" == "1" ]]; then
            ct_lps="$(echo "scale=0; $n / $ct" | bc -l)"
        else
            ct_lps="N/A"
        fi
        if [[ "$(echo "$dt > 0" | bc -l)" == "1" ]]; then
            dt_lps="$(echo "scale=0; $n / $dt" | bc -l)"
        else
            dt_lps="N/A"
        fi

        printf "| %-10s | %-12s | %-12s | %-8s | %-12s | %-12s | %-14s | %-14s |\n" \
            "$n" \
            "$(printf '%.3f' "$ct")" \
            "$(printf '%.3f' "$dt")" \
            "${speedup}x" \
            "${cr_mb} MB" \
            "${dr_mb} MB" \
            "$ct_lps" \
            "$dt_lps"
    done

    echo ""
    echo "Done. Log files preserved in $BENCH_DIR"
}

main "$@"
