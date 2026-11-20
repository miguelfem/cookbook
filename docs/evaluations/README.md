# Evaluations and evidence

[Saved recipes vs. Codex from raw inputs](2026-11-19-saved-recipes-vs-raw-codex.md) measures the same three workflows without providing Codex a ready-made script.

[Fresh threads and standalone runs](2026-11-19-fresh-threads-and-standalone.md) separates Codex turn overhead from one typed MCP call and measures a native schedule and a loopback queue outage drill.

[Codex alone vs. Codex with CookBook](2026-11-19-codex-head-to-head.md) compares three prepared workflows in paired model turns and records two separate first-use probes.

The [installed operation report evaluation](2026-11-19-installed-operation-reports.md) compares complete inline results with compact artifact delivery on two Codex-authored recipes.

The [connected MCP benchmark](2026-11-19-connected-mcp.md) measures warm turns, smaller results and direct typed dispatch with zero model turns.

The [profile batch evaluation](2026-11-19-profile-batch.md) measures one-call completion, worker limits, and actual Codex token use against direct and scripted execution.

## Latest follow-up — solved workflow reuse

The [reference-first browser report](2026-09-22-reference-first-browser.md)
records an agent-solved task converted into a background recipe: four exact
outputs, one-call solo/batch results and zero internal model calls. Seven real
browser probes and the required runtime/native checks passed; the report keeps
the initial redirect failures and current compatibility limits. This is bounded
engineering evidence, not a client-token or dollar comparison.

The [value diagnosis](2026-09-22-value-diagnosis.md) audits the original failures
and records a same-model Sol Medium comparison. All eighteen outputs matched;
CookBook was 5.0% slower and 4.5% costlier at API-equivalent rates than the prepared
script. Its 8.6% estimated saving against direct execution was dominated by one
cache-sensitive pair. No general quality, consistency or savings claim follows.

## Latest candidate — configurable workers

The [installed worker-capacity report](2026-09-22-worker-capacity.md) verifies
owner-selected limits through 64, continuous refill, native settings and a
preserved local installation. Its corrected native fixture passed 192/192
outputs; two failed pilots remain recorded. This is model-free capacity evidence.

The separate [README candidate evaluation](2026-09-22-readme-candidate.md) ran
E01–E05 within their stopping rules. Notes/handoff/scope passed after a harness
workspace correction. Autonomous E02 authoring failed; a disclosed assisted
setup then produced 9/9 exact prepared results, with CookBook slower and using
more total input/output tokens than the direct and reusable-script controls.
E03 stopped on uncertain adapter outcomes. Jev routing completed 24 calls but
failed its frozen gate at 13/24; Other discovery stopped before evaluation.
All five previously reviewed E05 controls behaved as expected. Three new external
effects remain uncertain, with no retries or invented zero-charge claim.

The report includes [charts](fixtures/readme-candidate/readme-candidate.png),
[client metrics](fixtures/readme-candidate/e02-cases.csv),
[provider outcomes](fixtures/readme-candidate/providers.json), original setup
failures, regression provenance and the exact tested package. **Launch acceptance
and general token/cost savings remain unproven.** The historical studies below
keep their original results.

## Earlier studies

The first internal evaluations were recorded on **2026-09-22**. They found working saved recipes, caller handoffs and scope refusal, but **the launch acceptance has not passed**. CookBook completed 2/3 original matched reuse cases; the direct agent and reusable script each completed 3/3. E03 provider outcomes remain unresolved. A later readback found the original Jev smoke completed; the wider E04 study is still partial. A fresh human-reviewed E05 follow-up observed all five expected control outcomes; its original blocked study remains unchanged, and label custody was not blinded. The [evaluation plan](../plans/evaluations.md) defines the experiments; the [measurement contract](../architecture/measurement.md) defines fair comparisons.

Launch acceptance covers reliability, recovery, understandable setup, caller-agent handoffs, exact shared-tool/account scope and permitted effects. Every advertised remote integration needs our own bounded live acceptance; an advertised Jev-enabled package also needs its small routing smoke. Larger comparative Jev studies remain separate. A client or platform unavailable on the test device is recorded as **untested**, excluded from the launch statement and kept pending. Later external usability sessions, larger comparative studies and provider-quality/economics work are optional follow-ups; none is implied by a local pass.

## What we have now

