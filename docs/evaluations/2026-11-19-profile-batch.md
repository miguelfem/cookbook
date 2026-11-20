# One-call profile batch evaluation

2026-11-19. macOS ARM64, Rust 1.94.0, unoptimized APR build, Python 3.11.
Eight distinct fictional email addresses were supplied in a fixed unsorted order.
Each lookup returned a fictional name, organization, and role after a simulated
220–280 ms I/O wait. The prepared arms used the same lookup. An independently
written oracle checked the complete ordered output. The fixture made zero
network or provider requests. [Example and rerun instructions](../../examples/profile-batch/README.md)

## Local execution

Four counterbalanced batches per arm. Timing starts after recipe registration
and includes the single MCP round trip for CookBook or the single subprocess
for the script. The CookBook payload size measures the MCP result object.
The script payload size measures JSON stdout, including the final newline.

| Runner · workers | Exact profiles | Median batch | Response bytes |
| --- | ---: | ---: | ---: |
| CookBook · 1 | 32/32 | 2.846 s | 1,437 |
| CookBook · 4 | 32/32 | 0.854 s | 1,437 |
| Prepared script · 1 | 32/32 | 2.068 s | 953 |
| Prepared script · 4 | 32/32 | 0.590 s | 953 |

All eight CookBook batches finished in the first compact reply. Every replay
returned the original run ID and output with the same request key. Four workers
ran CookBook about 3.3× as fast as its serial configuration on this
fixture. At four workers the prepared script finished sooner.

## Codex turns

Three fresh ChatGPT-authenticated CLI sessions per arm using Codex CLI 0.169.1,
GPT-6 Sol with Low effort, and automatic approval review. Case order rotated.
The direct arm could call the equivalent synthetic lookup for multiple addresses
and transform the records in one shell command. The script arm knew the prepared
command. The CookBook arm knew the immutable recipe reference and used a
run-only attachment with a compact result. All three were asked for the full
profile JSON. Recipe preparation took 0.41 s in the
harness and was excluded from turn timings. That figure covers fixture
registration and the disposable service and grant setup. It omits recipe
authoring and normal user onboarding.

| Arm | Exact batches | Calls per turn | Median turn | Total input | Cached input | Output | Standard API equivalent |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Direct agent | 3/3 | 1 | 12.07 s | 114,754 | 91,392 | 934 | $0.07434 |
| Prepared script | 3/3 | 1 | 11.59 s | 114,556 | 91,392 | 905 | $0.07366 |
| CookBook | 3/3 | 1 | 21.53 s | 166,858 | 138,240 | 1,197 | $0.09685 |

The local one-call path worked in actual Codex sessions. In this small workload
both baselines also needed one call. CookBook consumed more input tokens and took
longer than either baseline. Time includes client startup and any approval
review. The sample does not isolate which parts of the client caused the
difference. Estimates use Standard GPT-6 Sol rates of $2 for uncached input,
$0.20 for cached input, $2.50 for cache writes, and $10 for output per million
tokens. ChatGPT account charges were unobserved. [Model pricing](https://developers.openai.com/api/docs/models/gpt-6-sol)

An initial run used a client policy that refused the MCP start before execution.
Its CookBook arm returned 0/3 exact outputs. The corrected run enabled
client approval review for all arms and supplied the email list once in each
prompt. The initial failed run is excluded from the performance comparison.
Neither run evaluated real enrichment data, Claude Code, or warm reuse in
one chat.

[Sanitized per-case results and fixture hashes](fixtures/profile-batch-2026-11-19/metrics.json)

## Verification

Formatting and strict all-feature Clippy passed. Default workspace tests passed
418 tests and ignored 1. Fault-injection workspace tests passed
461 tests and ignored 1. The new v0.2 scalar key admission check
also verifies that v0.1 retains its property-pointer requirement.
