# Implementation tracker

## Prepared MCP calls — 2026-11-19

The prepared batch path uses the existing client connection and version pin.
Added output-only and receipt modes for finished runs and corrected the workspace
connection message to skip discovery on known recipes. The warm evaluation
measures client time separately from runtime execution.

## Prepared profile batch — 2026-11-19

The v0.2 manifest accepts an empty item-key pointer so a prepared recipe can take
a scalar list directly. Added a fictional email-to-profile example and measured
serial versus parallel execution and Codex reuse. The evaluation records the
full results and separates runtime speed from agent token and cost outcomes.

## Agent capability and workflow-design guidance — 2026-09-23

Clarify installed capabilities through an overview map, a focused
`workflow_design` MCP guide and the short skill. Teach operation ownership,
migration of existing instructions, reuse of combined helpers, and accounting
for model work inside handoffs. Preserve real quality checks, authority and
prepared-run brevity. This slice changes guidance and discovery only; it does
not implement automatic trace compilation or change production recipes.

Verification and delivery status are recorded in the
[validation log](../logs/runtime-validation.md). Source changes do not refresh
an already installed skill or a running MCP process.

## Reference-first workflows and background browser — 2026-09-22

Implement reference-first recipe authoring, safe admission/worker diagnostics, a complete Jev graph and isolated background browser reuse. Preserve successful agent evidence, convert established browser clicks to individual checked native steps without model calls, and keep caller/Jev for residual judgment. Native desktop remains separate with exclusive operation coordination; retained native failure is not declared fixed without new proof.

Implemented, independently reviewed, packaged and installed. The
[qualification report](../evaluations/2026-09-22-reference-first-browser.md)
records four exact packaged outputs, seven real-browser probes, required local
checks and unchanged live state. The installed skill and MCP guide carry the
reference-first contract. Redirects/login reuse remain unsupported, the original
native failure remains unqualified, and model savings still need a matched study.

## Current slice — configurable continuous workers and README evaluations — 2026-09-22

Expose owner-selected global/local/per-connection worker limits (default four,
bounded to 64), persisted with revision checks and read-only agent discovery.
Replace slowest-member waves with continuously refilled workers while preserving
durable reservations, source/grant authority, cancellation and crash uncertainty.
Protect live claims against other runtime openers, serialize OAuth refresh instead
of all requests, and use scoped aggregate family status for large composed waits.
Verify native settings/status and 32/64-worker exact-output workloads. Then freeze
the tested package and run the README evaluation suite with prepared setup outside
reuse measurements; preserve older failures, unknown charges and label provenance.

Implemented, independently reviewed, tested, packaged and installed. Required
checks passed: 410 default and 453 fault-injection tests, 46 UI tests, strict
lint and full-schema docs. The [capacity report](../evaluations/2026-09-22-worker-capacity.md)
records native policy controls, preserved state and 192/192 corrected fixture
outputs at limits 4/32/64, including two earlier failed pilots.

The [README studies](../evaluations/2026-09-22-readme-candidate.md) are recorded
through their stopping rules. First-use script authoring failed; assisted E02
prepared execution passed 9/9 but showed no speed/total-token benefit. Provider
uncertainty stopped E03 and Other learning; Jev routing failed its quality gate.
Reviewed E05 regression controls passed. These remain launch/evaluation blockers,
not unfinished implementation of the worker controls.

## Current slice — composed waits and bounded workers — 2026-09-22

Keep short composed batches within the existing MCP wait budget while returning
actionable handoffs promptly. Add bounded overlap of ready native/API work across
independent steps and child runs. Preserve serialized state changes, frozen grants
and sources, shared reservations, per-item concurrency, complete output, effect
uncertainty, cancellation and recovery. Validate actual overlap and one-call
composition using disposable state, then build/test/install the development app.

Completed and independently reviewed. Required checks passed (398 default and
440 fault-injection tests), and the exact packaged comparison passed all 18
batches with complete outputs. Every new-build batch returned in one call; native
overlap reached four. Native UI and installed-state preservation passed. See the
[recorded results](../evaluations/2026-09-22-parallel-batches.md). Actual model-token
savings and full-client speed remain unmeasured by this follow-up.

