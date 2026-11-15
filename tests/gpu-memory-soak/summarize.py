#!/usr/bin/env python3
"""Summarises soak series (crates/gnoblin/tests/gpu_memory_soak.rs): python3 summarize.py software.jsonl gles.jsonl ...

Per series: the run's length, the resident set at the start, the peak and the end, its growth in MiB a minute over the
second half of the run (a least-squares slope: a leak shows as a slope that stays, a warm-up as one that falls away),
the range of descriptors and threads, and the largest count of bytes the compositor says it holds.
"""

import json
import sys


def slope_per_minute(points):
    """Least-squares slope of (seconds, bytes) points, in MiB a minute."""
    n = len(points)
    if n < 3:
        return 0.0
    mean_t = sum(t for t, _ in points) / n
    mean_v = sum(v for _, v in points) / n
    num = sum((t - mean_t) * (v - mean_v) for t, v in points)
    den = sum((t - mean_t) ** 2 for t, _ in points)
    return 0.0 if den == 0 else num / den * 60 / (1 << 20)


def mib(value):
    """Bytes as whole MiB."""
    return f"{value / (1 << 20):.0f}"


def summarise(path):
    rows = [json.loads(line) for line in open(path) if line.startswith("{")]
    if not rows:
        return f"{path}: no samples"
    end = rows[-1]["t"]
    late = [(r["t"], r["rss"]) for r in rows if r["t"] >= end / 2]
    return (
        f"{path}: {len(rows)} samples over {end:.0f} s, {rows[-1]['cycle']} cycles, "
        f"{rows[-1]['lock_cycles']} lock cycles, {rows[-1]['capture_frames']} capture frames "
        f"({rows[-1]['capture_frames_failed']} refused, {rows[-1]['capture_sessions']} sessions), "
        f"{rows[-1]['screenshots']} screenshots; "
        f"rss {mib(rows[0]['rss'])} -> peak {mib(max(r['rss'] for r in rows))} -> end {mib(rows[-1]['rss'])} MiB, "
        f"second-half slope {slope_per_minute(late):+.2f} MiB/min; "
        f"fds {min(r['fds'] for r in rows)}..{max(r['fds'] for r in rows)}, "
        f"threads {min(r['threads'] for r in rows)}..{max(r['threads'] for r in rows)}; "
        f"gpu_memory_bytes max {mib(max(r['gpu_memory_bytes'] for r in rows))} MiB, "
        f"end {mib(rows[-1]['gpu_memory_bytes'])} MiB"
    )


if __name__ == "__main__":
    for series in sys.argv[1:]:
        print(summarise(series))
