# Complete the runtime beyond the first core

Implementation plan, 2026-09-16. Baseline: `65ee35d`, 69 default tests and 79 fault-injection tests. This document makes the remaining [delivery plan](implementation.md) concrete. It is an execution specification, not evidence that every item already works. The [tracker](task.md) and [validation log](../logs/runtime-validation.md) record verified status.

## Result and sequencing

The intended result is one headless Rust executable that an owner can install into a selected Codex or Claude project, then use to author and reuse bounded processes. The executable owns saved state, capabilities, artifacts, typed caller handoffs, trusted subprocesses and protected connections. An optional local inspector uses the same service. Neither the UI nor a client conversation owns execution. Jev remains an optional adapter; core setup and tests need no inference account.

Implement these increments in order, integrating each before its dependents:

1. Configuration and catalog authoring; managed client installation; a standalone bounded Python executor. These three modules can be implemented independently against the existing runtime.
2. Persisted foreach and consistent controls; integrate immutable script snapshots and trusted executor grants; add backups and retention.
3. Protected connection storage, dispatch intents, limits and reconciliation. Exercise the complete path with a local controlled HTTP server before any live account.
4. Versioned exact routing, policy decisions and pinned child-process composition. Add the optional Jev translator behind an explicit feature and binding.
5. A live inspector, package/install/update tools, compatibility diagnostics, multi-domain acceptance fixtures and reproducible evidence.

Missing client accounts, signing identities, release permissions or live provider access do not prevent local implementation. Record those external acceptance gates as unverified rather than substituting a mock result. Do not label a stable release until the required evidence exists.

## Interfaces and ownership

Tests continue to cross the agreed public interfaces: admission, Runtime/scoped calls, real CLI/MCP subprocesses, worker execution, connection dispatch, client installation and inspector service requests. SQLite and the filesystem are real temporary stores. A fake server is allowed only at the external provider interface. Assertions concern saved outcomes, identity, effects and observable files; they do not mirror private implementation helpers.

Keep `apr-core` free of client, provider, database and UI dependencies. `apr-runtime` owns state transitions and policy. `apr-mcp` translates typed tools and never schedules work. A client integration module owns settings merge/removal and diagnostics. The worker module owns process groups and bounded I/O. The connection module owns validated HTTP and credential retrieval, but cannot approve grants or retry itself. The inspector is an authenticated caller of those same methods.

Concurrent work uses explicit source ownership. One writer assigns database migration numbers and shared schema changes. Start with migration 3 for configuration and migration 4 for expansion; add later migrations only after those integrate. Existing version 1 and 2 fixtures must upgrade with preserved histories and receipts. Keep legacy default manifests and owner CLI calls compatible unless an explicit versioned change requires rejection.

## Configuration and catalog

Ordinary configuration is JSON, never a secret vault. Resolve manifest defaults, then project values, assignment values and allowed run overrides. Merge by top-level replacement, including replacement of nested objects; this avoids surprising partial merges. Validate the complete resolved value against the process config schema. Only explicitly allowlisted top-level keys can be supplied per run.

Add `{ "config": "/json/pointer" }` to bindings. The resolver receives the frozen config, not the current mutable assignment. Persist the complete resolved config, its digest, explicit run overrides, and source revisions when accepting a run. Start deduplication binds the process, input and explicit overrides. Repeating an accepted key returns its original frozen config even after project defaults change; changed explicit overrides conflict.

New persistent state:

- Process-keyed project and assignment config values, with monotonic revisions.
- Frozen run config, digest, supplied overrides and source revisions.
- Explicit promotion grants linking source and target assignments in the same registered project.

Author mode adds `config.read`, `config.set`, `process.test` and `process.promote`. Assignment config updates replace that layer's override object. Project updates and promotion grants are owner CLI operations. A model-supplied target ID alone cannot authorize promotion. Promotion copies the same immutable version into the authorized target catalog; it cannot mutate existing runs or add capabilities. Fixture testing queues a real scoped run without silently registering the draft or enabling external spend. Preflight still executes no process code.

Acceptance: precedence, nested replacement, invalid schemas/keys, frozen config after edits, same-key replay, changed-override conflict, unauthorized promotion, edited source, and fixture side-effect restrictions through public calls.

## Managed Codex and Claude setup

Provide preview, apply, update, doctor and uninstall. The installer accepts the selected project, client, executable and private attachment path. It never reads the attachment into generated settings or emits its contents. A preview contains only APR-owned proposed paths/entries and expected digests. Apply checks those expectations before writing; a changed file requires a new preview.

For Codex, merge one server table into the project's TOML and install the shared skill under the chosen project skill directory. For Claude, merge one server object into project MCP JSON and install the same skill body in its documented directory. Preserve unrelated settings and TOML formatting where practical. Use a format-preserving TOML library rather than string concatenation. Active paths are local; the source templates remain inert.

