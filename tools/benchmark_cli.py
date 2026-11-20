#!/usr/bin/env python3
"""Small local M0 resource sample; this does not measure tokens or model costs."""
import argparse
import json
import platform
import resource
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apr", type=Path, required=True)
    parser.add_argument("--iterations", type=int, default=10)
    args = parser.parse_args()
    if not 1 <= args.iterations <= 100:
        parser.error("iterations must be between 1 and 100")
    binary = args.apr.resolve(strict=True)
    root = Path(__file__).resolve().parents[1]
    fixture = root / "examples/local-review"
    samples = {"version_ms": [], "validate_ms": [], "run_to_handoff_ms": []}
    with tempfile.TemporaryDirectory(prefix="apr-benchmark-") as temporary:
        imported = subprocess.run([str(binary), "--state-dir", temporary, "artifact", "import",
                                   str(fixture / "source.txt"), "--media-type", "text/plain"],
                                  capture_output=True, check=True)
        input_path = Path(temporary) / "input.json"
        input_path.write_text(json.dumps({"record": {"title": "Resource sample"},
                                          "sources": [json.loads(imported.stdout)]}))
        for iteration in range(args.iterations):
            commands = {
                "version_ms": ["--version"],
                "validate_ms": ["process", "validate", str(fixture / "process.json")],
                "run_to_handoff_ms": ["--state-dir", temporary, "run", str(fixture / "process.json"),
                                      "--input-file", str(input_path),
                                      "--request-key", f"sample-{iteration}"],
            }
            for name, command in commands.items():
                start = time.perf_counter()
                result = subprocess.run([str(binary), *command], capture_output=True, check=True)
                samples[name].append((time.perf_counter() - start) * 1000)
                if name == "run_to_handoff_ms":
                    assert json.loads(result.stdout)["state"] == "waiting_for_agent"
    rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    rss_bytes = rss if platform.system() == "Darwin" else rss * 1024
    print(json.dumps({
        "platform": platform.platform(), "iterations": args.iterations,
        "binary_bytes": binary.stat().st_size, "peak_child_rss_bytes": rss_bytes,
        "samples": {name: {"median": round(statistics.median(values), 3),
                           "min": round(min(values), 3), "max": round(max(values), 3)}
                    for name, values in samples.items()},
        "caveats": "Independent CLI processes with warm OS caches; RSS is the maximum across children. No idle daemon or token benchmark.",
    }, indent=2))


if __name__ == "__main__":
    main()
