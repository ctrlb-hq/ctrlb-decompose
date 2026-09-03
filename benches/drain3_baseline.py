#!/usr/bin/env python3
"""Minimal Drain3 baseline for benchmarking against ctrlb-decompose."""

import sys
import time
from drain3 import TemplateMiner
from drain3.template_miner_config import TemplateMinerConfig

def main():
    config = TemplateMinerConfig()
    config.drain_sim_th = 0.4
    config.drain_depth = 4
    config.drain_max_clusters = 10000
    config.profiling_enabled = False

    miner = TemplateMiner(config=config)

    line_count = 0
    start = time.monotonic()

    for line in sys.stdin:
        line = line.rstrip('\n')
        if line:
            miner.add_log_message(line)
            line_count += 1

    elapsed = time.monotonic() - start
    clusters = miner.drain.clusters
    pattern_count = len(clusters)

    throughput = line_count / elapsed if elapsed > 0 else 0
    print(f"drain3_lines={line_count}", file=sys.stderr)
    print(f"drain3_patterns={pattern_count}", file=sys.stderr)
    print(f"drain3_elapsed={elapsed:.4f}", file=sys.stderr)
    print(f"drain3_throughput={throughput:.0f}", file=sys.stderr)

if __name__ == "__main__":
    main()
