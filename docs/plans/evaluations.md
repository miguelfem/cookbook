# CookBook evaluation plan

Future reuse studies begin with an observed solved workflow, derive both recipe
and script from the same frozen reference, and compare fresh matched inputs after
setup. Autonomous from-scratch authoring remains a separate setup question. The
[reference-first browser check](../evaluations/2026-09-22-reference-first-browser.md)
qualifies one model-free execution path; it does not replace E01–E05 or establish
comparative savings. [Measurement rules](../architecture/measurement.md) define
the reference and holdout boundary.

The later [configurable-worker candidate run](../evaluations/2026-09-22-readme-candidate.md)
records fresh E01–E04 attempts and the exact previously reviewed E05 regression.
Prepared E02 passed 9/9 only after disclosed setup assistance; autonomous
authoring failed. E03 and Other learning stopped on uncertainty, and Jev routing
failed its frozen quality floor. The launch gate remains unpassed. These later
records supplement the original protocol and never replace its failed attempts.

Status: **first execution recorded on 2026-09-22; launch acceptance did not pass**. E01 was incomplete, E02 completed with a CookBook connection failure, E03/E04 stopped with unresolved provider outcomes, and the original E05 was blocked pending human review. A [fresh reviewed E05 follow-up](../evaluations/2026-09-22-e05-reviewed-improvement.md) subsequently observed all expected control outcomes, with disclosed same-agent access to labels. See [dated reports and data](../evaluations/README.md). Preserve this protocol and the reports' amendments; a repaired journey needs newly frozen inputs. The required launch slice is a small acceptance run by the maintainer team using owned devices, accounts and agents. It does not recruit participants or require an outside reviewer. This plan alone does not authorize paid calls, account setup or publication. [Measurement rules](../architecture/measurement.md) own the comparison method.

The first question is whether the owner can use CookBook from the published instructions on the first launch cut. The next is whether a saved recipe executes and resumes correctly under a small matched check against direct agent work and a reusable script. Optional model providers and improvement loops should earn their place in a particular task.

## First-launch boundary

The required launch acceptance targets the unsigned **macOS ARM64** package and the Codex and/or Claude Code binaries actually available on the test device. Run each available client; record an unavailable client as untested. Other operating systems, architectures, client versions and missing native capabilities are excluded from the launch claim and remain pending. A passing local fixture, protocol test or packaged binary does not silently pass an unavailable platform or a model-driven client journey.

Launch-blocking checks cover reliability and recovery, understandable first-use setup, caller handoffs, exact account/tool scope, protected credential handling and permitted effects. They use synthetic project data and disposable runtime state. The owner records the result and can fix, stop or narrow the release. Later external usability, larger cohorts, broad platform coverage and population-level conclusions are optional follow-up studies.

## Start small

1. Freeze one candidate app/package, the README revision and the available-client list. Reuse its relevant [engineering validation](../logs/runtime-validation.md); rerun only checks affected by a change or an unresolved failure.
2. Run the internal E01 first-use acceptance in every available client, with a fresh project and disposable state. Fix blockers before calling the launch slice complete; keep absent clients explicitly untested.
3. Run the internal E02 instrumentation slice with three held-out synthetic inputs in each condition: nine measured executions plus separately recorded authoring. Verify that usage and quality can be measured before expanding.
4. Run E05 when the optional improvement workflow is included in the launch package. E03/E04 are separate provider studies and require an explicit account, model and spend decision; they are not prerequisites for the Jev-free core.

Each stage ends with a report, including failure or inconclusive results. This plan does not authorize live inference, publication, account setup, private-data export or automatic optimization.

## E01: Can the owner get a useful first result?

**Launch acceptance:** one maintainer-run session per available client on the same frozen macOS ARM64 package. Use the owner's device, client accounts and synthetic project content; do not recruit participants or ask an outside reviewer to operate the product. If a client is absent or cannot be authorized, record it as untested and narrow the launch statement.

Starting from the README and an empty project, install the package, connect the available agent, create/run the save-note starter, change its input and run it again. Then use a second empty project to check that the agent creates its own workspace without manual project setup. Exercise one caller handoff, one approved MCP/tool operation, one saved account reuse where available, and one denied out-of-scope operation. Include one required capability with no preinstalled adapter: the agent must prepare and test the supported API/CLI route, while the owner only authenticates and approves meaningful access or installation. A provider restriction or unsupported credential flow stays a named blocker. Record what the owner understands about recipes, approvals, waiting and provider billing.

For every remote connection named as launch-supported, run its browser/key flow,
list metadata, reopen the app, reuse the saved account and perform one explicitly
approved, bounded read in an owned disposable service project. Refusal and
out-of-scope checks must pass. This is our integration acceptance; provider
availability failures stay recorded. Unsupported services are excluded explicitly.

