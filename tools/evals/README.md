# Evaluation evidence tools

These scripts support the [2026-09-22 internal evaluations](../../docs/evaluations/README.md). They are evidence helpers, not a one-command acceptance suite. Some preparation and grading depend on ignored private freeze ledgers, original prompts, attachment files and disposable runtime state. Public fixtures and metrics allow aggregate review; a new clean evaluation needs new frozen inputs and its own explicit accounts, scope and budget.

The measured candidate and client hashes belong to the reports. Several helper scripts were subsequently parameterized for archival. This does not retroactively change the measured binary, original prompts, frozen inputs or observed outputs.

## Available helpers

| Tool | Purpose |
| --- | --- |
| `benchmark_codex_fresh_threads.py` | Repeat the same prepared fixtures in fresh threads with timing phases and input usage. The typed path uses the installed MCP connection without a model turn. |
| `benchmark_standalone_relay.py` | Compare a local timer with the native scheduler and exercise an offline-worker relay retry on loopback. |
| `run_codex_case.py` | One bounded, actual Codex session. Explicit binary/project/prompt/output arguments; optional explicitly scoped CookBook MCP. Captures raw events, event timings, client usage and stop reason. |
| `e01_export.py` | Read-only export of six E01 sessions and verification of three saved-note artifacts from disposable state. |
| `e02_scoped.py`, `e02_generate_prompts.py`, `e02_grade.py` | Fixed-attachment helper, frozen session prompts and exact independent-oracle grading. The grader requires the explicit original fixture root and private freezes. |
| `prepare_e03.py`, `prepare_e03_amendment.py`, `prepare_e04.py` | Prepare synthetic inputs and manifests without granting access or making provider calls. Adapter input directories must be explicitly supplied; the amendment requires an ignored directory and refuses to overwrite its input files. |
| `provider_runtime.py`, `run_e03_amended.py` | Scoped provider execution helpers. Require explicit binary, state and attachment; a provider runner makes live calls and must be used only under a separately authorized protocol/budget. Uncertain attempted cells are not replacement opportunities. |
| `record_provider_results.py` | Export the observed frozen E03/E04 snapshot. Its unresolved statuses are specific to this recorded run, not a live health check. Public export omits labels, predictions and run identifiers. |
| `e05_improvement.py`, `e05_export.py` | Prepare the pure diagnostic workflow and export its recorded diagnostics. The original run remains blocked and must not be relabeled as reviewed; the fresh reviewed follow-up has separate cases, hashes and receipts. |
| `plot_results.py` | Render E02 quality, time and visible client context from sanitized JSON using Matplotlib 3.10.6. No network/provider access. |
| `plot_token_efficiency.py` | Plot all twelve exact prepared-reuse holdouts across matched Luna/Sol direct-script and CookBook arms. Separates cached/new input, output and elapsed time; makes no provider or model call. |
| `measure_batch_workers.py` | Compare two explicit APR binaries using complete synthetic pure/native batches in new private state. Grants only a fixture Python interpreter; freezes inputs and binary identities, excludes setup from reuse timing, retains full traces and measures worker overlap, caller calls and message bytes. No provider/model calls or token estimates. |
| `measure_continuous_pool.py` | Exercise an explicit candidate at owner limits 4/32/64 in three new private states, with 64 exact native outputs per arm, worker intervals, live status, and heterogeneous synthetic waits. This proves overlap and refill, not model-token savings or real provider throughput. |
| `plot_batch_workers.py` | Render retained batch timing/call metrics with observed min–max ranges and a data/source provenance file. No runtime or API calls. |
| `plot_worker_capacity.py` | Plot the corrected 4/32/64-worker fixture from exact outputs and native intervals, with retained failed-pilot disclosure. |
| `plot_readme_candidate.py` | Plot the assisted E02 prepared comparison and frozen E04 routing results. Requires the selected candidate metrics; makes no runtime or model call. |

Keep raw MCP traces, attachment files and runtime databases in ignored private storage. Public exports use selected fields and preserve missing usage/cost as null. The caller batch appears on multiple E04 case rows; deduplicate by `batch_id` when totaling its usage.