## Current slice — batch execution and reviewed E05 — 2026-09-22

Verify the installed app's one-request job batching and actual concurrency,
separating agent-exchange reduction from parallel worker execution and token
savings. Run E05 with fresh source-disjoint expectations explicitly reviewed by
the owner. Preserve the prior blocked study, keep evaluation and fixture-only
adoption/rollback in disposable state, and dispatch no provider calls or effects.

Completed the [installed batch check](../evaluations/2026-09-22-batch-execution.md)
and [reviewed E05 controls](../evaluations/2026-09-22-e05-reviewed-improvement.md).
Direct foreach returned combined output in one call; composed child work needed
later inspection. Native/API dispatch was serial at that checkpoint. E05 produced the
five expected outcomes, preserved pins and completed fixture-only rollback with
zero provider/effect dispatch. Original failed harness assumptions and read-only
recovery are documented. No runtime change or rebuild; no token-saving claim.

## Current slice — prepared token efficiency — 2026-09-22

Apply the owner's approved persistent Codex run trust and verify its actual client
effect. Separate cached context, new input, output/reasoning, review and internal
recipe-provider usage; unknown billing remains unknown. Reduce recurring guidance
and unnecessary success metadata without dropping required output, provenance,
errors, handoffs or authority checks. Measure a prepared multi-step workload
against a competent reusable direct script, with setup separated and exact outputs.
Keep the previous pilots and distinguish a later trust diagnostic from concurrent
matched measurements. No uncertain provider retries or quality shortcuts.

Implemented approved trust, compact success receipts and reduced on-demand skill
loading. The [normal-project comparison](../evaluations/2026-09-22-e02-token-efficiency.md)
did not establish savings: 9/12 exact outcomes, with CookBook Luna blocked by
proof formatting and CookBook Sol reloading its skill on each reuse. The repair
preserves identity checks while accepting benign JSON formatting, makes project
workspace requirements explicit and excludes unchanged runs from authoring-skill
triggers. Required checks and native installation passed. Broad token savings
remain unproven; quality failures remain in the original record.

The installed repair passed a separate four-turn Luna/Sol regression: complete
outputs and page coverage in every case, with one run call and no skill reload or
review on each prepared reuse. Previously exposed cases and absent new direct
controls keep this result separate from comparative savings.

## Current slice — complete prepared execution path — 2026-09-22

Reduce repeated MCP context without widening accepted inputs, make Codex recipe
trust an explicit reviewed installation option, and measure successive turns in
an already prepared conversation. Keep every quality field, output limit,
workspace check, grant, code pin and revocation boundary. Native review must make
the tool-wide scope clear and invalidate stale previews. Preserve all previous
failed measurements; report default policy and optional trusted policy separately.
A faster local worker is not evidence of a faster client journey.

Implemented, independently reviewed, tested, packaged and installed. Schema
equivalence held for 1,620 cases; the native opt-in preserves existing approval
defaults and all runtime authority checks. The
[prepared-conversation study](../evaluations/2026-09-22-e02-prepared-conversation.md)
passed all nine exact outputs. CookBook Luna and Sol requested at low effort
finished within 2.2% and 1.6% of direct Luna's aggregate time; both missed the
first-reuse target. Observed client approval review accounts for most of the
CookBook tool interval. The owner subsequently approved the optional live Codex trust setting; it is
applied and the [later diagnostic](../evaluations/2026-09-22-e02-trusted-prepared.md)
observed zero run approval reviews. That non-interleaved follow-up does not replace
this original failed target or isolate its latency effect.

## Current slice — prepared recipe call overhead — 2026-09-22

Return small completed recipes in the original MCP start call through a bounded
wait on the independent service. Keep long work durable, all scoped authorization,
strict argument parsing, request-key idempotency and response limits. Clarify the
required inspection discriminator and reuse of known recipe contracts. Verify the
protocol and rerun a small frozen prepared-reuse comparison; preserve the previous
failed pilot and separate worker, MCP and complete client timings.

