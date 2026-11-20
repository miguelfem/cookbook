# Native execution and protected JSON operations

## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

## Recipe/desktop extension — 2026-09-17

On the implementation branch, recipe-author payload/cost analysis, scoped connection proposals, trusted owner approvals, outbound tools-only MCP, and the Tauri desktop now extend the earlier inspector. [Current architecture](../architecture/recipe-connections.md) and [desktop runbook](../runbooks/desktop.md) supersede earlier statements that all of those surfaces are deferred. Workflows are branded **Recipes**; wire identifiers stay compatible. Source presence is not release evidence: consult the validation log for actual checks. OAuth login/refresh, Windows, signing, general visual building, invoice reconciliation and hard monetary caps are not implied.

The service owns execution, including the time after a client disconnects. Ordinary configuration is frozen JSON; connection credentials remain in the private local `credentials.toml` store. This document describes the implementation extension. The validation log, rather than this document, records which platform/provider combinations have actually run.

## Trusted Python

An owner explicitly grants a pinned Python interpreter to one assignment. A process declares one UTF-8 source file, its SHA-256, and a matching `entrypoint`. Registration reads the file relative to the manifest's directory using the same project boundary as artifact imports, verifies its digest and stores its immutable bytes. A saved version never rereads edited source.

Python uses isolated mode, a fresh environment and a private temporary working directory. It reads `{protocol_version:"0.1",input,config,artifacts}` on stdin and writes `{protocol_version:"0.1",output,artifacts?:[...]}` on stdout. `artifacts` in the request maps verified artifact IDs to private readonly snapshot paths and their references. Output declarations contain a name, flat filename and media type; an exact `{worker_artifact:"name"}` object in the JSON output becomes its immutable artifact reference. The step result schema validates the resolved reference, including `artifact_id`, `sha256`, `media_type`, and `bytes`. Symlinks, hardlinks, special files, traversal and undeclared placeholders are rejected. The combined input/output exchange is limited to 32 files and the smaller of the manifest allowance and 64 MiB. Generated files are captured after process-group cleanup and published before the output commits. Unused published objects remain unlinked and eligible for retention after the orphan grace period. The process must declare its output schema. stdin/stdout/stderr and elapsed time are bounded; limits cannot exceed the worker's hard ceilings. The executor kills and reaps its process group after completion, timeout or cancellation.

The runtime supplies its run identity as `APR_RUN_ID` after clearing inherited environment variables. This metadata binds leased browser sessions to the admitted run; it never comes from workflow inputs and does not alter the existing 0.1 stdin envelope. Pinned trusted native code still executes with the owner’s OS privileges.

This mode trusts native code with the owner's OS access. It is not an OS sandbox, and an interpreter hash does not freeze installed packages or shared libraries. Dependencies must be prepared by the owner; execution never installs them. An assignment grant applies to scripts authored in that assignment. Keep run-only and author attachments distinct when giving access to other callers.

An approved absolute Python virtualenv path is retained for execution: resolving its symlink into the base interpreter would bypass that virtualenv's installed packages. The executable target is checked against its approved digest before each launch. This check does not freeze packages or prevent a same-user path replacement between check and spawn. Outbound trusted stdio MCP commands follow the same path-preserving rule; their environment is cleared, and dependencies must already be installed in the approved environment.

Native steps declare `effect:"write"`, even for a script intended to calculate pure values, and `retry.max_attempts:1`. The runtime cannot establish that arbitrary native code is pure. Failure after dispatch becomes an uncertain outcome that requires an explicit decision before another attempt.

## Connections

An owner configures a named operation for an assignment: exact HTTPS URL, HTTP method, input/output schemas, credential scheme, timeout and maximum response size. Processes select that binding by name and supply a JSON body. They cannot inject headers, change the URL, request an arbitrary credential, inherit an HTTP proxy or follow redirects.

`http.json` supports JSON-body POST, PUT, PATCH and DELETE, and fixed-destination GET with an empty input object. The bounded transport validates resolved destination addresses and passes that same resolution to the connection, rejecting private and local destinations. Fault-injection builds can explicitly enable a numeric loopback HTTP test connection; normal builds reject it. A credential may be a Bearer token or an approved API-key header. Known credential echoes in decoded JSON are rejected before persistence.

macOS and Linux use a versioned `credentials.toml` file under the runtime state directory. The directory is owner-only (`0700`); the file and its lock are `0600`. Writes are locked, atomic and synced. Corrupt, insecure, linked or unsupported files fail closed. This is explicitly selected plaintext storage, not an encrypted vault: other programs running as the same user can read it. Ordinary dispatch never reads Keychain or Secret Service. An explicit owner migration can import legacy entries once, without changing account/grant revisions or deleting the old entries. It releases the runtime lock while waiting for OS consent, then rechecks account revisions before publishing; changed/revoked accounts abort the import. CookBook backups exclude credentials. Secret input arrives on stdin to the trusted owner setup command, never as a command-line argument. The database contains opaque references and public operation configuration, not credential values. Local native scripts still share the owner's OS privileges; the broker is a serialization boundary, not protection from malicious code running as the same user.