The [reviewed E05 follow-up](../../docs/evaluations/2026-09-22-e05-reviewed-improvement.md)
and [batch capability check](../../docs/evaluations/2026-09-22-batch-execution.md)
retain separate private one-shot helpers and public selected metrics. The E05
helper reused only this archive's CLI/MCP transport; it did not rerun or rewrite
the original study. A read-only correction finished its CLI report check after
an argument error. Public batch manifests and input/output examples are linked
from the batch report; no provider credentials are needed for those pure fixtures.

## Recreate the chart

With Matplotlib 3.10.6 in a task-local Python environment, run from the repository root:

```sh
python tools/evals/plot_results.py \
  --data docs/evaluations/fixtures/e02/case-metrics.json \
  --setup docs/evaluations/fixtures/e02/setup-metrics.json \
  --output work/evals-2026-09-22/charts
```

The output includes PNG, editable SVG and a source-hash manifest. The chart was visually checked at full resolution. It includes all nine attempts and separates cached input from uncached input; neither is converted into subscription dollars. The chart is a local review artifact, not a published performance claim. Cold authoring is shown separately and does not include a timed estimate for the additional manual manifest repair/grant.

## Boundaries found by this run

Exact connection-proof bytes were easy for the client to write incorrectly; invalid manifest errors did not give it enough guidance to recover. A stale adapter lock passed static readiness but could not launch. The initial Gemini envelope and adapter-relative input paths also needed correction before a valid provider request. Jev's unresolved dispatch coincided with a possible protected credential prompt and later author-write timeouts; the cause was not proven. Resolve these findings and freeze a new protocol before claiming acceptance. Do not erase this run's failures with replacement cases.

## Prepared token-efficiency evidence

The project-aware comparison exports its frozen synthetic source pages, complete
expected JSON values, case metrics and study protocol under
`docs/evaluations/fixtures/e02-token-efficiency/`. Raw prompts, configuration
bookkeeping, model events, source access and the preserved aborted setup remain
private. The report records the infrastructure amendment and all setup costs.
This is a deterministic performance comparison, not E05 human-reviewed truth.

Recreate the chart from reviewed metrics (Matplotlib 3.11.2 used):

```sh
python tools/evals/plot_token_efficiency.py \
  --data docs/evaluations/fixtures/e02-token-efficiency/case-metrics.csv \
  --output work/prepared-token-efficiency.png
```

The helper requires all twelve attempted cases, displays every arm's quality
count, and plots only whole matched pairs that passed full-output/source-coverage
checks. Failed error paths remain in the CSV and are never shown as savings. New model runs require a new explicit
protocol; this helper only reads the published metrics.

## Prepared parallel-batch evidence

The [packaged comparison](../../docs/evaluations/2026-09-22-parallel-batches.md)
retains its frozen protocol, all 18 case rows, setup times, package identity and
chart provenance. Recreate its chart with Matplotlib 3.10.6:

```sh
python tools/evals/plot_batch_workers.py \
  --data docs/evaluations/fixtures/parallel-batches/metrics.json \
  --output work/parallel-batch-charts
```

For a new runtime comparison, `measure_batch_workers.py` requires explicit
`--baseline`, `--candidate` and a new private `--output` directory. It creates a
pinned Python grant only in disposable state. Retain the full output directory;
publish only reviewed synthetic exports. The helper does not use the normal
installation's store, client settings or real accounts.

## Configurable workers and README candidate

The [capacity report](../../docs/evaluations/2026-09-22-worker-capacity.md) retains
the corrected fixture and two failed pilots. The
[README candidate report](../../docs/evaluations/2026-09-22-readme-candidate.md)
retains actual client/provider attempts, assisted setup, failed gates, uncertain
effects and reviewed E05 regression provenance. These are separate measurements;
worker overlap is not a model-token result.

Recreate the charts using Matplotlib 3.11.2:

```sh
python tools/evals/plot_worker_capacity.py \
  --data docs/evaluations/fixtures/worker-capacity/metrics.json \
  --intervals docs/evaluations/fixtures/worker-capacity/intervals.json \
  --output work/worker-capacity-charts

python tools/evals/plot_readme_candidate.py \
  --data-dir docs/evaluations/fixtures/readme-candidate \
  --output work/readme-candidate-charts
```

The selected public task/source/expected data support aggregate review. Private
one-shot runners depend on frozen local attachments, exact binaries and original
model transcripts; the exports do not pretend to be a credential-free rerun of
live studies. A new provider run needs its own protocol, account authorization
and budget; an uncertain cell must not silently become a replacement call.