If the launch advertises the Jev-enabled package, add a small live smoke with our
own approved account: up to eight frozen synthetic routing cases covering a clear
route, Other and caller escalation. Require valid provider responses, the declared
routing/continuation behavior and receipts with observed usage or explicit unknowns.
Freeze the account, model and spend ceiling before dispatch; stop on uncertainty.
This smoke is required for the advertised adapter, while E04's larger comparative
quality/category-learning study remains optional. No paid calls are authorized by
this plan.

Record time to first correct artifact, completion without code changes or hidden rescue, installation/trust blockers, client restart requirements, recovery after closing/reopening the app or conversation, and misunderstandings about credentials or model billing. Record owner approvals separately from product failures. Use synthetic project content and retain the exact package/client/source identity.

**Launch decision:** every available client must complete the declared first-use path, caller handoff and permission checks on the frozen package, or the launch scope is narrowed and the failure is recorded. Any unintended permission change, lost committed result, unsafe external effect, false success or unrecoverable handoff blocks that capability. This is an internal acceptance result, not a population usability estimate.

**Distribution boundary:** the public source preview has no signed installer. A maintainer-supplied development package can test the app flow on the target device, but label it development-package acceptance. It does not validate finding, downloading or installing a public release. That journey remains a later release gate.

## E02: Does reuse help at the same quality?

**Tasks:** begin with a useful wrapper operation: accept a record ID, read two approved synthetic service sources, normalize their results in code and return one validated result with evidence. Exercise it through a direct agent, a reusable script and a saved recipe. This tests callable orchestration, not whether a schedule fires. After the small instrumentation slice, use a document-to-action workflow or project-specific service operation from the README, including a controlled caller judgment. Freeze inputs, required outputs and acceptance criteria before authoring.

Compare three conditions on the same client/model/settings and equivalent tools:

| Condition | Fair treatment |
| --- | --- |
| Direct agent | Ask a capable agent to complete the task. Allow its normal tools and scripting; do not force one model turn per trivial operation. Record any reuse it develops. |
| Agent with a reusable script | Prepare a competent reusable script for the same job. Allow the same batching and output limits; include the script's setup and maintenance cost. |
| Agent with CookBook | Front-load installation, project connection, recipe authoring/approval and development checks. The first measured call to the frozen prepared recipe counts as reuse number one. Include every discovery/runtime call and handoff that actually occurs during that call. |

For the **required launch slice**, use **three matched held-out inputs per condition**: nine executions for one deterministic task. Treat input cases as the comparison units. Record cold authoring once per condition as an observed session, not a stable estimate; use separate authoring examples, then freeze script/recipe versions. Counterbalance condition order where practical and isolate task context so outputs or solutions do not leak between conditions. Record cache state. A later comparative study may expand to 10 matched inputs per condition (30 executions for one task, 90 across three tasks) without changing the launch result.

The first E02 cut repeated project connection in fresh clients and therefore did
not isolate prepared reuse. Preserve it. A separately frozen diagnostic follow-up
uses new holdouts and a fixed run attachment, with local worker, MCP-to-result and
complete client timing reported separately. It does not erase onboarding failures
or replace the three-condition launch gate. Setup cost stays visible outside
per-reuse totals. Beating Luna on the first prepared reuse is a testable target;
stronger-model/lower-thinking comparisons require their own frozen model controls.

Measure accepted outputs, required evidence, all visible model/provider usage, elapsed time, human correction, and tool/result/artifact traffic. Grade deterministic outputs exactly; use a frozen rubric and blind review for judgment. Include costs from failed and repaired attempts. Report cost per accepted result as **all attributable execution cost divided by accepted results**, with setup and maintenance separately visible. If no output passes, that ratio is undefined.

Report cumulative cost at the observed repetition counts, plus an explicitly labeled break-even estimate where usage is complete. Missing client/subagent usage prevents a total-cost or token-savings claim. A subscription-native tool call is not automatically free or priced like an API call. CookBook's own billing view does not capture the client's full model usage.

**Launch decision:** required-output quality, recoverability and permitted effects must meet the frozen acceptance criteria in all three conditions; a cheaper failing output does not qualify. The nine-case slice does not establish broad statistical equivalence or savings. Publish counts, paired differences and ranges, and label any observed advantage as exploratory. Keep one controlled restart/replay check per task separate from the normal performance sample; verify saved work resumes without repeating committed effects. A larger external or multi-task study is optional.

## E03: Can a user choose a model for a useful tradeoff?

Use one grounded summarization or extraction recipe and **10 frozen cases × two approved model APIs × two repetitions**: a hard ceiling of 40 task-provider dispatches, including failed or uncertain calls. No replacement calls are added to this pilot; a further trial needs a new protocol and budget. Select compatible providers and a spend ceiling before execution; this document selects neither. Count authoring, evaluation usage and all failed/uncertain charges against the spend ceiling. Stop when a bound is reached; do not retry an uncertain paid call automatically.

Keep the recipe, source material, output contract and review rubric fixed. Freeze provider-specific prompt translations before opening the holdout and publish their differences. Change only the declared model connection/settings. The maintainer records rubric decisions from the frozen acceptance procedure, with blinding to provider identity or cost where practical; a second internal pass is optional and is not an outside-review requirement. Compare accepted-result rate, observed cost per accepted result and latency, including both repetitions. Record actual model identifiers and unresolved aliases.

