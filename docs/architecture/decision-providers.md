# Optional decision providers

2026-09-16. **Implemented contract plus future provider-interface design.** Exact policies are in `apr-core::routing`; caller handoffs and protected connector dispatch are in the runtime; `apr-provider-jev` is optional. Jev optionality is a confirmed product requirement. The concrete current execution path is documented in [execution](execution.md). This document owns the provider interface and enablement rules; [workflow routing](workflow-routing.md) owns control flow and [Jev integration](jev-integration.md) owns that provider's translation and evaluation.

## Three ways to reach a decision

| Decision source | Execution | Requirements |
|---|---|---|
| Exact rule | A built-in operation computes a declared outcome from typed inputs | Local runtime only |
| Calling agent | A normal durable agent step requests a structured outcome | An attached, authorized Codex/Claude session with the required capability |
| Independent provider | A brokered connector evaluates a bounded state/question request | Adapter included and enabled, configured connection, current grant, and available budget |

All three produce a typed outcome that a versioned policy can accept, stop, or escalate. Rules and agent handoffs do not call Jev internally. The runtime owns the accepted route and can execute it without a decision-provider abstraction being involved at every step.

The rules-only core operates offline after installation and dependency setup. Caller-assisted work depends on the selected client's availability. Independent inference is an explicit network operation. No inference account is required for first-run setup, the default examples, or core CI.

## Module interface and dependency direction

Pure routing policies live in `apr-core`; provider-specific wire translation lives in `apr-provider-jev`. The current synchronous, bounded `HttpBroker` performs one protected operation and the runtime commits its observation. A general asynchronous provider trait remains a future extension; it is not required by the implemented Jev path. The core does not import Jev types.

Proposed interface: `evaluate(request, context) -> DecisionObservation | DecisionError`. It performs one bounded attempt. The runtime owns durable attempt identity, retries, shared budgets, and result commitment. An adapter does not schedule child runs, grant access, mutate a process, or launch its own unbounded retry loop.

| Input | Required meaning |
|---|---|
| Request identity | Run, step, logical evaluation, attempt, definition digest, and policy reference |
| Projected state | Validated, bounded JSON materialized from authorized immutable inputs; preserve its digest |
| Questions | Stable IDs, kind, instructions, allowed options/rubric, and schema/version digest |
| Provider binding | Named connection and requested model/revision, resolved through the broker |
| Context | Deadline, cancellation, remaining operation/token limits, allowed destination/resource, and authorized connection handle |

Credential values never enter the process-visible request. The trusted broker resolves credentials at dispatch. Provider endpoints and permitted operations are configured by trusted code/bindings rather than supplied freely by generated scripts.

The first interface needs synchronous request/response inference only. Known asynchronous jobs remain part of the connector lifecycle; add decision-specific polling only when a real provider requires it. Avoid a speculative plugin framework before the test adapter and Jev adapter demonstrate the seam.

## Result contract

`DecisionObservation` contains the source/provider, requested and returned model IDs, original question IDs, validated answers, usage when supplied, timing, input/definition digests, and a scoped raw-evidence artifact reference. A remote model alias is recorded as an alias; it is not an immutable model fingerprint.

Answers are a discriminated union:

- **Choice:** selected option and provider-supplied option probabilities; optional provider confidence metadata.
- **Score:** ordered rubric levels and provider-supplied values/distribution; preserve the original rubric semantics and optional confidence metadata.
- **Noul / proposition probability:** probability that the specified proposition is true, without inventing a separate confidence field.

Validate against the adapter's declared support: expected question IDs, complete option membership, finite values, ranges, distributions, rounding tolerance, and response bounds. A provider unable to support a question kind fails capability preflight. Do not fabricate distributions for providers or callers that return only a selected label.

The policy creates a separate `DecisionResolution`: accepted outcome, review required, or stopped. Record its source as rule, caller, or provider. Missing/malformed results are errors, not an accepted negative answer or low confidence. A model observation and a later agent override remain separate evidence.

