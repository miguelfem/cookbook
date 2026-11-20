#!/usr/bin/env python3
"""Render recorded model-free batch measurements; makes no runtime/API calls."""

import argparse
import hashlib
import json
from pathlib import Path
import statistics

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    rows = json.loads(args.data.read_text())
    assert rows and all(row["quality_pass"] for row in rows)
    cases = ("composed-pure", "direct-native", "composed-native")
    names = ("Pure child recipes", "Native jobs", "Native child recipes")
    colors = {"baseline": "#526579", "candidate": "#BF603D"}
    fig, axes = plt.subplots(1, 2, figsize=(11.5, 4.8), layout="constrained")
    for axis, field, label in zip(axes, ("wall_ms", "caller_tool_calls"),
                                  ("Prepared batch time (seconds)", "Caller tool calls per batch")):
        for arm, offset, title in (("baseline", -0.18, "Previous installed build"),
                                   ("candidate", 0.18, "Bounded workers + composed wait")):
            samples = [[row[field] / (1000 if field == "wall_ms" else 1)
                        for row in rows if row["case"] == case and row["arm"] == arm]
                       for case in cases]
            means = [statistics.mean(values) for values in samples]
            errors = [[mean - min(values) for mean, values in zip(means, samples)],
                      [max(values) - mean for mean, values in zip(means, samples)]]
            bars = axis.bar([i + offset for i in range(3)], means, 0.34,
                            yerr=errors, capsize=3, color=colors[arm], label=title)
            axis.bar_label(bars, labels=[f"{v:.2f}" if field == "wall_ms" else f"{v:.1f}" for v in means], padding=5, fontsize=9)
        axis.set_xticks(range(3), names, fontsize=9)
        axis.set_ylabel(label)
        axis.set_ylim(0, axis.get_ylim()[1] * 1.18)
        axis.spines[["top", "right"]].set_visible(False)
        axis.grid(axis="y", alpha=0.18)
        axis.set_axisbelow(True)
    axes[0].legend(frameon=False, fontsize=8, loc="upper left")
    fig.suptitle("Prepared four-job batches: complete outputs, fewer caller exchanges", fontsize=14)
    fig.supxlabel("Means and min–max ranges across 3 paired repetitions. Native fixtures wait 350 ms per job.\nSetup excluded; incomplete replies polled every 100 ms. No LLM calls or measured token savings.", fontsize=9)
    args.output.mkdir(parents=True, exist_ok=True)
    for suffix in ("svg", "png"):
        fig.savefig(args.output / f"batch-workers.{suffix}", dpi=170)
    plt.close(fig)
    (args.output / "chart-provenance.json").write_text(json.dumps({
        "data_sha256": hashlib.sha256(args.data.read_bytes()).hexdigest(),
        "plotter_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "matplotlib_version": matplotlib.__version__,
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
