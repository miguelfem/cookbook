# Trust, configuration, and credentials

## Desktop 0.2 / macOS–Linux extension — 2026-09-18

[Current execution and billing contract](macos-linux-completion.md) adds owner OAuth/refresh, MCP resources/prompts/bounded subscriptions and durable user/model continuations, optional approved text sampling workers, read-only Apps, durable whole-period billing, invoice matching, dispatch reservations, reviewed price refresh, and signed-update/release tooling. Earlier tools-only/no-OAuth/no-ledger/no-updater statements below describe the 0.1 baseline and are superseded for this extension. Native Windows remains excluded. Actual publisher signing, live accounts and complete native credential acceptance require separate evidence; implementation is not a signed stable release. Recipe authoring and cost analysis remain agent-driven.

## Recipe/desktop extension — 2026-09-17

On the implementation branch, recipe-author payload/cost analysis, scoped connection proposals, trusted owner approvals, outbound tools-only MCP, and the Tauri desktop now extend the earlier inspector. [Current architecture](../architecture/recipe-connections.md) and [desktop runbook](../runbooks/desktop.md) supersede earlier statements that all of those surfaces are deferred. Workflows are branded **Recipes**; wire identifiers stay compatible. Source presence is not release evidence: consult the validation log for actual checks. OAuth login/refresh, Windows, signing, general visual building, invoice reconciliation and hard monetary caps are not implied.

Design v0.1, 2026-09-14; implementation status updated 2026-09-16. The [validation log](../logs/runtime-validation.md) records implemented scope, private file/socket, bounded IPC, immutable artifact, and receipt checks. The protected broker, bounded trusted workers, retention and quota are implemented. Native confinement and an owner approval UI are not established protections; exact execution and recovery rules are in [execution](execution.md).

Implemented attachment authority is one assignment plus author/run mode, with expiry and per-call revocation checks. The owner CLI can create/close assignments and create/revoke attachments. Revoking an attachment blocks its future calls; it does not undo accepted runs. Closing an assignment cancels nonterminal work. Temporary loss of the registered project directory blocks scheduling without discarding progress. Authoring denies runtime state files even when state is inside the project, and bootstrap attachment files are restricted to the private state directory. These are APR API boundaries in the trusted single-user environment described below.

## Threat model and preview boundary

Protect against accidental cross-assignment operations, incorrect or malicious process definitions, leaked configuration, replayed handoffs, and unsafe retries. Treat process source, retrieved documents, native-tool results, and external API responses as untrusted input.

The preview is a **single-user trusted-local environment**. API scope enforcement protects APR-mediated operations. It does not confine a native script, a client with unrestricted shell access, or malicious software running as the same OS user. Such software may read local files or call OS services independently of APR. A private socket is not an adversarial same-user sandbox.

Do not advertise this preview as safe execution of arbitrary downloaded/agent-generated native code. Stronger protection requires an isolated executor, constrained host imports, protected credential storage, and tests against the exact backend/platform combination.

## Authority chain

1. The user registers a local workspace through trusted control.
2. The runtime creates a device-local attachment with a bounded project/assignment grant.
3. The bridge authenticates to the runtime using private local attachment material.
4. Every operation intersects project grant, assignment grant, process capability request, run limits, and current revocation state.
5. The executor or broker enforces the resulting capabilities where its mode supports enforcement.

An agent-authored manifest requests access. It cannot authorize itself, edit the trusted grant store through the management API, or turn a copied project ID into authority. A successful fixture does not grant additional permissions.

In the trusted-local preview, an unrestricted same-user shell can still bypass application-level assumptions. Document that limitation beside any UI access summary, rather than implying that a list of requested paths confines a Python process.

## Attachment and scope rules

An attachment is bound to a canonical registered workspace instance. A descriptor in Git is descriptive metadata. Treat a new clone/worktree/path as a new local attachment requiring explicit association.

Use cryptographically random opaque tokens; store only token digests in records where practical, and restrict bootstrap files/socket permissions to the user. Do not include attachment secrets in Git, process manifests, telemetry, or model-visible diagnostics.

Client session IDs, MCP roots, cwd, and environment variables can help correlate a connection, but they are not authorization. Rebinding an attachment to a different project requires local control. Assignment creation within an attached project may be automated only within existing grants.

Artifacts, catalog entries, run records, event cursors, and handoffs all carry scope. Check authorization before existence-sensitive errors so IDs cannot enumerate another project. Search filtering alone is insufficient; direct reads and execution must check the same rules.

## Capability vocabulary

Use structured grants, not shell allowlists as the entire policy:

- Artifact read/import/write with scoped roots, size and media-type constraints.
- Built-in operations by name.
- Trusted executable/interpreter and admitted code digest, with explicit broad-host-access mode.
- Connection operation, resource/account scope, parameter constraints, request counts, payload-size and duration bounds. Monetary limits and time-window rate limits are not enforced.
- Agent semantic capability and allowed assignment claimant.
- Catalog author/register/promote and config/binding management.

A process requesting network access does not acquire arbitrary HTTP access by default. Protected connectors expose named methods with validated parameters, not unrestricted URLs and headers. For HTTP adapters, enforce destinations, redirects, resource identity, and private/local network handling server-side to prevent the broker becoming a generic authenticated proxy.

