#!/usr/bin/env python3
"""Plot retained native concurrency measurements; no runtime or provider calls."""

import argparse
import hashlib
import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--intervals", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    rows = json.loads(args.data.read_text())
    intervals = json.loads(args.intervals.read_text())
    assert [row["policy"] for row in rows] == [4, 32, 64]
    assert all(row["quality_pass"] and row["exact_outputs"] == 64 for row in rows)
    colors = ["#526579", "#BF603D", "#43796B"]
    fig, axes = plt.subplots(1, 2, figsize=(11.5, 4.8), layout="constrained")
    bars = axes[0].bar([str(row["policy"]) for row in rows],
                       [row["wall_ms"] / 1000 for row in rows], color=colors)
    axes[0].bar_label(bars, fmt="%.2f s", padding=5)
    axes[0].set(xlabel="Owner-selected worker limit", ylabel="64-job batch time (seconds)",
                ylim=(0, 15))
    for row, color in zip(rows, colors):
        samples = [v for v in intervals if v["policy"] == row["policy"]]
        assert len(samples) == 64
        events = sorted([(v["start_ms"], 1) for v in samples]
                        + [(v["end_ms"], -1) for v in samples])
        times, counts, active = [], [], 0
        for timestamp, delta in events:
            active += delta
            times.append(timestamp / 1000)
            counts.append(active)
        assert active == 0 and max(counts) == row["interval_peak"]
        axes[1].step(times, counts, where="post", color=color,
                     label=f"{row['policy']} workers; 64/64 exact")
    axes[1].set(xlabel="Seconds since first worker started", ylabel="Observed active workers",
                ylim=(0, 70))
    axes[1].legend(frameon=False, fontsize=8)
    for axis in axes:
        axis.spines[["top", "right"]].set_visible(False)
        axis.grid(axis="y", alpha=0.18)
        axis.set_axisbelow(True)
    fig.suptitle("64 native jobs: measured concurrency and complete outputs", fontsize=14)
    fig.supxlabel("One corrected run per limit; setup excluded. Synthetic waits: 0.55–0.79 s, one 2.4 s job.\nTwo failed pilots retained in the report. No model calls, token comparison or real API throughput claim.", fontsize=9)
    args.output.mkdir(parents=True, exist_ok=True)
    for suffix in ("png", "svg"):
        fig.savefig(args.output / f"worker-capacity.{suffix}", dpi=170)
        if suffix == "svg":
            svg = args.output / "worker-capacity.svg"
            svg.write_text("\n".join(line.rstrip() for line in svg.read_text().splitlines()) + "\n")
    plt.close(fig)
    (args.output / "chart-provenance.json").write_text(json.dumps({
        "data_sha256": hashlib.sha256(args.data.read_bytes()).hexdigest(),
        "intervals_sha256": hashlib.sha256(args.intervals.read_bytes()).hexdigest(),
        "plotter_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "matplotlib_version": matplotlib.__version__,
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
