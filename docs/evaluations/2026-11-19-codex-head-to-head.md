# Codex alone vs. Codex with CookBook

2026-11-19 · macOS ARM64 · codex-cli 0.169.1 · GPT-6 Sol, Low

## Matched prepared runs

This is a direct comparison in actual Codex model turns. The Codex-alone arm
had the equivalent prepared Python script and its normal terminal tools. The CookBook
arm had the registered recipe and used the installed, development-signed APR
executable in disposable state. Both arms used identical synthetic inputs and
the same chat output contract. Each workflow had its own thread in each arm.
A read of Codex's tool inventory verified zero MCP servers with tools in the
Codex-alone threads and only APR in the CookBook threads. A preliminary pass
was excluded after its Codex-alone session inherited the APR tool catalog.
Four inputs per workflow were paired by exact content. Arm order alternated by
input and workflow. The benchmark used no live workflow data or provider
connections inside the recipes.

The profile cases requested the complete ordered list for 4, 8, 12, and 16
addresses. The inventory and SLA cases used 72, 96, 144, and 300 rows each.
Those cases required the same three counts in chat and the same complete saved
report. The profile workers used the same fixture lookups with simulated I/O
waits and four workers. User-level set-up, code authoring, recipe registration,
and input staging precede the timed turns. CSV import was part of staging.
Cold first-use probes below answer a separate question.

| Workflow | Codex median | + CookBook median | Codex input tokens | + CookBook input tokens | Codex cost estimate | + CookBook cost estimate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Profile lookup | 10.74 s | 12.81 s | 171,995 | 176,930 | $0.09055 | $0.07721 |
| Inventory reconciliation | 3.86 s | 7.51 s | 167,096 | 175,726 | $0.05699 | $0.05358 |
| Ticket SLA checks | 3.62 s | 6.86 s | 167,203 | 179,847 | $0.04026 | $0.06870 |

Input and estimated cost totals cover four turns per arm in each row. Across all
12 matched turns per arm, Codex alone took 73.49 s and used
506,294 input tokens (34,742 uncached). Codex + CookBook took
106.93 s and used 532,503 input tokens (28,567
uncached). CookBook was faster in 0/12 pairs and lower in estimated cost in
2/12. Total API-equivalent cost was $0.18780 for Codex alone
and $0.19948 with CookBook. Billing estimates apply Standard GPT-6
Sol rates to observed cached, uncached, cache-write and output tokens. This run
used ChatGPT-authenticated Codex and observed no subscription invoice.
[Official model pricing](https://developers.openai.com/api/docs/models/gpt-6-sol)

## Output quality

Codex alone produced 12/12 exact tool outputs and 11/12 exact chat replies. Codex + CookBook produced 12/12 exact tool outputs and 11/12 exact chat replies.

The grader checks saved report bytes against a separate fixture oracle and
audits scoped CookBook report exports. Chat quality is scored separately.
Profile lists are compared in order.

## Codex first use from raw files

These two supplemental 144-row turns gave Codex only each synthetic CSV and the
task rules. It could write and run its own local code. These are first-use
observations separate from the prepared comparison.

| Workflow | Codex wall time | Input tokens | Tool calls | Saved report | Chat answer |
| --- | ---: | ---: | ---: | --- | --- |
| Inventory reconciliation | 8.56 s | 41,219 | 1 | Exact | Exact |
| Ticket SLA checks | 10.77 s | 41,318 | 1 | Exact | Exact |

Four matched inputs per workflow make this a small reproducible pilot. The
comparison covers deterministic, prepared workloads under the observed client
and cache conditions. Each reported turn includes the model's tool selection
and final reply.

## Reproduce the matched run

```sh
mkdir -p work
python3 tools/evals/benchmark_codex_head_to_head.py --output work/codex-head-to-head
```

Use an installed CookBook managed connection with the existing scoped run
approval and an active ChatGPT Codex sign-in. The harness creates its own
private synthetic state. It records all attempts and leaves the user
configuration intact. Pass a new output directory to the
[first-use harness](../../tools/evals/benchmark_codex_first_use.py) to repeat
the two raw-CSV probes. That probe compares parsed JSON report values.

[Benchmark harness](../../tools/evals/benchmark_codex_head_to_head.py) · [Sanitized full metrics](fixtures/codex-head-to-head-2026-11-19/metrics.json)