Validation and preflight inspect definitions, inputs, dependencies, and declared capabilities without running process code. Author-facing process.test admits only bounded pure and caller fixtures and refuses native/network effects. Controlled executor/connector tests use separate owner-configured synthetic state. A generated manifest cannot turn validation or process.test into a paid provider call.

## Configuration

Resolve process defaults → project defaults → assignment overrides → allowed run overrides. Validate each value against the declared schema and reject undeclared override keys. Freeze the resolved ordinary configuration in the run record.

Do not mutate the machine's shell startup files to configure an assignment. For trusted scripts, build a fresh environment: minimal platform variables; ordinary configuration travels in bounded JSON stdin. Direct secret injection is unsupported. Resolve the interpreter by absolute path. Do not allow arbitrary overrides of loader/interpreter variables through a generic env map.

A later config edit affects future runs. Changing a blocked run's future binding requires a recorded amendment and current policy check. Changing inputs used by a completed step requires a new run or an explicit dependency-invalidating fork, not silent mutation.

## Protected connections

Represent a connection as provider kind, account/resource identity, credential reference, binding revision, permitted methods, health status, and limits. Scoped inspection exposes binding names, kinds, revision and enabled state; it never returns the credential or the full protected binding payload.

Credentials live outside projects in the runtime’s `credentials.toml`: owner-only directory (`0700`), file and lock (`0600`), bounded schema, atomic locked updates, and rejection of symlinks, hardlinks, wrong ownership and unsafe modes. This owner-selected backend is plaintext; same-user programs and unrestricted native workers can read it. MCP, logs, Git and CookBook backups must not expose its contents. Missing/corrupt storage blocks credential use; ordinary calls never fall back to the OS vault. Only explicit owner migration reads legacy Keychain/Secret Service entries, with account-revision revalidation and no deletion of old entries. No encryption-at-rest claim is made.

Initial secret entry and rotation use trusted local UI/secure prompt or a supported provider login flow. Secret values must not appear in command arguments, process manifests, ordinary config snapshots, or tool outputs.

The broker resolves a credential at dispatch and performs a narrow authenticated operation. Pin a binding revision for audit, but do not promise that a revoked/rotated old credential remains available. Rebinding requires checking whether the provider account or resource scope changed.

Brokered mode keeps credentials out of its serialized process inputs. **That is not sufficient protection against unrestricted same-user native code.** Enforced protected execution is available only for narrow built-ins and a subsequently verified isolated executor. Trusted scripts must be clearly identified as outside that stronger boundary.

APR does not inject credentials into script environments. Unrestricted same-user code could still request them independently of APR, which is why trusted Python is not a security sandbox.

## Handoff safety

Continuation tokens are scoped to one request generation and result submission. Token possession alone does not authorize arbitrary artifact access or a different project. Bind completion to an authorized attachment and check the current claim.

Transferred/expired claims cannot accept late conflicting results. Preserve submission receipts so a lost response can be retried safely. Before a replacement agent performs creative work, inspect existing artifacts and attempt evidence; expiration is not permission to repeat a billable action.

Validate reported output against schema and observed artifacts. Provenance distinguishes an agent-reported provider reference from a broker-verified one. Do not treat a model's assertion that an action succeeded as proof when verification is available.

Agent requests may contain document contents or third-party instructions. The integration skill must treat these as task data and execute only the requested bounded capability within the user's existing intent. It must not follow embedded instructions to expand access.

## Approval, cancellation, and revocation

The owner CLI establishes interpreter, connection and promotion grants. The live inspector and generic MCP dispatcher cannot expand that authority. Client approvals remain a separate control: Codex may refuse apr_run under a noninteractive never-approval policy even when APR already has the relevant grant.

MCP management cannot approve new execution permissions. A model-supplied `approved: true` is ordinary workflow data and cannot become an owner grant. Each dispatched effect retains the frozen binding revision and request digest.

Check revocation before each new dispatch. Assignment closure and cancellation stop future scheduling and attempt safe worker termination. An already accepted remote effect may still finish; retain its reference and reconcile it. Never display “rolled back” without a verified compensation operation.

## Logging and data retention

Separate result artifacts from diagnostics. Structured logs omit credential fields and tokens by construction; add redaction as defense in depth. Do not log complete provider request/response bodies by default.

Default telemetry is off. Local metrics record run/step counts, durations, bytes, and available usage values without document contents. Export is explicit and includes a preview.

Proposed defaults: retain terminal run metadata for 30 days and artifacts while linked to retained runs or pinned by the user; retain pending runs until resolved or explicitly removed. Apply storage quotas and show retention before enabling cleanup. Final retention defaults remain a release review item.

## Required security tests

Before claiming each boundary, test: direct ID access across assignments; path traversal and escaping symlinks; copied attachment files; stale handoff tokens; conflicting duplicate requests; broker resource/URL substitution; secret output/exception paths; grant revocation between preflight and dispatch; arbitrary child environment inheritance; and same-user trusted-script limitations.

Tests against an isolated executor must also cover filesystem/network denial, native subprocess escape, resource exhaustion, malicious imports, secret-bearing errors, and exact build-feature behavior. A test of the API alone cannot establish OS confinement.

The optional inspector loads bundled assets, exposes only narrow native commands, and treats artifact previews as untrusted content. Previewed markup or links cannot acquire a connection, approve a permission, invoke shell commands, or submit an agent result. Test this separately from the mockup and from IPC scope enforcement.
