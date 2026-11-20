# E02 follow-up — prepared recipe reuse

Status: **six sessions completed; exact quality passed, speed target failed**.
After front-loading setup, CookBook still took 105.05 seconds versus 54.29 seconds
for direct Luna-low across three new matched cases. This diagnostic follow-up
preserves the [original E02 failure](2026-09-22-e02-reuse.md); it does not replace
the three-condition launch study or establish a stronger-model benefit.

Subsequent repair and fresh measurements are recorded in the
[one-call follow-up](2026-09-22-e02-one-call.md). The results below remain unchanged.

## Frozen comparison

Three new synthetic invoice-readiness inputs and an independently calculated exact
JSON oracle were frozen before prompts. Both arms used Codex CLI
`0.155.0-alpha.9.2`, `gpt-5.6-luna`, low reasoning, ordinary approval review,
fresh client sessions and a 180-second/12-tool-call ceiling. The direct agent
retained normal scripting. Order was direct/CookBook, CookBook/direct, direct/CookBook.

CookBook's project, recipe, Python grant, fixed run-mode attachment, preflight and
warm service were prepared before timing. Every measured call, beginning with
the first reuse, received the exact recipe reference and one new record ID.
No project connection or discovery occurred during measured calls. Actual errors
and recovery remained included. No external API was called.

Private protocol SHA-256:
`862e009fde2e06a730f65448c4cba83d161770abc0962c33a8fe9b1ef135b286`.
Fixture freeze SHA-256:
`d56c1b8b77d62a91ecbf76411ddfbec5b4d2e745d97b16993e3c80144338e633`.
Tested packaged APR SHA-256:
`bd3beb0d5fe721893a620633792da8c8e55ccf4cfbc3bb57d3875cb3dbbabe45`.
Source/prompt fingerprint checks and independent oracle recalculation passed.
Labels lived outside the client project; same-user filesystem access remains a
blinding limitation. Full manual setup time was unmeasured. Registration/preflight
took 0.161 s and the warm connection check 0.025 s; neither is a setup total.

## Results

| New case | Direct client | Prepared CookBook client | Saved run created → succeeded | Exact output |
| --- | ---: | ---: | ---: | --- |
| prep-ash | 14.852 s | 29.398 s | 137 ms | Both pass |
| prep-birch | 24.063 s | 46.817 s | 133 ms | Both pass |
| prep-pine | 15.375 s | 28.833 s | 111 ms | Both pass |
| **Total** | **54.291 s** | **105.048 s** | **381 ms** | **3/3 each** |

| Reported client usage | Direct | Prepared CookBook |
| --- | ---: | ---: |
| Input tokens | 148,762 | 615,159 |
| Cached input, included above | 123,648 | 506,880 |
| Noncached input | 25,114 | 108,279 |
| Output tokens | 1,455 | 2,057 |
| Tool calls | 9 | 12 |

CookBook made one command call, one `apr_run` and two `apr_inspect` calls per case.
Every first inspection omitted `view` and failed; the agent then added
`"view":"run"` and retrieved the correct output. The exposed schema requires that
field inside its run variant, but inspection has 29 variants and its description
does not show that basic call explicitly. This is an observed client argument
failure; there is no evidence that the client modified the schema.

`apr_run` client tool events lasted 4.378–12.293 seconds, while native effects
lasted 109–135 ms. Client approval, scheduling, transport and model turns were not
isolated; the difference cannot be attributed entirely to the runtime. Pure MCP
wire latency remains unknown. Every arm/case read each source exactly once.
Subscription dollars and approval-review model usage were unavailable. Cached
tokens are a subset of input, not an additional quantity or a full-price estimate.

## Decision and retained evidence

CookBook missed the first-prepared-reuse speed target in all three pairs and was
1.94 times slower in aggregate. Setup does not explain this remaining gap. Local
execution is fast; the client interface still adds calls, context and recoverable
argument errors. Prioritize that recurring overhead before expanding model tests.
A stronger model with reduced reasoning was **not tested** here.

[Case CSV](fixtures/e02-prepared/case-metrics.csv) and
[sanitized metrics](fixtures/e02-prepared/metrics.json) support new charts without
changing the original exports. Raw prompts, events, private state, oracle and
hashes remain in ignored `work/evals-2026-09-22/prepared-reuse/`. No case was
replaced, retuned or omitted after observing its result.
