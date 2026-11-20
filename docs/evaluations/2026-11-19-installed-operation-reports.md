# Installed CookBook and synthetic operation reports

2026-11-19, macOS ARM64. CookBook APR 0.2.0 was rebuilt from `c25b6480b16a3ea41298a4d030cefdeb43b1d7c4`
as a development-signed app with its existing optional Jev support. Native
compilation and strict Clippy passed. The DMG bundling helper failed, so the
signed app was bundled directly and verified with deep, strict code signing.
The staged and installed apps used the existing app and sidecar designated
requirements. A private rollback app and runtime backup were retained.

The pre-test upgrade check matched all **51** saved database tables and
the original credential and Codex configuration bytes. The five unresolved
runs were preserved. The native app reopened. No change to GitHub visibility
or external deployment was made.

## Prepared Codex acceptance

A fresh Codex app-server thread used the existing installed `apr` command,
project-aware arguments, and scoped `apr_run` approval. A disposable synthetic
project was connected, granted its own local interpreter and given the
registered eight-profile fixture before timing. The actual Codex agent made
one `apr_run` call, with zero discovery, shell or inspection calls in the measured
turn. The MCP result and agent response both contained all eight exact profiles
in order. The call took 0.869 s and the turn took 12.663 s. The
runtime recorded eight successful fixture effects and zero provider calls. The
synthetic assignment was closed after verification.

## Put the work in the recipe

Codex authored the two CSV workers and their manifests once. The authoring turn
reported 228,928 input tokens (214,016 cached) and
4,029 output tokens. That setup is separate from reuse. The first
runtime probe exposed a result-schema mismatch: the draft described the worker
placeholder where the runtime validates the resolved artifact reference. The
manifests were corrected, and standalone report writes were changed to refuse
overwrites before repeating the probe.

The fixtures cover inventory reconciliation and ticket SLA rules at 72, 96 and
144 rows each. An independent oracle verified every ordered report row and
content digest. Both variants wrote byte-identical reports. Each agent turn
made one run call. Only the output boundary differed between the variants.
The model was asked for the counts and report ID in both arms. An earlier
pilot returned null in chat despite exact tool output and was recorded before
the harness was corrected to keep scoring agent replies separately.

| Reply | Exact tool results | Exact agent replies | Tool reply bytes | Input tokens across 6 turns | Median chat turn | API-equivalent estimate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Rows, counts and report | 6/6 | 6/6 | 6,988–16,636 | 353,290 | 7.97 s | $0.13088 |
| Counts and report reference | 6/6 | 5/6 | 461–465 | 332,014 | 8.12 s | $0.11890 |

The concise replies were **93.4–97.2% smaller** with the same
stored reports. The matched warm turns used 6.0% fewer input tokens.
The difference in estimated model cost was 9.2% with observed caching.
The sample did not establish a general chat latency or cost advantage. Typed
MCP calls and agent recaps were independently scored. Standard GPT-6 Sol rates
were used for the estimates. Actual ChatGPT account charges were unobserved.
[GPT-6 Sol pricing](https://developers.openai.com/api/docs/models/gpt-6-sol)

## Freeze the narrow interface

The final brief variants take only the input artifact and return only counts
and a report reference. With **300 rows per workload**, direct calls through the
installed binary's managed-project MCP returned exact output in
**0.115–0.508 s** with zero thread tokens. The replies were 455–457
bytes, and verified exports recovered complete reports of 27,210 and 33,710 bytes.
Codex also dispatched both variants in one tool call each. Both tool results
were exact, and 1/2 chat recaps reproduced the counts.
A consumer with typed inputs can display the structured tool output directly.

The standalone prepared Python baseline did the same deterministic processing
and wrote exact reports for all six inputs. The public model-free harness passed
all 14 runtime checks using the installed executable. It can
be rerun without a model account. Tests also covered malformed headers,
duplicate IDs, and protected report destinations.

[Recipe source and rerun instructions](../../examples/operation-reports/README.md) · [Sanitized metrics](fixtures/installed-operation-reports-2026-11-19/metrics.json)