Each binding revision is frozen at run acceptance. Reconfiguration or revocation blocks future dispatch by older runs. The owner must explicitly rebind a paused/blocked run after reviewing the new configuration. Rebinding cannot discard an unresolved effect.

## Configurable continuous workers

The service has one state coordinator and one SQLite writer. The owner's persisted
worker policy defaults to four simultaneous external jobs and four local scripts,
with an explicit maximum of 64. Optional named-connection limits are shared across
assignments using that name; different aliases are separate limits. Recipe
`foreach.concurrency`, dependency order, grants and shared budgets still apply.

The coordinator commits intent and billing reservations under its short writer
lock, then releases the lock during external work. Database-free threads receive
immutable snapshots. Each completion is committed and its slot refilled without
waiting for the slowest sibling. Pure/control work and optional model-sampling
steps remain serial. Lowering a limit lets admitted work finish and constrains new
claims; it does not revoke accounts, cancel work or change frozen manifests.

The service owns a lifetime execution lock before runtime recovery. Direct owner
commands enqueue while that service owns execution. Per-attempt file leases stop
other runtime openers from recovering live attempts as abandoned. Lease inodes
remain private and stable; no per-attempt cleanup is added in this change.

Calls sharing an OAuth account can overlap. Only expired-token refresh holds a
cross-process, per-reference lock. The credential is reread after locking and
rotated with compare-and-replace; concurrent owner replacement or removal cannot
be overwritten. Normal API work holds neither the refresh lock nor the vault lock.

Every launched observation is settled, including successes beside failures.
Validated full outputs and effect receipts remain durable. Panic, process death,
ambiguous dispatch or failed result commit leaves the exact intent uncertain and
prevents automatic replay. Owner cancellation fences attempts, preserves uncertain
effects, and signals their workers; stale completion cannot resurrect them.

The [capacity runbook](../runbooks/capacity.md) describes native Settings, owner CLI
and scoped read-only status. These controls do not grant new execution authority.

## Attempts, recovery and reconciliation

Before calling a provider or launching native code, APR commits an intent with the run, step instance, attempt, binding revision and request digest. External execution happens outside the database transaction. A validated output and successful intent are then committed together. No transport helper retries.

After a crash or an ambiguous response, the intent remains `uncertain` and the run enters `needs_reconciliation`. This includes a crash after writing the intent but before sending the request: conservative uncertainty can require human investigation, but it cannot silently duplicate a write. Errors that the broker proves occurred before network dispatch are recorded as `not_sent`.

Reconciliation requires an intent ID and bounded observed evidence. The caller can supply a validated observed result, record confirmed no-effect and allow a new attempt, or cancel while preserving the uncertain history. A retry consumes another attempt and provider reservation; explicit reconciliation cannot evade the five-attempt ceiling or the run-tree budget. Cancelling does not claim to undo an external operation. Unresolved intents remain inspectable even in cancelled runs; reconciling them does not silently resume cancelled work. The authenticated fast path signals every active native worker in the requested run family; each owned process group is killed and reaped, then the coordinator records the durable control. HTTP dispatch observes its cancellation flag and timeout where the transport permits, but an in-flight request cannot be unsent. A separate coherent SQLite reader keeps inspection available, and the coordinator releases the writer lock during external execution.

## Optional Jev

Build the CLI with `--features jev` to enable `jev.evaluate`. The default CLI does not link this adapter. Enabling it does not create a connection or issue inference. The owner must configure a protected Bearer binding to TypeSafe's documented endpoint; each process explicitly supplies its model, projected state and named questions.

The adapter supports Choice, Score and Noul, validates exact answer keys and candidate membership, checks distributions/ranges and retains requested/reported model names and reported usage. Missing confidence or usage stays absent. Choice/Score observations can feed the ordinary routing gate; the adapter cannot grant permissions or select undeclared processes.

The wire contract was checked against the [TypeSafe API reference](https://docs.typesafe.ai/api) on 2026-09-16. The adapter performs one request through the same broker and accounting path. Contract fixtures establish serialization behavior, not live model quality, pricing, alias pinning or task accuracy.

Recovery of a Jev call accepts the raw provider response. The intent retains the bounded resolved request and its digest alongside the immutable binding snapshot; reconciliation validates the raw connection schema, re-applies the typed adapter against that original request, then validates the normalized step result. Reconciliation cannot revive a cancelled tree. Any other uncertain effect keeps the run in `needs_reconciliation`.

## Optional connection metering

A connection may include the public `metering` configuration described in [local analytics](metering.md). It is validated and frozen with the binding. Observations commit with effect outcomes, before any later workflow-level validation can discard cost visibility; unobserved outcomes remain unknown. Reconciliation appends caller-reported observations and analytics selects the latest per intent, never a duplicate charge. No metering configuration grants access, expands the MCP surface to owner setup, changes retries, or enforces a monetary budget.

Python owner approval also has a reviewed-hash entrypoint for the native UI. The expected hash is checked under the runtime lock. Reapproving an identical enabled interpreter preserves its grant revision; changing its path, identity or contents creates a new revision. Revoked grants cannot be silently reused.
