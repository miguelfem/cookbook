# Contract fixtures

These payloads illustrate the saved review handoff in the [local review process](../local-review/process.json). They contain synthetic data and no usable token, stored artifact, or live run.

1. [waiting-for-agent.json](waiting-for-agent.json) shows the service's saved request.
2. [submit-result.json](submit-result.json) shows a caller submitting an observed review decision.
3. [resume-receipt.json](resume-receipt.json) shows the committed acceptance receipt.

Run/request/step/submission identities align across the fixtures. A retry with the same submission key and output must return the receipt rather than perform the review again. A changed output under the same key must conflict.

An identical repeated response may mark `duplicate: true`; its accepted revision and result remain the original receipt. Resume completion does not guarantee the full process has finished: this fixture returns to `running` while the final package is still being assembled.

The [run envelope schema](../../schemas/run-envelope.schema.json) validates the common envelope shape. Handoff result validation uses the request's `result_schema`; authorization, token checking, ownership, and effect evidence require runtime checks and cannot be proven by a JSON payload.
