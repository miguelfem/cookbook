# E02: approved run trust diagnostic — 2026-09-22

After explicit owner approval, CookBook applied a fresh managed Codex connection
plan with `apr_run` approval mode `approve`. The installed skill and exact entry
matched the approved scope; all unrelated configuration remained unchanged.
Runtime execution grants, pins, project scope and revocation still apply.

The previously blocked arm of the
[prepared-conversation study](2026-09-22-e02-prepared-conversation.md) then completed
one setup and three held-out turns. **All 3/3 held-out outputs were exact, with
zero automatic approval reviews, manual approval requests, errors or retries.**
Each turn used one run call and read exactly its two required synthetic sources.

| Held-out case | Full client wall | Run-tool interval | APR execution | Noncached input | Output |
| --- | ---: | ---: | ---: | ---: | ---: |
| Amber — first reuse | 6.330 s | 0.147 s | 0.107 s | 7,976 | 232 |
| Cobalt | 5.215 s | 0.160 s | 0.102 s | 1,189 | 177 |
| Linden | 5.374 s | 0.155 s | 0.097 s | 2,537 | 169 |
| **Total** | **16.918 s** | **0.462 s** | **0.306 s** | **11,702** | **578** |

Total input was 186,294 tokens, including 174,592 cached tokens. Reasoning output
was 68 tokens. Counts are per-turn differences in cumulative app-server usage;
cached input is included, not additive. Separate review usage, billed dollars,
model-call count and service tier remain unknown. Low effort was requested for
every turn; actual model `gpt-5.6-luna` was confirmed, but effective per-turn effort
was not independently emitted.

The setup turn, excluded above, took 15.652 s with 15,474 noncached input and 538
output tokens. It made one run call, without an observed task-file read. The
earlier default-policy setup had read the task and repeated a deduplicated run
call. The client's skill catalog now contained the installed 7,920-byte APR
skill, but no explicit skill-body read was observed. These differences affect
retained context and prevent an isolated causal comparison.

## Interpretation

Observed behavior supports the approved per-tool policy taking effect: the prior
default-policy arm emitted an approval review for each run; this arm emitted
none, and its tool intervals were close to local execution time. Its first and
later reuse times were below the previous direct-Luna observations. However, this
was a **later, non-interleaved diagnostic**, not a new concurrent control study.
Do not present the 33.937-to-16.918-second comparison as an isolated causal speedup.

The default-to-trusted input difference was 45,272 tokens, including 45,056 cached
and only 216 noncached. Different setup behavior and retained context prevent
assigning that difference to approval alone. This is not a monetary savings claim.

The effective evaluation connection still used a **fixed run attachment**, with
run/inspection tools, against disposable state. It did not test the full normal
project-aware tool surface. The next token-efficiency comparison must include
that surface and same-model direct-script controls. Setup stays separate, but
discovery or repair recurring in a measured turn stays in its usage.

## Evidence and boundaries

[Case/setup CSV](fixtures/e02-trusted-prepared/case-metrics.csv) and
[sanitized metrics](fixtures/e02-trusted-prepared/metrics.json) retain results and
private evidence hashes. Original freezes and failed/blocked records were not
rewritten. The amendment fixed the harness's method-plus-ID request detection and
ended wall timing at `turn/completed`, keeping only the subsequent usage drain
outside the clock. Raw events confirm zero server approval requests; none were
answered by the harness.

The same frozen fixture, recipe and separately authored, non-human-reviewed oracle
were reused. Installed APR SHA-256 was
`11b3b47d6c0c035ec1e2405163f370d430300744f3863726375341eaa0d21712`;
Codex CLI was `0.155.0-alpha.9.2`. Private events, source access and durable receipts
remain under ignored `work/evals-2026-09-22/trusted-prepared-amendment/`. Disposable
service/client processes stopped cleanly. No live account, provider call,
production run, sandbox policy or other tool approval changed.
