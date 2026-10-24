#!/usr/bin/env python3
"""Summarize paired GPU experiments; Python standard library only."""

import argparse
import collections
import json
import random
import statistics
from pathlib import Path


def analyze(path):
    records = [json.loads(line) for line in path.read_text().splitlines() if line]
    if "loads" in records[0]:
        return analyze_pipeline(path, records)
    groups = collections.defaultdict(list)
    for row in records:
        if row["kind"] == "sample":
            groups[row["batch"], row["round"]].append(row)
    values = collections.defaultdict(list)
    excluded = []
    for (batch, round_id), rows in sorted(groups.items()):
        anchors = {r["variant"]: r for r in rows if r["variant"].startswith("production_")}
        if len(anchors) != 2:
            raise ValueError(f"Incomplete round: {path}, {batch}, {round_id}")
        before, after = anchors["production_before"], anchors["production_after"]
        drift = abs(after["mh_s"] / before["mh_s"] - 1)
        if drift > 0.05 or any(max(r["thermal_before"], r["thermal_after"]) >= 2 for r in rows):
            excluded.append({"batch": batch, "round": round_id, "anchor_drift_pct": drift * 100})
            continue
        mid = lambda r: r["started_unix"] + r["seconds"] / 2
        for row in rows:
            if row["variant"].startswith("production_"):
                continue
            fraction = (mid(row) - mid(before)) / (mid(after) - mid(before))
            baseline = before["mh_s"] + fraction * (after["mh_s"] - before["mh_s"])
            values[batch, row["variant"]].append((row, baseline))
    summary = []
    for (batch, variant), pairs in sorted(values.items()):
        rates = [r["mh_s"] for r, _ in pairs]
        gains = [(r["mh_s"] / baseline - 1) * 100 for r, baseline in pairs]
        rng = random.Random(42)
        bootstrap = sorted(statistics.mean(rng.choices(gains, k=len(gains))) for _ in range(10000))
        entry = {
            "batch": batch,
            "variant": variant,
            "n": len(pairs),
            "median_mh_s": statistics.median(rates),
            "min_mh_s": min(rates),
            "max_mh_s": max(rates),
            "median_baseline_mh_s": statistics.median(b for _, b in pairs),
            "mean_gain_pct": statistics.mean(gains),
            "bootstrap_95_pct": [bootstrap[249], bootstrap[9749]],
        }
        for metric in ("gpu_command_ms", "gpu_gap_ms", "cpu_wait_ms", "latency_ms", "drain_ms"):
            measured = [r[metric] for r, _ in pairs if r.get(metric) is not None]
            entry[metric] = statistics.median(measured) if measured else None
        summary.append(entry)
    return {"source": str(path), "metadata": records[0], "excluded_rounds": excluded, "results": summary}


def analyze_pipeline(path, records):
    groups = collections.defaultdict(dict)
    values = collections.defaultdict(list)
    excluded = []
    for row in records[1:]:
        groups[row["batch"], row["load"], row["round"]][row["variant"]] = row
    for (batch, load, round_id), rows in sorted(groups.items()):
        before, measured, after = (rows[v] for v in ("before", "pipeline", "after"))
        drift = abs(after["mh_s"] / before["mh_s"] - 1)
        if drift > 0.05 or any(max(r["thermal_before"], r["thermal_after"]) >= 2 for r in rows.values()):
            excluded.append({"batch": batch, "load": load, "round": round_id, "anchor_drift_pct": drift * 100})
            continue
        values[batch, load].append((measured["mh_s"], (before["mh_s"] + after["mh_s"]) / 2))
    summary = []
    for (batch, load), pairs in sorted(values.items()):
        gains = [(rate / baseline - 1) * 100 for rate, baseline in pairs]
        rng = random.Random(42)
        bootstrap = sorted(statistics.mean(rng.choices(gains, k=len(gains))) for _ in range(10000))
        summary.append({"batch": batch, "load": load, "variant": f"pipeline @ {load}%", "n": len(pairs),
                        "median_mh_s": statistics.median(r for r, _ in pairs),
                        "min_mh_s": min(r for r, _ in pairs), "max_mh_s": max(r for r, _ in pairs),
                        "median_baseline_mh_s": statistics.median(b for _, b in pairs),
                        "mean_gain_pct": statistics.mean(gains),
                        "bootstrap_95_pct": [bootstrap[249], bootstrap[9749]] if len(pairs) > 1 else None})
    return {"source": str(path), "metadata": records[0], "excluded_rounds": excluded, "results": summary}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="+", type=Path)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    reports = [analyze(path) for path in args.files]
    if args.json:
        args.json.write_text(json.dumps(reports, indent=2) + "\n")
    for report in reports:
        print(f"\n{report['source']}")
        print("batch | variant | n | baseline MH/s | median MH/s | paired gain | bootstrap 95%")
        for r in report["results"]:
            interval = r["bootstrap_95_pct"]
            confidence = f"[{interval[0]:+.2f}, {interval[1]:+.2f}]%" if interval else "insufficient samples"
            print(f"{r['batch']} | {r['variant']} | {r['n']} | {r['median_baseline_mh_s']:.2f} | "
                  f"{r['median_mh_s']:.2f} | {r['mean_gain_pct']:+.2f}% | {confidence}")
        print(f"Excluded rounds: {report['excluded_rounds']}")


if __name__ == "__main__":
    main()
