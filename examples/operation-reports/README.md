# Inventory and SLA reports

Two deterministic recipes turn synthetic inventory or ticket CSV files into
action counts and a full immutable report. The inputs are fictional and the
workers use only the Python standard library.

- **Inventory** flags overcounts and shortages and calculates the net change.
- **Ticket SLA** counts late first responses and unanswered tickets using three
  fixed thresholds.

Use the `*-brief.json` manifests for prepared execution. Import the CSV as a
scoped artifact, then call the registered recipe once with `{"source": <artifact reference>}`.
The reply contains the action counts and report reference. The full ordered report
is available for viewing or verified export. The comparison manifests also
accept `include_rows` so the evaluation can measure the same work with rows
returned inline.

## Reproduce the benchmark

From a checkout with Python 3.11 or later and the pinned Rust toolchain:

```sh
cargo build --locked -p apr-cli
mkdir -p work
python3 tools/evals/benchmark_operation_reports.py --apr target/debug/apr --output work/operation-report-check
```

The harness creates private state, generates its own data, registers the
recipes, and checks every output against an independent oracle. It compares
inline and report-only replies for 72, 96, and 144 rows, then checks both
brief interfaces with 300 rows. It exports the full reports separately to
verify every byte. The harness issues no model or provider requests. Use a new
output directory each time.

[Benchmarks from the installed build](../../docs/evaluations/2026-11-19-installed-operation-reports.md)
