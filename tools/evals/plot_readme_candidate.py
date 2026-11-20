#!/usr/bin/env python3
"""Render selected README evaluation results without runtime/model calls."""

import argparse
import hashlib
import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    e02 = json.loads((args.data_dir / "e02-aggregate.json").read_text())["arms"]
    providers = json.loads((args.data_dir / "providers.json").read_text())
    arms = [e02[key] for key in ("direct", "script", "cookbook")]
    assert all(row["exact"] == row["attempted"] == 3 for row in arms)
    labels = ["Direct Luna", "Reusable script", "CookBook"]
    colors = ["#526579", "#43796B", "#BF603D"]
    fig, axes = plt.subplots(1, 3, figsize=(13.5, 4.7), layout="constrained")
    bars = axes[0].bar(labels, [row["wall_seconds"] for row in arms], color=colors)
    axes[0].bar_label(bars, fmt="%.2f s", padding=4, fontsize=9)
    axes[0].set(title="E02 · all 3/3 outputs exact", ylabel="Three prepared turns (seconds)",
                ylim=(0, 100))
    cached = [row["cachedInputTokens"] / 1000 for row in arms]
    uncached = [row["noncachedInputTokens"] / 1000 for row in arms]
    axes[1].bar(labels, uncached, color=colors, label="Noncached input")
    bars = axes[1].bar(labels, cached, bottom=uncached, color="#D9DEE2", label="Cached input")
    axes[1].bar_label(bars, labels=[f"{(c + u):.1f}k" for c, u in zip(cached, uncached)],
                      padding=4, fontsize=9)
    axes[1].set(title="E02 · retained client context", ylabel="Reported input tokens (thousands)",
                ylim=(0, 590))
    axes[1].legend(frameon=False, fontsize=8, loc="upper left")
    e04 = providers["e04"]
    scores = [e04["rules_baseline"]["exact"], e04["caller_baseline"]["exact"],
              e04["routing"]["exact"]]
    bars = axes[2].bar(["Rules", "Caller Luna", "Jev + gate"], scores, color=colors)
    axes[2].bar_label(bars, labels=[f"{v}/24" for v in scores], padding=4, fontsize=9)
    axes[2].axhline(22, color="#8C4740", linestyle="--", linewidth=1)
    axes[2].set(title="E04 · frozen routing quality", ylabel="Exact routes after confidence gate",
                ylim=(0, 28))
    for axis in axes:
        axis.tick_params(axis="x", labelsize=8)
        axis.spines[["top", "right"]].set_visible(False)
        axis.grid(axis="y", alpha=0.18)
        axis.set_axisbelow(True)
    fig.suptitle("README candidate: exact execution, unresolved efficiency and routing gates", fontsize=14)
    fig.supxlabel("E02: assisted setup excluded from reuse; its original authoring failure remains recorded. No provider calls or dollar estimate.\nE04: authored synthetic cases, threshold 0.75; all controls preserved required reviews. Not a matched token/latency comparison.", fontsize=9)
    args.output.mkdir(parents=True, exist_ok=True)
    for suffix in ("png", "svg"):
        fig.savefig(args.output / f"readme-candidate.{suffix}", dpi=170)
        if suffix == "svg":
            svg = args.output / "readme-candidate.svg"
            svg.write_text("\n".join(line.rstrip() for line in svg.read_text().splitlines()) + "\n")
    plt.close(fig)
    (args.output / "chart-provenance.json").write_text(json.dumps({
        "e02_sha256": hashlib.sha256((args.data_dir / "e02-aggregate.json").read_bytes()).hexdigest(),
        "providers_sha256": hashlib.sha256((args.data_dir / "providers.json").read_bytes()).hexdigest(),
        "plotter_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "matplotlib_version": matplotlib.__version__,
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