Implemented, tested, packaged and installed. The fresh
[one-call pilot](../evaluations/2026-09-22-e02-one-call.md) returned all three
complete recipe outputs without inspection. Warm MCP calls took 103–121 ms;
full fresh-client wall remained 25.2% above direct Luna-low. The next investigation
is recurring client context and dispatch latency. At that checkpoint, stronger
models and successive calls within one prepared conversation were untested;
the later study above records those results separately.

## Local credentials and persistent Python approval — 2026-09-22

Replace per-use OS vault access with private TOML storage, as explicitly selected by the owner. Migrate existing keys through a separate owner action; preserve account identity, grants and pending requests. Add native workspace Python review and make identical approval idempotent. Rebuild/install the tested native package and verify with disposable state. Preserve uncertain provider attempts; do not retry paid work as part of this repair.

Implemented, packaged and installed with local development signing. Six saved
credentials migrated and account/grant/request/run/effect rows remained unchanged.
Native disposable Python approvals and runs passed. The live workspace grant still
awaits owner review. The separately recorded prepared-reuse pilot passed exact
outputs but failed its speed target; client overhead remains unfinished work.


## Connection compatibility completion — 2026-09-22

Include owner-reviewed saved-account reuse for native adapter updates. Keep exact
automatic matching, bind the exception to proposal/account/review digests, and
verify source-only updates, different destinations/commands, stale/revoked
accounts, repeated approval and preservation of unrelated state. Exercise the
pending-review boundary with synthetic data.

Implement metadata-only MCP version detection, configured hosted/public OAuth identities and an installed CLI adapter scaffold. Validate the installed agent journey and package with disposable state. Public identity hosting and named live account qualification remain separate, explicit release evidence. Preserve accounts and grants. The owner has since selected private TOML credential storage; the following current slice supersedes the earlier Keychain discussion.

## Recipes as reusable capabilities and connection coverage — 2026-09-21

Replace generic scheduled-task examples with concrete wrapper-replacement designs:
creative assets, document-to-action intake, composed service operations and explicit
model/Jev routing. Separate implemented primitives from ready-made integrations.

Teach the agent to prepare and test missing API/CLI adapters with existing tools,
while keeping authentication, installations and permission grants owner-controlled.
The [coverage plan](connection-coverage.md) records current limits and remaining
release work; broad compatibility is not declared implemented by this documentation pass.


## Simple connections and maintainer-run launch acceptance — 2026-09-21

Make the normal owner flow browser sign-in or one API-key field. Combine public
OAuth discovery with login, persist key accounts independently of discovery, and
reuse exact compatible accounts without broadening tool authority. Keep provider
integration gaps explicit; users should not register OAuth applications.

Strengthen README examples and optional Jev routing/Other-learning explanations.
The [launch evaluation plan](evaluations.md) requires our own available clients,
devices and accounts, with no recruited participants or outside reviewers.
Product studies remain unrun; local regressions do not substitute for them.


## Generic service connections and saved accounts — 2026-09-21

Follow-up: fix the reproduced Supabase registration-method mismatch generically,
verify the discovery-to-registration path against a minimal provider response,
and replace the local development app after focused connector/native checks.

Repair installed contract inspection, add public scoped OAuth discovery and native
dynamic client registration, and separate account storage from operation approval
and tool sharing. Preserve exact destination/auth/scope contracts and existing
recipes. Local fixtures and packages qualify implementation; live service login,
bounded read equivalence and matched token savings remain separate acceptance.

## Repository presentation and evaluation planning — 2026-09-21

Present CookBook through a plain-language README, a working architecture diagram,
clear development-installation paths, current contribution instructions and visible
upstream credits. Reserve honest slots for recorded visuals and measured results.
The [evaluation plan](evaluations.md) starts with first-use sessions and a small
matched reuse pilot; provider studies require separate account/spend decisions.
Planning and documentation are delivered in this slice. E01–E05 remain unexecuted;
project licensing remains undecided at the owner's request.