| Evidence | Available record | What it establishes |
| --- | --- | --- |
| Local runtime, scope and recovery checks | [Runtime validation](../logs/runtime-validation.md) | Observed behavior of the exact tested revisions and fixtures. |
| Packaged macOS app and installed MCP guidance | [Native/package validation](../logs/runtime-validation.md) | Account-free native journeys and installed contract availability on the recorded host. |
| Synthetic outcome/resource tooling | [Harness scope](../runbooks/distribution.md#measure-local-behavior) | Saved-definition execution, canned handoffs, protocol traffic and bounded resource samples. |

The engineering records above remain distinct from the actual Codex client studies below. Neither establishes broad compatibility or comparative savings. A validation log entry keeps its original source identity when later documentation changes.

## First recorded results

| Study | Question | Status | Result slot |
| --- | --- | --- | --- |
| E01 | Can the owner create and reuse a recipe on each available client? | Incomplete; launch criterion failed | [First-use report](2026-09-22-e01-first-use.md): three notes verified, real handoff and scope refusal; adapter recipe authoring failed. Claude Code absent. |
| E02 | Does CookBook execute and resume a repeated task under a small matched check? | Complete pilot; quality gate failed | [Reuse report](2026-09-22-e02-reuse.md): 8/9 exact outputs overall; CookBook 2/3 after one failed connection proof. Separate replay added zero source reads. |
| E03 | What does changing the external LLM trade off? | Stopped; inconclusive | [Model-choice report](2026-09-22-e03-model-choice.md): two uncertain adapter attempts, no graded provider output. Current-binding amendment prepared but not dispatched. |
| E04 | Do Jev routing and optional Other-learning help this task? | Partial; one Jev smoke completed | [Routing report](2026-09-22-e04-jev.md): actual caller 24/24 exact routes; rules 6/24 with one missed required review. Later readback verifies the original Jev route; Jev comparison, escalation and learning not run. |
| E05 | Does an independently checked recipe change preserve required results? | Blocked pending human review | [Improvement report](2026-09-22-e05-recipe-improvement.md): provenance guard held; zero controlled evaluations or adoptions. Separate diagnostics are not E05 acceptance. |

The later [reviewed E05 follow-up](2026-09-22-e05-reviewed-improvement.md) used
fresh frozen cases and explicit owner review: one correct simpler candidate
passed 6/6; two incorrect candidates scored 0/6 and failed; two unsupported controls
were inconclusive without execution. Fixture adoption, preserved pins and rollback
passed. A CLI readback argument error was repaired through read-only retrieval
of the same reports, with no reevaluation. This is a synthetic mechanism check
with accessible labels, not a blinded production-quality or savings study.

An [installed batch check](2026-09-22-batch-execution.md) verified one jobs JSON
and complete keyed results. A direct pure batch returned in one call; composed
child work needed later inspection. Native/API dispatch was serial at that checkpoint.
No model-token comparison was performed.

The [bounded-worker follow-up](2026-09-22-parallel-batches.md) implemented real
native/API overlap and complete short composed replies. All 18 synthetic batches
passed exact output checks; every new-build batch finished in one call. Four
native jobs averaged 0.493 s versus 1.839 s previously; through child recipes,
0.557 s versus 2.147 s. Setup was excluded and retained separately. Measured
worker overlap reached four; no model-token or live-provider comparison was run.

A separately frozen [prepared-reuse follow-up](2026-09-22-e02-prepared-reuse.md)
excluded installation, project connection and authoring from measured calls.
Three new cases passed in both arms, but prepared CookBook took **105.05 s versus
54.29 s** for direct Luna-low. Saved local runs finished in 111–137 ms each;
recurring client/tool overhead and inspection-argument failures remain. This
does not overwrite E02 or test stronger models at lower reasoning.

The subsequent [one-call follow-up](2026-09-22-e02-one-call.md) tested the new
development-signed package: all three CookBook results returned in the start call,
with zero inspections or tool errors. Model-free MCP calls took 103–121 ms. Fresh
client sessions still took **78.398 s versus 62.608 s** for direct Luna-low, so
the speed gate remains failed. Its [CSV](fixtures/e02-one-call/case-metrics.csv)
and [metrics](fixtures/e02-one-call/metrics.json) preserve client, MCP and worker
timings separately. It did not test successive calls inside one prepared conversation.

The [prepared-conversation follow-up](2026-09-22-e02-prepared-conversation.md)
then measured first and later reuse in the same conversation, with setup retained
separately. All **9/9** full outputs matched. Direct Luna totalled **33.208 s**,
CookBook Luna **33.937 s**, and CookBook Sol **33.744 s**, with low effort requested
in all arms. The first-reuse speed target still failed. Actual client approval
review occupied 10.946 s of Luna's 11.411 s of CookBook tool intervals; runtime
execution totalled 0.366 s. At that checkpoint the persistent-trust arm awaited
owner consent; that approval and its separate follow-up are recorded below. No
counterfactual speedup or monetary saving is reported. Its
[chart and data](2026-09-22-e02-prepared-conversation.md#observed-results) retain
quality, setup, timing and usage separately.

After owner consent, the [trusted-run diagnostic](2026-09-22-e02-trusted-prepared.md)
completed all three exact outputs with zero approval reviews and 16.918 s total
wall time. It ran later with different retained setup context, so the comparison
does not isolate a causal speed or token saving. The normal project-aware
connection and same-model direct scripts need their own matched comparison.

The subsequent [normal-project token study](2026-09-22-e02-token-efficiency.md)
retained all four arms: direct Luna/Sol and CookBook Sol passed 3/3; CookBook Luna
failed 0/3 after a proof-formatting error. The complete Sol comparison used 36.4%
more noncached input and 43.8% more time through CookBook. Every successful
CookBook reuse unnecessarily reloaded the authoring skill. Proof/schema and
skill-routing repairs follow that evidence; failed cases remain failures. A
separate four-turn regression on the installed repair passed every full output
and page check; both reuse turns used one run call, no skill reload and no review.
It uses previously exposed cases without new direct controls, so it does not
establish comparative savings.

The original E01–E05 candidate was the existing unsigned macOS ARM64 0.2.0 development app, using Codex CLI 0.155.0-alpha.9.2 and synthetic data. Each report identifies its hashes, procedure, failures and amendments; that original set built no new package. Later follow-ups identify the packages they tested. No uncertain provider attempt was retried, no credential was exported, and the original E05 provenance was not relabeled as human. The fresh follow-up records a separate explicit human review. Monid and fal were not called. The owner authorized up to $10 on OpenRouter; that original run used a stricter $1 reservation ceiling and stopped after one unresolved attempt. Actual provider charges and client subscription dollars remain unknown.

## Data and next decision

Chart-ready exports include [E01 sessions](fixtures/e01/sessions.csv), [E02 matched cases](fixtures/e02/case-metrics.csv), [E03/E04 provider and routing status](fixtures/providers/case-metrics.jsonl), and [E05 diagnostics](fixtures/e05/diagnostic-case-metrics.csv). The [dated summary](fixtures/2026-09-22-summary.json) links these outcomes without turning blocked or unrun cases into zeros. Raw JSON event streams, prompts, private state and freezes remain in ignored local `work/`. [Evaluation tools](../../tools/evals/README.md) explain the replay boundary and chart generation.

Separate [reviewed E05 cycle metrics](fixtures/e05-reviewed/cycle-metrics.csv)
and [batch metrics](fixtures/batch-execution/metrics.json) retain the follow-up
outcomes without changing the original summary snapshot. Unexecuted controls
have unknown quality, not a zero success rate.

The proof/skill repairs have separate passing regression evidence above; a fresh
matched client study is still needed for comparative claims. Resolve the existing
provider intents before new live calls and verify the current adapter/payload
path first. Reviewed E05 now has controlled mechanism evidence; broader quality
studies still need independent custody and representative cases. Cached tokens,
setup and failed attempts remain visible. Monetary and broad token savings remain
unproven. Bounded parallel dispatch now has the separate synthetic/native
evidence above; broader client and provider claims still need their own studies.

## Add a report

Copy the [report template](report-template.md) to a dated Markdown file in this directory. Link its internally reviewed aggregate data and reproducible fixtures, with sample counts, source/package identity, client/model versions, acceptance criteria and limitations. Include unsuccessful runs and protocol changes. Record unavailable quantities as unknown, including missing client or provider usage.

Future chart slots are described in the [visual guide](../media/README.md#results-charts). A chart links back to the report and data that support it. Any savings claim must include an equivalent accepted-result baseline, setup/maintenance and failed-attempt costs; otherwise describe only the narrower observed metric. No launch acceptance report authorizes a new paid call or proves provider quality, Jev benefit or token savings.
