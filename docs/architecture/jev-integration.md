# Jev integration proposal

Research checked 2026-09-15; design broadened 2026-09-16. **Optional adapter implemented; live quality/evaluation remains unverified.** `apr-provider-jev` now translates Choice/Score/Noul through the protected broker under the `jev` feature. Synthetic contract and controlled HTTP tests cover its wire behavior. No live Jev account or paid inference call was used. See [the execution contract](execution.md) and [validation log](../logs/runtime-validation.md).

The user has confirmed that Jev must remain optional. The [provider contract](decision-providers.md) requires exclusion from default builds/setup, explicit connection enablement, and core verification without inference credentials or calls. The [JEV work packages](../plans/implementation.md) are an optional track rather than a prerequisite for the general runtime.

## Recommended role

Add a provider-neutral decision interface to APR, with TypeSafe Jev as its first candidate adapter. The agent builds and revises processes; the runtime executes, composes, and records them; the decision provider handles bounded semantic choices where ordinary rules are inadequate. These choices can route a whole assignment, select a child process, or determine the next branch of a complex workflow. Their bounded output does not restrict the overall product to simple workflows.

| Component | Responsibility |
|---|---|
| Codex / Claude Code | Author processes, generate creative content/code, resolve difficult exceptions, use native tools |
| APR and ordinary code | Scope, access checks, exact validation, process composition, parallel scheduling, control flow, artifact storage, limits, durable resume |
| Jev or another decision provider | Select among supplied eligible routes, classify state, estimate a yes/no proposition, or score against a rubric |

Keep this optional. A process consisting only of exact transformations needs no model call. Do not turn every filesystem check or arithmetic comparison into a Jev query.

## What is available now

TypeSafe's launch is dated September 15, 2026 and describes early access. Its published price is $0.042 per million input tokens, with no output-token charge. Its large speed/cost comparisons are vendor evaluations, not an APR benchmark. [Launch](https://typesafe.ai/blog/introducing-system-one-models-and-jev).

