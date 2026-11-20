# Reference-first solved workflow

This synthetic contract is a portable template for deriving a recipe from a
workflow that already succeeded. It is evidence to inspect, not an automatic
trace compiler. Replace every placeholder with observations from the accepted
workflow, or mark the field unknown. Do not claim a step succeeded because a
recipe was generated.

The example describes a general support-intake workflow: records are normalized,
deduplicated, assigned to a declared queue, and returned with an artifact
reference. The source and artifact references are synthetic and contain no
device paths, credentials, continuation tokens or personal project data.

Use this reference with the ordinary `process.audit` sidecar. Its `steps`
entries map to `ReviewInput.steps`; its `output` maps to `ReviewInput.final_output`;
and its call inventories remain author observations until measured. A new recipe
must pass validation, equivalence checks against the observed cases, readiness,
and an authorized run before it is treated as reusable.
