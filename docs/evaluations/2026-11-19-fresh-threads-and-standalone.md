# Fresh Codex threads and standalone CookBook runs

2026-11-19 · macOS ARM64 · Codex CLI 0.169.1 · GPT-6 Sol, Low

CookBook took longer in all 12 matched Codex runs. Most of the time difference accumulated before the tool started. A typed call through the same installed MCP connection produced every exact output with zero model tokens. The native scheduler also ran independently of Codex.

## Fresh-thread comparison

Each of the 36 measured model turns ran in its own ephemeral Codex App Server thread. All three arms used the same synthetic inputs and expected outputs as the [previous matched study](2026-11-19-codex-head-to-head.md). The Codex-alone arm had an equivalent prepared Python command. The other two arms used the installed CookBook executable, with disposable state and six or two APR tools. The two-tool arm used a process-local allowlist for `apr_project` and `apr_run`. The harness verified each thread's tool inventory and rotated arm order. Recipe setup, grants, input staging, and connection proofs happened outside the timed turns. Fresh threads remove conversation carryover. They can still share model prefix caches and host load.

| Workflow | Codex alone | CookBook, six tools | CookBook, two tools | Typed MCP |
| --- | ---: | ---: | ---: | ---: |
| Profile lookup, 4–16 addresses | 10.25 s | 12.55 s | 12.47 s | 0.889 s |
| Inventory reconciliation, 72–300 rows | 4.93 s | 8.52 s | 8.73 s | 0.121 s |
| Ticket SLA checks, 72–300 rows | 5.31 s | 8.05 s | 9.22 s | 0.104 s |
| Total for 12 cases | 83.61 s | 117.45 s | 122.59 s | 4.55 s |
| Total input tokens | 495,449 | 545,434 | 589,520 | 0 |
| Exact tool outputs | 12/12 | 12/12 | 12/12 | 12/12 |
| Exact chat replies | 12/12 | 12/12 | 12/12 | No chat turn |

Times per workflow are medians of four inputs. Model-turn time includes selecting the tool and writing the final reply. The typed column uses a freshly connected thread for each input and times one `mcpServer/tool/call` to `apr_run` after setup. The typed caller has the recipe and input already. The profile recipe returns an ordered list. The inventory and SLA recipes return three counts and an artifact reference. The grader verified the full report bytes separately. Timing the typed call excludes this verification and report export.

## Where the extra time and tokens went

| Client-observed phase, across 12 turns | Codex alone | CookBook, six tools | Difference |
| --- | ---: | ---: | ---: |
| Turn start to tool start | 45.65 s | 78.95 s | +33.30 s |
| Tool start to tool completion | 2.32 s | 5.06 s | +2.74 s |
| Tool completion to turn completion | 35.64 s | 33.44 s | −2.21 s |

These are client boundaries from Codex notifications. They cannot resolve cloud queue, network, and inference time within a phase. Each case executed exactly one visible workflow tool call. Ten six-tool turns produced two token-usage events, as did all 12 Codex-alone turns. The remaining two six-tool turns produced three events each. Their third events accounted for 43,588 of the 49,985 added input tokens (87%). In those turns, the first event already arrived several seconds before the tool started. Summing the first two usage events per turn yields a smaller 6,397-token difference. The experiment establishes where the client reported the extra work. It leaves the reason Codex made extra requests unresolved.

The allowlist reduced the app-reported serialized tool catalog from 23,454 bytes to 3,145 bytes. First-request input across 12 turns was nearly identical with either catalog (247,717 vs. 247,709 tokens). The two-tool arm recorded four three-request turns and higher total input and elapsed time. Catalog size in bytes is a discovery measurement. Codex determines what it places in each model request. The observed data do not support promoting the allowlist as a token or speed improvement.

At the published Standard GPT-6 Sol API rates, the observed usage comes to $0.22086 for Codex alone, $0.26493 for six-tool CookBook, and $0.21802 for two-tool CookBook. Higher cache reuse accounts for the lower two-tool estimate despite its larger input. These are API-equivalent estimates from actual input, cache and output counters. The study used an existing ChatGPT sign-in. [Standard pricing](https://developers.openai.com/api/docs/models/gpt-6-sol)

## Model-free schedule and relay

A local timer running the equivalent Python SLA command delivered four exact 300-row reports in a median **50 ms** from scheduled due time. CookBook's native one-time scheduler also delivered four exact reports with a median **314 ms** from due time to recorded success. Its median due-to-dispatch delay was **196 ms** and its median dispatch-to-success delay was **119 ms**. The timer and native scheduler used zero model tokens. The fixture alternated runner order, excluded registration and post-run verification, and measured the schedule on an online idle host. The native path provides durable scheduling, assignment authorization, run history and artifact custody. The scheduler's documented cadence is about 200 ms. [Native scheduler](../architecture/scheduled-jobs.md)

The loopback relay test submitted the same 300-row workload while the CookBook service was stopped. It received HTTP `202` in **3.80 ms** locally, replayed the submit with the same request key, and read the queued job after a relay restart. The worker yielded an exact result **169 ms** after restarting. A deliberate lost acknowledgement caused the queue lease to expire, then a retry with the same CookBook request key returned the original run, with **one** recorded native effect. The producer could read the final run ID and counts. The full artifact stayed local. The simulated offline interval was injected by the harness. These timings cover one outage drill and do not measure VPS or internet latency.

## Proposed deployment model

Use Codex to author, verify and revise recipes. Once the inputs and output format are known, a scheduler or typed caller runs the pinned recipe and displays its structured result. For inventory and SLA reports that means counts and a link produced by an authorized artifact reader. Ask a model to interpret anomalies or make a decision when the workflow calls for judgment.

Put an authenticated durable queue on the VPS. Commit a job under its producer-scoped idempotency key before responding with HTTP `202`. A Mac worker can poll outbound over HTTPS and lease only jobs for a locally authorized, pinned recipe and its bounded input contract. Use the same key in CookBook, renew leases for long work, preserve the distinction between an accepted/leased/waiting job and a terminal result, and publish only an authorized receipt. Browser, device and local-file work waits for the Mac to reconnect. A headless worker on an online host is required for any compatible job that must finish while the Mac is unavailable. Verify its runtime, grants, artifact access and queue recovery separately.

The fixture binds only to loopback and uses synthetic data, disposable state and ephemeral tokens. It exercises durable receipt, lease expiry and idempotent retry on one workload. VPS deployment and remote worker acceptance remain proposed work.

## Reproduce

Run with the installed, approved managed CookBook connection. The model-turn benchmark needs an existing ChatGPT Codex sign-in and makes 36 measured turns by default. The typed and standalone runs make zero model calls. Each command needs its own new output directory and creates disposable fixture state inside it.

```sh
mkdir -p work
python3 tools/evals/benchmark_codex_fresh_threads.py --output work/fresh-model
python3 tools/evals/benchmark_codex_fresh_threads.py --typed-only --output work/fresh-typed
python3 tools/evals/benchmark_standalone_relay.py --output work/standalone
```

The checked [per-case CSV](fixtures/2026-11-19-fresh-threads/fresh-cases.csv) and [trace and relay evidence](fixtures/2026-11-19-fresh-threads/fresh-results.json) contain synthetic metrics only. The installed CookBook executable SHA-256 is in the trace. The benchmark overlays stayed process-local.