## Recipe improvement and cookbook design — 2026-09-21

Deliver a manually initiated, assignment-scoped improvement cycle: inspect retained
evidence, freeze a labeled evaluation policy, propose one immutable candidate,
evaluate it safely, and explicitly adopt or reject the exact report/version.
The first complete slice executes bounded deterministic recipes in disposable
runtime state. Unsupported scripts, providers, unrecorded responses and external
effects remain unevaluated; no production effects or implicit live calls. Preserve
contracts, required outputs, existing runs, schedule pins and selection conflicts.

Refresh the native interface as a cookbook: warm paper, dark ink, restrained rust,
clear recipe rows and a new book icon. Keep operational labels, approvals and
accessible navigation literal. Verify real disposable native state and a fresh
package; leave the owner's installed app and client settings untouched.

Implemented and locally validated, including the mounted final ARM64 installer.
See the [executed results and package identity](../logs/runtime-validation.md).
The evaluator remains deliberately bounded; live provider/model evaluation is separate.

## Optional classifier learning — 2026-09-21

Add installed classify/propose/evaluate recipes with opt-in Other collection,
scoped retained-run paging, add-only candidate definitions, independent held-out
validation and reviewed immutable registration. Deterministic logic works without
Jev. Verify the real runtime against a local provider fixture and the installed
MCP guide; no paid inference, automatic schedules or production claims. Results
are recorded in the [validation log](../logs/runtime-validation.md): local tests,
packaged MCP and native launch passed; live account quality remains separate.

## Agent-created projects and computer-use updates — 2026-09-21

Replace mandatory per-project setup with one user-level Codex/Claude connection.
The agent proves current-folder write access, creates/reconnects its workspace,
and includes its explicit handle on every MCP call. Preserve fixed attachments,
revocation, owner grants and concurrent-project isolation. Validate with disposable
homes/state and the fresh native package. Add an immutable, review-only upstream
computer-use checker; updating the adapted worker remains a reviewed source change.
No automatic execution/replacement of downloaded code or saved recipes.

Implemented and locally validated: one-time client setup, agent bootstrap, scope
checks, both ARM64 packages and native UI. See the [validation log](../logs/runtime-validation.md).
Live client/account and publisher release gates remain separate.


## Installation and agent onboarding — 2026-09-21

The prior 0.2/native/UI branch is merged to main at `1589ee8bd0c8931ba1f4efdc784d7e215f2c5c33`.
This continuation cleans stale setup instructions and exposes complete installed
authoring schemas/examples through MCP with a short project skill. It adds an
explicit Jev-enabled package option while preserving the default account-free build.
Verify an empty project's ordinary recipe, example admission, attachment revocation,
running-service feature reporting, managed client setup and a fresh native package.
Use disposable state. Publisher signing, paid accounts and full model-driven client
journeys remain separate gates. See [first project](../runbooks/first-project.md).


## Native macOS 0.2 handoff — 2026-09-21

Local ARM64 development completion has real Tauri-window evidence: isolated first
launch/setup, scoped owner approvals, recipe execution and costs, full-period
billing, independent work surviving desktop exit, saved-state reopening, update
preferences and an isolated App preview with protocol/result delivery and denied
network/native/parent access. The lead integrated independent runtime, native and
UI reviews, adding regressions for the failures actually found. See the
[validation entry](../logs/runtime-validation.md) and disposable-launch instructions
in the [desktop runbook](../runbooks/desktop.md).

The final local runner includes default, fault-injection and all-feature workspace
suites, strict lint, documentation/schema/distribution checks, JavaScript behavior
regressions and native desktop tests. Final source identity, executed results and
fresh installer checksums live in `work/handoff-20260921/verify-final` and
`work/handoff-20260921/build-final`; their reports must pass without source drift.
That baseline was subsequently committed and merged by the owner. No hosted Actions
or publication was part of the handoff.

Still unqualified: native-vault/live-account acceptance, real agent-client journeys,
paid-provider quality/economics, other host architectures, and production licensing,
signing/notarization or signed-update hosting. These do not invalidate the local
unsigned development scope and are not claimed as passed.

