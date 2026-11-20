# Measurement and release gates

The [evaluation plan](../plans/evaluations.md) turns these rules into staged pilots. Completed reports belong in the [results index](../evaluations/README.md); unexecuted studies remain labeled planned.

## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

Design updated 2026-09-16. All numerical targets below are provisional engineering goals, not measured results.

## Compare the same accepted result

For future recipe-preservation studies, first retain an observed successful agent
workflow as the reference. Freeze its inputs, outputs, actual tools, required
checks, evidence and variable fields before deriving the recipe and equivalent
prepared script. Measure solving, extraction and validation as setup. Supply fresh
matched inputs for reuse; do not leak held-out outputs through the reference or
force the direct agent to discard useful prepared tools. Keep autonomous authoring
from a new request as a separate setup study. Existing reports retain their actual
method and original outcomes; this clarification does not regrade them.

Use three initial fixtures:

1. **Deterministic report**: read a fixed set of local records, normalize, deduplicate, validate, summarize, and package. No model judgment needed inside the pipeline.
2. **Document review**: source collection to per-document review handoffs, validation, and a review package. Keep source documents, review requirements, document count, and acceptance rubric constant.
3. **Scoped connector**: fetch records, filter locally, and perform one permitted mutation against a controlled test service. Inject response loss and test reconciliation.

Compare normal direct-agent execution, a competent reusable script and APR, separating cold authoring/first execution from repeat runs in each condition. The direct agent may batch or script work; do not force wasteful tool-by-tool behavior to inflate savings. Include the old n8n workflow if its runnable definition is available; do not invent its baseline from recollection. Add a bounded Lobster/Code Mode comparison where the same task can be supported fairly.

For a prepared-recipe reuse comparison, installation, project connection, authoring,
approvals and development checks are front-loaded setup. The first measured call
after that freeze is reuse number one; it must not inherit the setup total. Report
setup and cumulative economics separately. Freeze a scoped attachment and exact
recipe reference before supplying fresh held-out inputs. Report whether the service
is warm; cold service startup is a separate measurement, not an unreported exclusion.
Discovery or repair that actually recurs during a measured call stays in its time
and token totals. A prepared-attachment result does not qualify the default
project-connection journey in a new client session.

Separate worker execution, MCP start-to-completed-result, and full prompt-to-final
agent time. Count cached input, noncached input, output/reasoning and internal model
calls separately where reported. Faster first reuse than a small direct agent is
a target to test, not an architectural guarantee. To test a stronger model with
less thinking, freeze model and reasoning settings per arm and include a same-model,
same-effort direct control; otherwise model selection and delegation are confounded.

Use the same client/model/settings and equivalent tools/provider operations. Record deviations. For creative output, use a predefined quality rubric and preferably blind review. A cheaper output that fails the rubric is not a saving.

## General routing and composition

After the baseline engine is proven, add composed fixtures across research, business operations, and software delivery. Vary candidate-catalog size, branch depth, parallel child count, missing context, and interruption points. Use the same approved inputs and outcome criteria for rules-only routing, agent routing, and the optional decision provider. See the [routing contract](workflow-routing.md) and [Jev evaluation proposal](jev-integration.md).

Report candidate-retrieval coverage separately from selection accuracy. Include unsupported requests and cases where the right process is deliberately absent; forced matches are failures. Track wrong routes, missed required escalations, unnecessary review, whole-assignment acceptance, and correction/recovery cost by domain and complexity. Do not infer end-to-end reliability by multiplying model confidence values.

Count state preparation, catalog maintenance, every decision call, agent escalation, and changed-process authoring. For restart tests, verify that committed routing decisions and child runs are reused without new model calls. Compare completed assignments, not just price or accuracy per prediction.

## Record separately

| Metric | Required treatment |
|---|---|
| Model input/output tokens | Include orchestration, authoring, subagents, retries, and handoff work where visible. |
| Cache/provider usage | Record actual reported usage; missing values are unknown, not zero. |
| Native image/model costs | Count separately from orchestration; never assume subscription-native calls cost the same as API calls. |
| Accepted-result rate | Track rejection/rework alongside token counts. |
| Human intervention | Count setup, approval, correction, and reconciliation events. |
| Wall time | Separate machine execution, model waits, provider waits, and human waits. |
| Host footprint | Binary/install size, service and worker memory, idle CPU, cold startup, repeated call latency. |
| Context traffic | Tool catalog bytes, result bytes, and artifact reads; token counts may be client-specific. |

Store JSONL measurements with fixture/version, input hashes, client/model versions, usage evidence, run identifiers, quality result, and timing. No source documents or credentials in the default measurement export.

## Economic model

Let A be extra authoring/setup cost, B the accepted per-run baseline cost, and R the accepted per-run APR cost after setup. If B > R, approximate break-even repetitions as `ceil(A / (B - R))`. If B ≤ R, there is no observed cost break-even under those conditions.

Report authoring maintenance and failure recovery, not just the best repeat run. Begin with 10 repeated deterministic trials per condition and several matched creative trials; show ranges and all exclusions. Expand sample size when variance or quality differences prevent a useful conclusion.

## Initial performance targets

On a specified Apple Silicon reference machine, warm local discovery should aim for p95 < 100 ms, cold MCP readiness < 1 s, and idle service RSS < 50 MiB without active Python/provider workers. Return ordinary tool envelopes within 2 s; long runs continue in the service.

These targets are hypotheses. Measure the whole installed/executing system, including workers and Tauri, before describing it as lightweight. If targets conflict with correctness, preserve correctness and revisit the target.

## Reliability gates

Use a controlled fake provider for faults rather than paying for destructive repeated API tests. Crash at boundaries: after intent write, after remote success, before local result commit, after commit before reply, during artifact rename, and during handoff submission.

Verify duplicate starts/submissions, concurrent claims, revoked bindings, missing files, checksum failure, daemon restart, loop item recovery, and client termination. Test the actual scheduler/store/bridge path, not only pure state-transition functions.

Do not claim exactly-once external execution. Claim only the verified local deduplication behavior and provider-specific idempotency/reconciliation support.

## Stop or simplify criteria

After the vertical slice, consider extending an existing runtime if the new engine offers no practical integration advantage. Reduce scope if process authoring dominates cost, users repeatedly need unsupported free-form control flow, or the trusted-script mode cannot meet the intended deployment threat model.

Do not start a marketplace or elaborate UI to compensate for an unproven core workflow.

## Local operational analytics slice

[Connection metering and analytics](metering.md) records actual attempt usage where available, estimates only reviewed configured units, and keeps unknown values and provider-reported amounts distinct. It includes retained fixture runs and does not observe calls made by the agent client itself. These views are operational evidence, not the matched direct-agent/first-authoring/reuse economic experiment described above. No savings claim or complete invoice cost follows from this dashboard.
