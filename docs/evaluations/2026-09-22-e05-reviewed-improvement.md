# E05 — fresh reviewed recipe-improvement controls

Date: 2026-09-22. **All five expected control outcomes were observed** through the
installed app's scoped MCP evaluator. The simpler candidate preserved all six
held-out outputs with one fewer pure step per case. Both incorrect candidates
failed; unsupported and effectful candidates were inconclusive before execution.
Fixture-only adoption, preserved history/schedule pins and rollback passed.
No production recipe was changed and no provider was called.

This is a new follow-up to the [original blocked E05](2026-09-22-e05-recipe-improvement.md).
Its exposed diagnostics and provenance rejection remain unchanged. This small
synthetic study qualifies the bounded improvement mechanism; it does not establish
production quality, model-token savings or faster execution.

## Review and frozen protocol

Five candidate definitions and **60 fresh cases** were frozen before execution:
six development and six held-out cases for each candidate. Source IDs and complete
inputs are disjoint across all cases and differ from the original E05. The rule
comes from a deterministic specification, not observed baseline outputs: return
exactly the input's `answer` and `evidence`, preserving whitespace, Unicode,
newlines and special characters, with no extra fields. Answer and evidence differ
in every case so the negative controls cannot pass accidentally.

The owner reviewed the rendered cases and explicitly answered **“Yes—expectations
reviewed and approved.”** A receipt binds this approval to the exact expectation
file, SHA-256 `20013b874069e2e5c604e5e4d835510578e0a46afad366ed47ca00ffc3d2f7a4`.
The original file still records pending provenance as a historical freeze;
the harness set `provenance: "human"` in memory only after validating the receipt.
These are specification-generated, human-reviewed labels, not human-authored data.

The executing agent could access the labels. This is a disclosed departure from
the plan's isolated holdout custody, not a blinded test or an external quality
review. Candidates were fixed before results; there was no retuning or replacement
evaluation. Development cases were frozen design context and **were not executed**
by `pure-v1`, which evaluates only held-out cases.

Acceptance was frozen as exact complete output and schema validity for all six
held-out cases, plus at least one fewer completed pure step per case on average.
Unsupported operations must produce an inconclusive report without dispatch.
The five reports each consume one evaluation; reading a saved report again must
not execute it again. Adoption and rollback are limited to the disposable fixture.

## Actual outcomes

| Candidate | Result | Cases executed | Baseline exact | Candidate exact | Baseline → candidate steps |
| --- | --- | ---: | ---: | ---: | ---: |
| Simpler, same required output | Pass | 6 | 6/6 | 6/6 | 12 → 6 |
| Cheaper, wrong answer | Fail | 6 | 6/6 | 0/6 | 12 → 6 |
| Cheaper, incorrect evidence | Fail | 6 | 6/6 | 0/6 | 12 → 6 |
| Unsupported builtin | Inconclusive | 0 | Not run | Not run | Not run |
| Effectful connector | Inconclusive | 0 | Not run | Not run | Not run |

The three supported comparisons executed **36 pure recipe runs** inside the
evaluator's disposable states: 18 baseline and 18 candidate. The controlling
workspace retained one separate historical baseline run. All supported candidates
returned both fields, but wrong values still failed. Saving steps never overrode
complete-output correctness. Unsupported controls dispatched neither arm.

The passing report was read back through `improvement.evaluate` with the identical
digest and `evaluations_used: 1`. Fixture adoption selected that exact candidate
for future discovery at revision 1. The existing successful run and paused future
schedule retained the baseline hash. Explicit selection rollback restored the
baseline at revision 2. All five reports subsequently matched scoped CLI readback.

The final controlling database contained **zero saved accounts, execution grants,
effect intents and billing calls**, five improvement cycles, one historical run
and one paused schedule. Integrity and foreign-key checks passed. Disposable MCP
and service processes exited; the user's live runtime was not used.

## Retained failure and recovery

The original harness completed the evaluations, adoption, pin checks and rollback,
then stopped at its final CLI readback with `INVALID_INPUT`. It had sent
`detail: true`, an MCP-only response-budget field, to the service-level CLI
inspection contract. This was a harness error, not an evaluated recipe failure.

A separate, recorded read-only amendment omitted that field and retrieved the
five existing reports. No evaluation, adoption, selection or input was repeated
or replaced. The original harness, raw result, error description, MCP events and
recovery remain retained. An earlier static review also corrected an unsupported
`detail` field on `schedule.pause` before any execution.

The original failing CLI response was visible in the task output but was not
saved as a standalone raw log. Its request and the recorded error description
remain. `final-result.json` reconstructs the completed adoption/rollback from
original MCP events and supplements it with recovery-time CLI/SQLite reads.
An independent read-only audit reconciled those records and all five report
digests; it found no repeated evaluation or additional effect dispatch.

## Reproduction and limits

| Item | Recorded identity |
| --- | --- |
| Checkout | `e7de5595be5ec16e4fb6e973a9cb1c389d186fc2`, with plan documentation edits and ignored evidence helpers; runtime source unchanged. |
| Installed app | macOS ARM64 26.6.2, CookBook APR 0.2.0 development-signed app; embedded CLI reports 0.1.0. APR SHA-256 `df9b08eae1d36d10b2dded2047d63e9cb1e2239845417e535ab05842b36b37b8`. |
| Existing package | `CookBook APR_0.2.0_aarch64.dmg`, SHA-256 `bc03738cbc1db2ed32a254e39de320cfd3e565b399f71a455e9afe3ea2987213`; [build and installation record](../logs/runtime-validation.md#2026-09-22--proof-robustness-and-prepared-run-skill-routing). No new package built for this evidence-only change. |
| Interface | Installed CLI and scoped MCP, `pure-v1`; no client model, provider, credentials or native worker. |
| Execution freeze | `d9de5b9b483ae5dfcc5e1f40a2c61f6be7f13e4f3a834930529a80d30cbd9c35`, recorded before runtime work at 2026-09-22 19:18:51 UTC. |
| Original harness | `b53d13ea1ac3b5b9a1450ad7efff8eac990eabc61d7f5db0788c1b476eb409af`; archival CLI/MCP transport helper `78cb5c2cb87b13e5208bc532a049d7304e14d77a40559955acae56a990b7d17f`. |
| Read-only recovery | `d56776505046a1e2b046180e2040f3e8e73c5bd9ae84aa13b20f7188ced888eb`. |
| Final private result | `e280c3fcfa6cdc74acb4bda230e2fa40cf7a5a61c48246e0b36575d504d87495`. |

Full receipts, hashes, cases and raw responses remain in ignored
`work/evals-2026-09-22/e05-reviewed-followup/`. The public
[cycle metrics](fixtures/e05-reviewed/cycle-metrics.csv) and
[study record](fixtures/e05-reviewed/study.json) contain aggregate outcomes,
version/report identities and custody limitations, without raw labels or private
attachments. These files support charts without converting unexecuted controls
into quality zeros.

Steps are structural counts. Evaluator latency, caller/model tokens and monetary
savings remain unmeasured; the evaluator deliberately reports cost and latency as
unknown. Live-provider optimization requires a separate protocol. Runtime code
did not change, so the full Rust suite and app build were not repeated for this
follow-up; documentation validation is recorded in the current validation log.