## Local-only continuation — 2026-09-18

With hosted Actions unavailable, use `tools/local_verify.py` and
`tools/build_desktop_local.py`. No new hosted jobs are required for source
transport or checks. Local evidence must name its source fingerprint and actual
platform; missing native prerequisites remain blockers, not inferred passes.
See the [local guide](../runbooks/local-verification.md).

## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](../architecture/macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

Updated 2026-09-16. The local implementation now includes the [computer-use and scheduled-jobs extension](computer-use-and-schedules.md), following the earlier [completion plan](remaining-implementation.md). The [validation log](../logs/runtime-validation.md) records executed checks; this tracker separates code from external acceptance.

## Implemented

| Area | Current result |
|---|---|
| Durable core and service | Rust CLI, SQLite migrations, private Unix IPC, recovery, scoped catalog, official-SDK MCP, bounded inspection and handoffs. |
| Authoring and configuration | Immutable source snapshots; defaults/project/assignment/run configuration frozen per run; fixtures and explicit catalog promotion. |
| Reuse and execution | Ordered keyed foreach, bounded trusted Python workers and binary artifacts, pause/continue/cancel/retry and durable receipts. |
| Effects | Named HTTPS JSON bindings, native credential backend, frozen grants, intent snapshots and evidence-based reconciliation after uncertain effects. |
| Optional Jev | Feature-gated Choice/Score/Noul translation and controlled contract tests; no mandatory provider dependency or paid fallback. |
| Client setup | Preview/apply/update/doctor/uninstall preserving unrelated settings; private backups and interrupted-operation recovery. |
| Portable demo | CookBook APR branding and a local launcher for fresh synthetic review/schedule state; documented source handoff to another device. |
| Inspector | Read-only process/run diagrams, lazy child navigation and family handoffs, narrow author-only Jev question/policy preview/publication, plus existing scoped controls and artifact previews. |
| State maintenance | Consistent hashed backups, restore with revoked authority, project relocation, pins, retention and managed-artifact quota. |
| Distribution | Development packaging, provenance/dependency notices, explicit-prefix installation/update/removal and recovery tests. |
| Capacity | Owner policy in SQLite migration 10, explicit new-authoring defaults 32/1024/10000, hard maxima 128/4096/100000, iterative graph analysis and route-aware work bounds. Verification is recorded in the latest validation entry. |
| Routing and composition | Manifest0.2 rules/choices, guarded branches/joins, frozen child graph/configuration, stable child identity and shared budgets. |
| Computer use | Optional macOS scaffold with bounded observation/action cycles, Jev or caller decisions, exact visible-condition completion and durable caller review/reconciliation. Upstream MIT code is credited and preserved. |
| Scheduled jobs | SQLite-backed once/interval/UTC-cron jobs; frozen templates, atomic occurrences, non-overlap, coalescing after downtime, scoped CLI/MCP/inspector controls and inert launchd/systemd setup files. |
| Hosted verification | Both macOS and Ubuntu jobs pass on extension source `f7faac0`, including default/all-feature tests, schema checks, synthetic outcomes and fresh development packaging. |

## Local completion evidence

The extension passes 204 default and 232 all-feature local tests, with one native-credential test deliberately ignored in each suite, and both hosted platform jobs. Coverage includes real SQLite/IPC/MCP/HTTP/CLI schedule tests, migration and crash recovery, restore/retention checks, and computer-use tests with fake desktop observations/actions and a controlled Jev endpoint. The [latest validation entry](../logs/runtime-validation.md) records exact commands, counts and limits. Live desktop operation and live Jev quality remain unverified.

The earlier baseline passed 174 default and 199 all-feature tests, strict lint, versioned schemas, 18 multi-domain outcome runs, deterministic packaging and distinct-binary installation/update/removal. Both baseline hosted jobs are green. Those measurements and browser observations describe that revision; they are not fresh performance or visual evidence for this extension.