Keep ownership records and private originals under runtime state. Lock installer operations; validate project-relative targets and reject substituted symlinks. Use bounded reads and atomic replacement. If a crash occurs between files, the saved operation record must support an idempotent retry or safe recovery. Updating or uninstalling compares the owned entry and skill bytes with the last installed values; user changes become a conflict rather than being deleted. Remove only owned keys/files; never replace the entire user config with an old backup.

Diagnostics distinguish executable found, client found, configuration present, attachment usable, service connected and actual client policy unknown. A successful `mcp get` is configuration evidence only. Do not disable workspace trust or client approvals. Different conversations sharing a client config also share its assignment; automatic chat isolation is not inferred from cwd or title.

Acceptance: fresh and preexisting settings, paths with spaces, stale previews, user-edited owned entries, same-command replay, interrupted writes, missing binary/client, denied attachment, and safe removal. Then run the actual discover/author/run/handoff/reconnect journey in each available client with recorded versions.

## Trusted scripts and bounded expansion

Python is opt-in. Owner control pins an absolute interpreter identity and explicitly grants trusted native execution to an assignment. Inspection of an interpreter does not grant permission. Snapshot admitted script bytes with their declared digest; later edits to source do not alter an existing process. No runtime action installs dependencies or changes shell startup files.

The worker runs an argument array in a fresh private directory and process group. Use isolated Python mode and a fresh environment containing only platform essentials. Input is bounded JSON containing protocol version, input and ordinary config. Output must be exactly one protocol/result envelope. Enforce an absolute deadline and independent stdin/stdout/stderr limits. Poll I/O and cancellation; kill and reap the whole process group on timeout, cancellation or overflow. Public errors never include raw stderr, source or output. Dependency environments must be prepared explicitly; no claim of OS confinement is made for trusted code.

Foreach expansion freezes its input array, stable keys and order exactly once. Duplicate keys, excessive items, bad item types and undeclared item bindings fail before dispatch. Persist each item as an independently checkpointed step instance; each gets its own attempt and handoff identity. Aggregate only committed item outputs in the original order. A failed or pending item cannot be treated as a successful join. Serial execution may implement an initial concurrency limit conservatively; any parallel implementation must share the same reservations and fencing.

Pause stops new dispatch at the next safe boundary. Continue rechecks authority. Cancel stops unstarted work and terminates supported active workers without claiming to undo effects. Retry is explicit, preserves evidence and pinned inputs, and consumes a new bounded attempt. Accepted handoff receipts remain replayable after controls change state. A replacement claimant receives a new token generation; it does not trigger regeneration automatically.

Acceptance: real sleeping/noisy/malformed Python processes, leaked-parent-env sentinel, process-group descendants, interpreter/source mutation, queued and active cancellation, foreach restart after one item, repeated keys, item-limit rejection, multiple caller waits and stable aggregate hashes.

## Protected connections and uncertain effects

Connection setup is trusted owner control. Each connection contains a provider kind, public label/account identity, allowed exact operation/destination, output contract, timeout and response limit, credential reference, binding revision and revocation state. Secrets live in the platform credential backend; unavailable or locked storage blocks the operation. Never silently use a mock or plaintext store in a production build. Secret entry uses stdin/secure prompt, never a command-line argument. No secret is placed into a manifest, worker environment, tool result, event or backup.

The first generic transport is a narrowly configured JSON HTTP operation. The owner fixes the HTTPS origin and path, method and credential scheme. The process supplies only validated body fields; it cannot supply URLs, headers or arbitrary credential references. Disable redirects and inherited proxies. Reject unexpected destination changes and private addresses except an explicitly owner-created loopback test connection. Validate response status, size, JSON and output schema. Detect credential echoes before a response enters runtime data. Do not report provider bodies in exceptions.

Persist connection grants and frozen binding references per run. Before dispatch, verify current revocation and identity against the frozen reference. Rotation or reconfiguration cannot silently point an existing run at a different account. Reserve the logical operation and provider budget before network I/O, outside the SQLite transaction. Each intent records run/step/attempt, connection revision, request digest and stable logical key. Commit outcome, output/artifact links, usage and event together after the call.

A definitely-unsent failure may be retried under declared policy. After a request may have reached the provider, timeout, process death or malformed acknowledgement becomes `needs_reconciliation`; never blindly replay a write. Supported provider idempotency is explicit, not assumed from an `Idempotency-Key` header. Reconciliation is an explicit owner/caller action with evidence: accept observed result, record confirmed no-effect and allow a bounded retry, or cancel while preserving uncertainty. An adapter performs one bounded attempt; the runtime owns retry accounting.

Acceptance uses a local server that counts received requests, captures credential headers without exposing them to generated code, can return redirects/oversized/invalid/credential-bearing data, and can accept a write then close before replying. SIGKILL after intent and after response commit must not duplicate the effect. Real-provider checks are separate and require an explicitly configured account/operation.

## Rules, decisions and child processes