Errors distinguish missing capability/configuration, rejected authentication, invalid request, overload/rate limit, deadline, cancellation, malformed response, budget exhaustion, and uncertain dispatch. Map them to the stable tool error/state contract with a specific required action. Unknown charged usage stays unknown; response loss is not a free retry.

## Jev build and runtime optionality

The optional `apr-provider-jev` dependency is enabled by the implemented `jev` feature on `apr-cli`, forwarded through `apr-runtime`. Build the normal executable with `cargo build --locked -p apr-cli`; select `--features jev` explicitly when needed. Workspace-wide build/test commands intentionally include all member crates for adapter contract verification; they do not prove that Jev is linked into the normal CLI.

The normal binary includes the local browser inspector and trusted Python launcher. It does not bundle Python or require a desktop framework. Python execution requires an installed interpreter and owner binding; the inspector starts only when explicitly requested. These capabilities are optional at use time, while Jev is also an optional compiled dependency.

Cargo can combine features from multiple dependency paths, so validate the **resolved executable dependency graph**, not only one `default = []` declaration. Optional packages may still appear in Cargo.lock; that alone does not mean they are built or enabled. [Cargo features and optional dependencies](https://doc.rust-lang.org/cargo/reference/features.html).

Enablement has three distinct checks:

1. **Included:** this executable contains the adapter, or diagnostics explain how to install an explicitly selected build that does.
2. **Configured:** the user enabled a named connection and supplied credentials through trusted control.
3. **Authorized for this step:** the process explicitly requests it, current grants allow the projected data/operation, and limits permit dispatch.

Including an adapter never enables a network call, health probe, credential prompt, or onboarding requirement. Default distributions omit Jev. If provider-enabled binaries are distributed, label their included features and keep all provider connections disabled until configured. Never download an adapter or executable module silently at runtime.

An absent/disabled Jev adapter blocks only the process step that explicitly requires it. Other workflows continue. A declared caller/rules fallback can run within its existing grants; there is no automatic paid-provider substitution or silent change in data destination.

## Decision lifecycle

1. Resolve the pinned decision definition, candidate set, inputs, and authorized projection.
2. Check kind/model support, connection state, grants, and budgets before reserving an attempt.
3. Persist evaluation intent and reserve shared operation capacity. Materialize the bounded request.
4. Run one adapter attempt under the common deadline. Record known usage/dispatch evidence even on failure.
5. Validate the response; commit observation/evidence references and accounting together.
6. Apply the pinned deterministic policy; persist the accepted route or durable review request before any dependent effect.

Restart reuses a committed observation and resolution. An uncommitted or uncertain attempt follows its declared connector retry/reconciliation policy; replay is never an excuse to treat another hosted request as free or identical. Cancellation prevents new work and retains any returned usage/evidence. Do not erase a successful provider response merely because the requesting client disconnected.

No cross-run inference cache is enabled initially. Future cache keys would need input, questions, model revision, policy/data permissions, and retention semantics. Unresolved remote aliases make silent cache reuse especially inappropriate.

## Authoring and discovery

Process discovery exposes required decision capabilities and connection names without credentials. Preflight reports adapter-included, connection-configured, and authorized separately. A process author can choose a rules step, caller step, or provider step deliberately and can test it against fixtures before registration.

The default authoring skill begins with reusable code and ordinary rules, requests caller reasoning where needed, and considers an independent model only when configured and suitable. It must never turn exact file existence, arithmetic, or schema checks into inference by default.

The optional Jev authoring skill can help write valid questions, but is not required to install or use APR. Any bundled provider instructions must be versioned, reviewed, and clearly separate from the common client skill.

## Required verification

The [verification plan](../plans/verification.md) includes a default installation with no Jev environment variables, credentials, adapter package, or inference network access. It must complete discovery, process authoring, rules-based branches, child runs, and explicit agent-result submission.

Provider-enabled tests use a local fake endpoint for mapping, rejection, response-loss, and reconciliation behavior. They do not enable automatic provider retries. Live Jev tests are separately opted into with a spend bound; they are never required in a fork pull request or normal core release. Core behavior is not conditional on a vendor's availability or benchmark result.