The implementation plan is complete for the local development scope. The remaining items below require client interaction, provider evaluation, release authority or a product decision; they are not substitutes for missing local code.

## New product work requested 2026-09-17

The [visual workflows and compact agent access plan](visual-workflows.md) is corrected to visual inspection of agent-built workflows, narrow Jev decision tuning and nested process navigation. General manual visual construction is out of scope. Desktop packaging is explicitly deferred; an arbitrary Code Mode-like execution layer is separate future work.

## External acceptance and release gates

| Gate | Boundary |
|---|---|
| Actual Codex and Claude Code journeys | Managed settings, official-SDK MCP tests and a browser session pass. A real Codex model registered/discovered; starting was refused by its noninteractive approval policy. Complete client journeys remain unverified; Claude Code is absent on this host. |
| Live providers | HTTP fault contracts and native credential storage are tested separately. No paid Jev inference or production account operation has been performed. |
| Native computer use | Fake desktop and protected-provider contract tests pass. Live macOS permissions, OCR quality, application behavior and model-selected action quality require a separate bounded trial. |
| Outcome quality and economics | Synthetic fixtures demonstrate execution. Matched direct-agent/first-authoring/reuse studies and held-out decision quality remain unmeasured. |
| Licensing | The public source preview has no project license yet. Existing development packages remain local previews. |
| Signed release and platform support | Signing/notarization, Windows and stronger native sandboxing are separate work. Report only executed macOS/Linux configurations. |

The original [delivery plan](implementation.md) and [verification matrix](verification.md) retain full work-package descriptions. Do not restart completed modules to satisfy old M0/M1 status prose. Complete unverified gates with evidence and preserve the original local-first, scoped, protected-binding and explicit pause/resume requirements.

## Analytics/cost implementation slice — 2026-09-17

On the analytics-cost-foundation development branch: explicit provider-neutral numeric metering, frozen fixed-point price cards, transactional per-intent usage observations, scoped analytics inspection, and inspector analytics/connection-status sections. [Architecture and limits](../architecture/metering.md) distinguish usage, estimates, provider reports, unknowns, and page-only subtotals. Tests cover arithmetic, uncertain effects, reconciliation, deduplication, nested runs and scope. Verification results belong in the validation log; source presence alone is not execution evidence.

Still separate work: the desktop shell/installers, Windows runtime port, trusted owner credential-entry UI, outbound MCP client and consent flow, intervention inbox/readiness, whole-period spend aggregation, provider billing reconciliation, money-budget enforcement and real-client acceptance. This slice does not mark those items complete.


## Recipe/desktop implementation — 2026-09-17

Implemented on the same review branch: author-time request/response/constant metering with schema-digest analysis provenance; cost preview and scoped connection-contract inspection; migration-11 durable proposals and explicit owner approval/rejection; outbound tools-only MCP over HTTPS JSON/SSE or trusted stdio; pinned tool schemas and existing uncertainty/cost accounting; static readiness and attention view; cookbook UI language with protocol compatibility; owner CLI equivalents; a Tauri app with per-window capabilities, project/client setup, key entry and bundled independent runtime.

The [architecture](../architecture/recipe-connections.md) and [runbook](../runbooks/desktop.md) define supported scope and limits. Compilation, transport/runtime tests, UI observations and packaged app verification are recorded separately in the validation log. Still separate: OAuth browser authorization/refresh, signed release, Windows, always-on remote execution, generalized interactive MCP extensions, whole-period invoicing and monetary cap enforcement.

## Reusable tools and execution review — 2026-09-21

Implementation slice: reusable connection library and exact-contract reuse, owner-selected approval scope, existing-key promotion/reuse, project/global revocation and restore safety; installed execution-boundary review/audit/examples; virtual-environment command preservation. Validate with synthetic credential/runtime tests, installed MCP and native UI.


Reusable tools and execution-review implementation is locally verified. See the
[validation log](../logs/runtime-validation.md) for the suite, packaged MCP and
disposable native evidence. Updating the owner's installed app and live accounts
is a separate owner-controlled step.