Introduce a versioned routing grammar rather than silently treating the draft baseline as already supporting it. Keep the first extension acyclic and finite. Exact rules compare bounded typed data to declared values and select only declared route labels. Conditional branches persist the accepted route before selecting steps; nonselected work is `skipped`, with explicit merged outputs rather than references to nonexistent values.

Child-call steps name immutable catalog versions, validated input bindings and output projection. Pin the reachable process graph at acceptance, reject recursion/cycles and enforce maximum depth and total children. Creating a child plus binding it to its parent step is one transaction with a stable dispatch key. A restart reuses that child. Its assignment authority cannot exceed its parent's; pause/cancel propagates, and waiting descendants remain discoverable with their own handoff identities.

Reserve agent/provider/child capacity against the run tree so nesting cannot reset a budget. Independent branches can interleave, but joins require every selected required child. Persist decision source, projected-state digest, candidate snapshot, route/policy version and observations. Replaying a committed decision never invokes a model again.

Decision sources are exact local rules, the ordinary caller handoff, and an optional provider. All return normalized typed observations and are checked by the same route policy. Missing confidence remains missing. `no_match` and `needs_clarification` are valid outcomes; do not force an unsuitable candidate. For small catalogs, use the complete eligible set; bounded retrieval must surface truncation instead of implying exhaustive search.

Acceptance: research comparison, operations intake and software verification use the same engine. Test skipped output access, wrong types, no match, uncertain decisions, child creation/commit crashes, child caller waits, recursion, shared-budget exhaustion and cancellation.

## Optional Jev

Keep the adapter outside the default compiled feature path. Enabling a feature alone does not configure a credential or start inference. Use the verified TypeSafe endpoint and Choice/Score/Noul shapes, with a named protected binding and explicit model selection. Keep request construction, strict response validation and policy acceptance separate.

Validate answer keys, choice membership, probability bounds/distribution sums, and optional confidence without inventing fields. Retain requested/returned model identity and provider usage as reported. No implicit paid fallback, remote alias pinning claim or cross-run inference cache. Contract tests use a local fake endpoint. Live conformance, data terms/pricing and multi-domain held-out evaluation require provider access and separate evidence; they never gate the account-free core.

## Live inspector, backups and packaging

The inspector reads actual service data and exposes scoped run/config/event/artifact views plus declared controls. It must distinguish stopped service, disconnected caller, waiting request, paused work, and uncertain effect. Use bundled assets and a narrow authenticated transport; no remote scripts or unrestricted native command bridge. Render artifact text as text, never trusted markup. Secrets have trusted setup flows outside agent-visible payloads. Keep keyboard focus, accessible labels and reduced-motion behavior explicit. The existing HTML mockup remains labeled until replaced by this live path.

Backups use SQLite's backup interface under runtime ownership and include every referenced immutable artifact plus manifests, frozen source and saved workflow state. Store a checksummed inventory and verify it before restore. Backups contain private run data and must stay private and outside Git; omit handoff bearer files, client attachment files and platform credentials. Restore into a new absent destination, validate versions and integrity, invalidate attachments and disable execution grants. Preserve waiting requests and completed results, but require explicit handoff renewal before submitting a caller result. Interrupted native or network effects remain in `needs_reconciliation`; restore must never infer that they are safe to repeat. Reattaching and approving a new binding are separate from explicitly rebinding an existing run. Never overwrite a running store.

Retention defaults to preview. Referenced artifacts of active/pending/retained runs and imported assignment roots are pinned. Deletion requires explicit selection, lock ownership and rechecking references before unlinking; never race an uncommitted publication. State quotas and orphan cleanup report what can be removed, preserving waiting work. Diagnostics export metadata/codes and public resource identities, omitting payloads and secrets.

Package a default executable that excludes optional provider adapters. The Python launcher and bundled inspector can share that executable; Python requires a separately prepared interpreter and owner grant, and the inspector starts only when requested. Produce checksums, lockfile-derived dependency inventory, supported-platform manifest and reproducible build instructions. Installer/update records own only installed APR files; state deletion is a separate explicit operation. Signing/notarization, hosted CI permissions and a second-machine/Linux qualification must be reported from actual execution, not implied by a script. Select APR's license with the owner and preserve upstream notices before calling the package a complete open-source release.

## Completion evidence

For each increment record exact commands, dependency/features, test counts, failures corrected and remaining limits. Run focused behavioral tests while changing code, then workspace formatting, strict lint, default and fault-injection suites. Add feature-enabled Jev contract tests without credentials and a default dependency/egress check. Verify live UI interactions against temporary real state. Exercise install/update/remove in disposable projects before touching selected user settings.

Maintain reproducible account-free fixtures and an outcomes benchmark harness that records actual calls, bytes, elapsed time and reported usage. Unknown token usage stays unknown. Cost-saving claims require matched direct-agent, first-authoring and reuse runs at the same accepted quality; do not extrapolate from a small model's advertised price or from the runtime's binary size.
