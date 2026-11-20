# Research Triage fixture

This synthetic v0.2 process selects an exact declared route, skips the other branch, joins the selected output, and writes an immutable JSON report. One case runs independently; the other pauses for an explicit caller result. The supplied `cases.json` contains canned caller answers for reproducible runtime checks. They are not model judgments, live evidence, or measured quality.

Run all three domains from the repository root with:

```sh
python3 tools/benchmark_outcomes.py --apr /absolute/path/to/apr --iterations 3
```

The harness uses a disposable project, private state and an owned foreground service. It checks actual saved results and artifact bytes, and reports protocol byte counts and duration. Token and cost values remain unknown.