The public interface is a hosted `POST https://api.typesafe.ai/v1/systemone`, authenticated by Bearer key; examples use `jev-latest`. Requests contain a state plus named questions. The [quick start](https://docs.typesafe.ai/introduction/quickstart) gives the connection shape.

State is text or structured JSON. Several questions can share that state. No direct image-input contract was verified; an image caption is not equivalent to inspecting the image. [State documentation](https://docs.typesafe.ai/concepts/state). The documented shared state/question budget is approximately 32k tokens; an answer-dependent question needs a later request. [Question limits](https://docs.typesafe.ai/primitives).

Three outputs are documented: **Choice** selects a supplied option; **Score** evaluates ordered rubric levels; **Noul** estimates whether a proposition is true. Jev does not write code, prose, explanations, or creative prompts. [System One](https://docs.typesafe.ai/concepts/system-one).

Choice/Score include distributions and a derived `confidence` statistic; Noul returns a probability without that separate field. Confidence is not a literal guarantee that this answer is correct. [Confidence](https://docs.typesafe.ai/confidence).

The documented API exposes the response model, answers, and usage; 429/529 responses require bounded backoff. Stable snapshot identifiers, account limits, data retention, and self-hosted model availability must be verified before relying on them. No public local model deployment was established in this research. [API](https://docs.typesafe.ai/api).

## Routing complex work

The target is a general workflow capability across research, operations, software and creative tasks. The [routing and composition design](workflow-routing.md) owns how accepted decisions drive execution; Jev supplies one possible source of those decisions.

| Decision location | Example question | APR's next action |
|---|---|---|
| Assignment intake | Which eligible process fits this request? | Start the selected pinned process, or persist clarification/no-match work |
| Within a workflow | Which supported document type does this resemble? | Select the corresponding extraction/comparison path |
| Child-process selection | Which specialist process should handle this portion? | Validate inputs and start the chosen child with shared authority and limits |
| Worker/provider selection | Which configured implementation fits the stated requirements? | Apply capability, privacy, cost, and permission checks before dispatch |
| Review and exceptions | Does the supplied evidence meet this process's rubric? | Apply a versioned continue/review/stop policy; unresolved cases return to the agent |

For example, a research assignment can classify inputs, delegate several evidence-gathering tasks, compare the results, ask the agent to synthesize a memo, and route incomplete cases for clarification. Business operations, software delivery, and creative production can reuse the same mechanisms with different process catalogs.

A familiar multi-step workflow can therefore run with many small semantic decisions and few open-ended agent turns. A novel problem may still require substantial agent reasoning. Jev does not need to generate code or invent a plan for the combined system to orchestrate complex work. This proposed division also follows TypeSafe's documented pattern of code-owned control flow around bounded decisions. [Building with System One](https://docs.typesafe.ai/concepts/how-to-build-with-system-one).

Do not classify transport errors or uncertain external effects with a model when the connector's exact protocol determines recovery. A semantic prediction cannot establish whether a payment, message, or other write actually occurred.

## Implementation boundary

Register the optional `apr-provider-jev` adapter through the runtime's connection module; no Jev-specific MCP tool is needed. Keep the existing discover/run/inspect/resume surface. The Rust broker owns the credential and outbound request; generated process code receives structured answers and artifact references.

Proposed operation: `decision.evaluate` under the existing `connector` step kind. The selected connection binds the provider. Treat inference as a metered external read, not a pure deterministic function: a retry may incur another charge and produce a different answer.

A versioned decision specification contains:

- Input projection: exactly which source values leave the device, and their size limits.
- Question definitions, criteria, stable IDs, and a schema digest.
- Provider/model selection and connection binding.
- Per-run request/token budget, concurrency, timeout, and bounded retry policy.
- A separately versioned acceptance/escalation policy and its evaluation evidence.

Use one total deadline and attempt budget across any client-library and runtime retries. Do not accidentally multiply retry layers. Keep request/response bodies out of diagnostic logs; retain authorized inference evidence in the scoped artifact store.

Normalize output with a discriminated result type: `choice`, `score`, or `noul`. Preserve the provider's distributions and optional confidence fields rather than flattening them into one invented “accuracy” number. Validate keys, finite ranges, option membership, and distribution sums with a documented rounding tolerance. Missing, malformed, or timed-out responses take a distinct error/review path; they are not low-confidence predictions or an implicit pass.

An original, illustrative request shape:

```json
{
  "model": "jev-latest",
  "state": {
    "request": "Compare the two supplied proposals and prepare a brief of their differences.",
    "inputs": {"documents_available": 2, "structured_record_sets_available": 0}
  },
  "questions": {
    "next_process": {
      "type": "choice",
      "instructions": "Select the next action that best fits the request and available inputs. Choose needs_clarification if the intended outcome or required inputs are unclear. Choose no_match if the request is clear but none of the supported processes fits.",
      "criteria": {
        "compare_documents": "Compare two or more supplied documents and prepare a differences brief.",
        "summarize_records": "Summarize an available structured record set.",
        "needs_clarification": "Ask for missing context or required inputs before selecting a process.",
        "no_match": "Return a clear request that is outside this candidate process set to the authoring agent."
      }
    }
  }
}
```

This is sample data and documented request structure, not an executed request or measured result. The Choice request shape was rechecked on 2026-09-16 against the [official Choice contract](https://docs.typesafe.ai/primitives/choice). Route labels are mapped to approved pinned targets by APR; they are not permission tokens. Include the eligible candidate set when it fits the context budget, and only the necessary task state rather than a whole conversation.

## Required change to APR's current draft

The existing manifest has step dependencies but **no general conditional-edge, child-process, or decision-policy construct**. Adding a provider connector alone does not implement workflow routing or confidence-based escalation.

Start with a narrow deterministic `decision.policy_gate` operation: it receives committed predictions plus a pinned policy. It either completes with a typed accepted outcome or persists a review handoff using APR's existing request/receipt mechanism. If the reviewed outcome blocks continuation, stop that dependent path explicitly.

This requires specifying gate outcomes, transition rules, and resumed-result validation before implementation. Preserve both the original prediction and the agent's review/override; do not rewrite the inference history.

The target extension also needs declarative branches, pinned child-process calls, explicit skipped-step/join semantics, and limits shared across nested and parallel work. These are a core part of the general routing design, with a staged implementation in M3a. Do not introduce arbitrary executable expressions or pretend today's schema already supports this. No machine-readable schema or example manifest has been changed as part of this proposal.

## Reliability and policy

**Typed is not correct.** TypeSafe's no-hallucination comparison concerns schema conformance; it is not evidence that every business or creative judgment is true. Calibrate on our task, including ambiguous cases, before enabling automatic continuation.

**Probability is not permission.** A high score cannot grant a new tool, connection, project, or budget. Runtime grants and exact policy checks remain authoritative.

**Parallel questions are not statistically independent evidence.** Do not multiply their probabilities to claim a combined reliability without a justified model. Retain individual signals and evaluate the final route.

**Do not assume deterministic inference.** The vendor's consistency example even shows borderline probabilities crossing a decision threshold; its repeated queries include a changing irrelevant field, so it is not an identical-request determinism test. Preserve the accepted response for replay. [Consistency cookbook](https://docs.typesafe.ai/cookbooks/consistency_noul_cookbook).

**A pinned process does not pin a remote model alias.** Record requested and returned model identifiers, timestamps, state/question hashes, raw response, usage, and policy version. If a stable model revision is unavailable, mark it unresolved. Resuming a committed step reuses its response; a new evaluation is recorded as a new attempt. Cross-run caching and silently adopting provider changes remain disabled until explicitly designed and validated.

**Hosted calls change the data path.** APR can remain local while selected state is sent to TypeSafe. Request the relevant project/provider grant, enforce input projection and quotas, and verify retention terms for intended production data. A network/API failure must follow a declared wait/fallback policy; no silent switch to another paid provider.

## First experiment and release gate

Keep M0's deterministic core and manual handoff independent of Jev availability. Prove routing/recovery with rules and a fake provider, then test Jev's decisions in observation-only mode:

1. Build a small, versioned routing set across research, business operations, and software delivery. Include clear matches, ambiguity, missing context, absent candidates, and unsupported requests. Record acceptable outcomes and required escalations. An initial roughly 100 cases tests feasibility, not per-domain reliability.
2. Separate policy tuning from held-out evaluation; expand each domain's sample if error rates or uncertainty remain inconclusive. Do not infer a universal confidence threshold from the combined sample.
3. Compare exact rules where applicable, the current agent's judgment, and Jev over the same candidate set and state. Separately measure whether candidate retrieval includes a correct route.
4. Measure wrong-route rate, missed required escalations, unnecessary escalations, the fraction handled automatically, review workload, p50/p95 latency, and accepted-result cost. Then measure whole-workflow completion across composed routes. Include state/candidate preparation and any stronger-model fallback.
5. Audit a sample of high-confidence accepted cases as well as escalations. Reviewing only uncertain cases hides confident errors.
6. Test malformed output, overload, response loss, changed model identity, and restart after the response is committed.

Enable automatic continuation only when a task-specific held-out target is met. Thresholds are selected from evidence, not copied from a demo or treated as universal percentages. Revision of the questions, provider/model, state builder, or policy triggers reevaluation.

## Client authoring convenience

TypeSafe already supplies an agent skill for Codex/Claude Code and a Claude plugin path. It can teach the coding agent how to author questions; it does not supply APR's durable execution, scoped authority, or existing native-tool access. Review/pin that material when integrating, and keep runtime secrets out of the authoring context. [Official agent skill](https://docs.typesafe.ai/agent-skill).

**Recommendation:** make routing and composing reusable workflows a general APR capability; benchmark Jev as the first optional semantic-decision provider. Preserve the ability to run without it or replace it, and earn automation through task-specific and end-to-end evidence.
