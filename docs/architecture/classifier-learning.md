# Optional classifier learning

2026-09-21. A reusable, agent-assisted workflow for evolving a Jev classifier's
category definitions. It is opt-in per classifier/assignment, with no background
learning, account activation, model retraining or automatic schedule changes.

The installed `classifier_learning` MCP guide contains three complete manifest
0.2 recipes and a bounded connection proposal. The short APR skill points to this
guide only when requested. All deterministic helpers are in the Rust core and work
without Jev; actual classification and evaluation use the optional protected adapter.

## Data and authority

The classification recipe freezes `learning_enabled` in its normal configuration;
false is the default and it is not a run override. Its pure `classifier.collect`
step copies only actual `other` observations into the run's learning output, after
validating the complete keyed foreach results. Known-category low confidence is
not converted into Other. Failed or uncertain runs never appear in the history view.

`apr_inspect` view `learning_cases` selects retained successful, non-fixture runs
of the exact registered template version in the attached assignment. Both catalog
access and cursors are scoped; ordering uses `(created_at,id)`. It pages whole
batches with run/config/question identities under the existing detail limit.
There is no new data store, subscription or independent learning ledger. Existing
run retention, revocation and backup/restore rules apply. Cases are sensitive,
untrusted input deliberately available to the authorized attached agent; the view
does not infer labels or dispatch anything. Start a fresh traversal for a later
review; runs finishing during pagination are not a guaranteed incremental feed.

## Proposal and evaluation

`classifier.review` checks the exact baseline digest and supported classifier
shape. A durable caller handoff sees up to 16 Other examples and the existing
question, with no held-out set. `classifier.candidate` accepts zero to four
additions, each supported by two unique supplied cases, preserving all existing
IDs/descriptions and changing only both copies of the category criteria. It admits
the resulting manifest and returns its immutable digest. Zero additions is a
successful `no_change` result. Arbitrary graph/connection/schema edits belong to
ordinary recipe authoring and cannot pass as a taxonomy-only change.

Evaluation is a separate recipe, started only after independently labeled held-out
examples are available. `classifier.evaluation_plan` re-derives the candidate,
rejects changes, duplicate samples and discovery/evaluation ID or normalized-text
overlap, and requires two examples for every category including Other. Cases are
bounded to 2000 bytes each and 24 KiB per batch; evaluation allows 32 examples.
Both arms execute actual protected `jev.evaluate` steps with the same model,
questions and frozen examples. Labels are not sent to the provider. There are at
most 64 calls and no automatic retries. Model aliases remain aliases; the report
does not claim to reproduce historical provider revisions.

`classifier.evaluate` rejects incomplete/duplicate/invalid predictions. A passing
report requires at least 90% candidate accuracy, improvement over baseline,
preservation of every baseline-correct decision (new labels map to Other for this
preservation test), and two correct predictions per category at confidence >=0.75.
Missing confidence fails the gate. Baseline accuracy is measured against the new
labeled taxonomy so correctly identifying a new category is an improvement.
The report records version/dataset/discovery digests, evaluator revision,
per-category counts and rejection reasons. These small-sample defaults are example
policy, not calibrated confidence or production-quality acceptance evidence.

## Activation boundary

The caller reviews the passing report and uses existing `process.validate` and
`process.register` on the exact candidate source, checking the expected digest
before/after registration. This is an authoring workflow gate, not a new authority
boundary: an authorized author can still create arbitrary recipes independently.
Registration adds an immutable catalog version; the caller explicitly uses that
version for new work. Existing runs, parents, schedules, grants and configuration
are not repointed. Narrow `process.tune_jev` invariants remain unchanged.

Read the [installed guide](../../integrations/knowledge/classifier-learning.md)
for exact agent steps and the [user runbook](../runbooks/classifier-learning.md)
for the optional product flow. Live model quality/cost requires separate evidence.
