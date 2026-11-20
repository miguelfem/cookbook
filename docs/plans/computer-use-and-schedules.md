# Computer use and scheduled jobs

2026-09-16. Requested implementation extension. This plan owns the delivery work; the architecture documents and validation log will record the resulting contracts and executed evidence.

## Intended result

An agent can prepare an optional computer-use process, register it in an assignment, and run it through APR's existing durable execution. Jev selects bounded actions from structured UI observations. A separate native helper observes and acts; the protected Rust connection retains the provider credential. Unsupported or uncertain situations return a durable request to the calling agent. Jev remains optional and the default executable needs no computer-use dependencies.

An author attachment can schedule a registered process once, at a fixed interval, or with a UTC cron expression. The local service creates ordinary durable runs when jobs become due. Schedules survive client disconnects and service restarts. The machine must be awake and the service running; missed occurrences coalesce without a burst of catch-up runs.

## Implementation decisions

- Reuse the existing manifest 0.2 DAG, trusted Python executor, connector broker, routing gates, handoffs and effect intents. Generate a finite computer-use process; do not add recursive workflows or hide a whole session in one effect.
- Pin the upstream computer-use source revision, preserve Aaron Levin's complete MIT license, and document which code was adapted. Native observation/action helpers require owner-prepared dependencies and OS permissions. Scaffolding installs nothing and grants no permissions.
- Reject stale observations and targets outside the observed, allowed application. Bound steps, time, candidate count and output. Reobserve after actions; a model's completion claim is insufficient evidence of success.
- Retain one intent per native action. Interrupted actions keep the existing uncertain-effect reconciliation semantics; no automatic click replay.
- Schedules freeze the process, child graph, resolved inputs/configuration and execution binding revisions at creation. Later catalog/config edits cannot change accepted work. Credentials remain protected references.
- Persist schedule, occurrence and run linkage in SQLite. Atomically create each due occurrence and its run while advancing the next due time. Keep occurrence identity through ordinary run retention.
- Allow one active run per schedule, including paused, waiting, blocked and uncertain work. Pause/delete prevent future occurrences and do not pretend to cancel an accepted run. Assignment closure and revoked execution grants prevent further dispatch.
- A schedule is accepted independent work. Attachment expiry/revocation stops new management calls, while explicit schedule controls and assignment/execution authority govern already-created jobs. Restores pause schedules until explicitly resumed on the destination.
- Keep the same five MCP tools: add author schedule operations to `apr_manage`, and schedule inspection to `apr_inspect`. Add matching CLI commands and portable setup instructions.

## Work packages

| Package | Deliverable | Required evidence |
|---|---|---|
| CU-01 | Pinned upstream source, license and adapted helper | Provenance/hash inventory and complete upstream notice |
| CU-02 | Validated scaffold and optional native observe/action helper | Generated manifest admission; bounded candidates/actions; stale observation and denied-app tests |
| CU-03 | Jev decision and explicit caller fallback | Controlled provider/native fixtures; low confidence and completion verification; default build independent of Jev |
| CU-04 | CLI, instructions and runbook | Reproducible scaffold/setup; no automatic installs, grants, screenshots or live actions during tests |
| SJ-01 | SQLite schedule and occurrence records | Transactional migration, duplicate create/conflict, restart, closed assignment, scope isolation |
| SJ-02 | UTC one-shot/interval/cron dispatch | Deterministic clock tests, missed-run coalescing, no overlap, frozen templates and binding changes |
| SJ-03 | Scoped service/MCP/CLI controls | Real IPC/CLI/MCP creation, listing, pause/resume/delete and scheduled caller handoff |
| SJ-04 | Backup/restore/retention and documentation | Restored jobs paused, referenced artifacts retained, occurrence identity cannot replay after collection |

## Verification and delivery

Run focused module tests while implementing, then the repository's required formatting, strict lint, default and fault-injection workspace suites. Exercise all-feature Jev paths with controlled fixtures and run the documentation/contract checker. Do not use the user's desktop as a test target or call paid providers. Record live Jev accuracy and actual macOS GUI operation as separate unverified acceptance until those journeys are performed. Update the product spec, tracker, architecture, integration skill and handoff instructions; publish source changes to the existing public repository after checks pass.