**Pilot decision:** reject any configuration that fails the required factual/evidence criteria or the predeclared quality floor. A useful observed cost/latency tradeoff may justify a larger task-specific trial. This study measures provider choice; it does not by itself show that CookBook saves more than direct API orchestration.

## E04: Does Jev improve routing, and can Other safely evolve?

Keep routing and category evolution as two separately reported phases.

**Routing study:** use 24 independently labeled cases, including clear matches, ambiguous inputs, unsupported requests and cases that must be reviewed. Compare exact rules, the caller agent and Jev on the same declared routes and authority. Record retrieval coverage where relevant, correct routes, missed required reviews, unnecessary reviews, whole-task acceptance, latency and every model/escalation cost. Match the output contracts and include the reasoning cost of the caller baseline. Freeze thresholds before the held-out run; reported confidence is not proof of correctness. This study is optional and requires an explicitly authorized Jev account and spend bound.

**Other-learning pilot:** enable collection explicitly. Use at most 16 discovery examples to propose one or two justified categories, then freeze the candidate. Use a separate independently labeled set with at least two examples per category including Other. That held-out batch contains **at most 32 examples total**, within the 24 KiB batch limit; the discovery examples never enter it. The two-arm evaluator makes at most 64 Jev calls. Include known categories, true new categories and cases that should remain Other. Report old-category preservation, new-category coverage, remaining errors and no-change outcomes; do not reward reducing Other by forcing a label.

**Study decision:** honor the implemented [classifier gates](../architecture/classifier-learning.md), and separately require that every mandatory-review case routes correctly in the held-out study. Example thresholds and small batches are not production calibration. A passing candidate needs explicit owner review before adoption. A revision uses new held-out data; providers, templates and schedules never change automatically as part of this study. No Jev result is required to launch or operate the Jev-free core.

## E05: Can a proposed recipe improvement survive an independent check?

Use the [implemented pure evaluator](../architecture/recipe-improvement.md): initial pilot of **six development plus six held-out cases** in a cycle, bounded by its maximum 12 cases per split and 24 supported pure steps. Keep required schemas, configuration and full expected outputs fixed. Labels must come from an independent specification or reviewer, not the baseline output alone.

Use separate cycles for a genuinely simpler candidate, a cheaper-but-wrong candidate and missing evidence. Keep the unsupported provider/effectful path in a separate negative-control fixture that intentionally makes its entire evaluation inconclusive; it never enters the pure candidate's held-out batch. Expect pass, fail or inconclusive as appropriate. Verify explicit owner adoption of the exact tested candidate and unchanged existing runs/schedules. Reuse existing regression evidence for authority/recovery mechanics; this study adds independently specified task cases and can run without a provider.

Report executed pure steps and exact required-output coverage. The built-in evaluator's cost and latency fields remain unknown: a reduction in steps is not a measured dollar or token saving. Live model/provider optimization requires a separately designed evaluation, not an expanded claim for this evaluator.

## Shared controls and reporting

- Register task, comparisons, success/failure criteria, budgets, stopping rules and exclusions **before** looking at held-out results. Separate development and holdout by source entity/document as well as exact input. Freeze dataset hashes and policy/candidate versions.
- Retain holdout labels outside the authoring agent's accessible workspace until evaluation. The maintainer may be the evaluator because this is internal acceptance; record label custody and any possible leakage. In-app label hiding is not isolation from an agent with filesystem access. Retuning requires fresh held-out cases; do not reuse a previous cycle's holdout for development.
- Use synthetic/public or explicitly authorized data. Keep raw traces, private text, keys and attachment tokens out of repository exports. Shared reports contain reviewed aggregates and reproducible fixtures where distribution rights allow.
- Count failed attempts, refusals, corrections, uncertainty and setup. Keep absent usage as `null`/unknown, never zero. Report denominators, protocol deviations and unsupported comparisons.
- A report identifies app/package checksum, source/dirty state, client/model versions, feature flags, schema/config/input/policy hashes, timing method, grading and spend evidence. Start from the [report template](../evaluations/report-template.md).

Later studies may cover installed footprint and bounded computer use in a dedicated disposable app/session. Existing [local harnesses](../runbooks/distribution.md#measure-local-behavior) measure service resources and synthetic outcomes; canned handoffs and protocol bytes are not human quality or model tokens. Native computer-use accuracy and upstream benchmark numbers remain separate evidence.

## Next decision

The first run found exact proof-file authoring and manifest feedback to be
immediate client-journey problems. Later repairs have separate regression evidence;
retain the failures and use fresh matched cases for comparative claims. E03/E04
need evidence about the existing uncertain intents before further dispatch.
Reviewed E05 now establishes the bounded pure control outcomes, fixture adoption
and rollback; its accessible labels limit broader quality claims. The installed
batch check establishes keyed aggregation, not parallel dispatch or token savings.
