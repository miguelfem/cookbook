# macOS/Linux execution, billing and desktop extension

Implementation 2026-09-18. Desktop version 0.2.0; the existing APR runtime, MCP tool names and recipe manifest versions remain compatible. [Validation](../logs/runtime-validation.md) distinguishes executed tests, native builds and unqualified live accounts. Windows is excluded. This document supersedes the tools-only and no-OAuth limitations of the earlier 0.1 preview, not its consent or recovery invariants.

## OAuth: trusted login and refresh

Discovery prefers `none`, then `client_secret_basic`, then `client_secret_post`
among the provider's supported methods. Basic is the RFC 7591 default when a
registration response omits the method; preferring it over POST avoids rejecting
otherwise compatible minimal responses. Explicit method mismatches still fail
closed with a rediscovery instruction. Existing reviewed configurations and saved
credentials retain their exact method and digest; there is no silent conversion.

`Auth::Oauth` binds a credential reference to reviewed authorization configuration: issuer, authorization/token endpoints, public client ID, resource audience, scopes and issuer-response policy. Protected-resource metadata discovery accepts the reviewed resource or an explicit metadata URL, follows the advertised authorization server and requires S256 support. Preregistered public client IDs and provider-supported client metadata IDs remain supported. With an empty client ID, discovery selects an advertised dynamic registration endpoint and a supported token authentication method (`none`, `client_secret_post`, or `client_secret_basic`). Discovery does not register a client or authenticate an account. Empty scope input uses the protected resource’s advertised scopes; the owner reviews the resulting list before sign-in. The resource URL, including query parameters, must match exactly.

The trusted desktop window opens the system browser only after explicit consent. A native loopback listener is bound before registration or browser launch. Dynamic registration sends that exact redirect URI and PKCE is still mandatory. Returned client metadata must preserve the redirect, selected authentication method and requested scopes. Generated client IDs and any issued client secret live inside the protected token bundle; no client secret enters the public configuration. Authorization uses a random state and PKCE verifier, five-minute expiry, exact callback path/Host, duplicate-parameter rejection, state checking and optional required authorization-response issuer checking. Codes and tokens never enter JavaScript, ordinary recipe inputs, diagnostics or the repository. The CLI has the same flow with explicit `--open-browser`.

Tokens are stored only in the private local TOML credential store, bound to the reviewed OAuth configuration. Refresh occurs before dispatch near expiry. Scope expansion is rejected and a rotated refresh token must be persisted before continuing. A store failure blocks dispatch; there is no automatic fallback. The selected TOML backend is plaintext with owner-only permissions, not an encrypted vault. A lost or rejected paid request is not replayed merely because a new access token becomes available. Setup, credential rotation and rebind are distinct from execution approval. Successful owner-authorized sign-in saves an independent account in the local credential store before metadata discovery and without creating an operation grant. Metadata failure leaves the account saved. A short-lived native cache keeps only its account identity for metadata retry or a subsequent exact-config approval; it holds no token. Account disconnect disables all its grants; provider-side revocation remains in the service’s own settings.

Normal discovery/authorization/token transports require public HTTPS, reject redirects/inherited proxies and validate the actual connection destination. Literal loopback HTTP is compiled only for test fixtures. Current metadata parsing and callback/token tests do not imply compatibility with every provider's registration policy or a real user account.

## MCP capability profiles and durable requests

Tools, exact resources and named prompts share the existing `connector` / `mcp.call` broker path. A reviewed `McpBinding` pins one tool or feature contract. Discovery can list tools, resources or prompts; execution rechecks the relevant identity/contract. External descriptions and prompt/resource content remain untrusted data, not instructions to widen permissions.

MCP tool and connection schemas support explicitly declared Draft 7
(`http://json-schema.org/draft-07/schema#`) and Draft 2020-12; absence defaults to
2020-12. The original schema is pinned without changing dialect semantics.
Validation preserves bounds, local-only references and cycle rejection for both
dialects. Recipe admission and direct JSON HTTP schemas remain strict 2020-12.
Metadata account selection survives app restart and never grants a tool operation.

The explicitly selected 2026-07-28 protocol supports multi-round tool requests (MRTR). When a server returns `input_required`, the runtime stores opaque `requestState`, original business arguments, generation, binding revision and bounded input requests in migration 13. It pauses the exact run. Only the continuation's reserved protocol fields are added; a caller cannot edit the original business payload. Completed continuations consume provider-call and monetary reservations like other external attempts. No hidden write replay or protocol downgrade is introduced.

`apr_inspect` view `mcp_interactions` exposes bounded pending requests and worker status, never opaque server continuation state. The attached agent can submit actual sampling results through `apr_resume` action `mcp.respond`. Elicitation answers require the separate owner surface. Mixed requests can be answered in stages with per-key idempotent receipts: an agent's model result cannot also approve user input. Form inputs use the reviewed, supported flat schema; URL elicitation displays the exact public destination and requires the owner's explicit response. Neither a click nor an LLM assertion is proof that a remote effect finished. Sensitive/unsupported input types stop rather than bypassing consent.

Subscription features implement filtered, bounded 2026 `subscriptions/listen` windows. The initial acknowledgement must be within the approved filter and every event must carry the correct subscription ID and approved notification type/resource. A normal server end, an intentional window boundary and a transport gap remain distinct. Output explicitly says continuous delivery is not guaranteed. A durable schedule can reopen bounded windows, but this is not an indefinite background socket or a lossless event log; refresh authoritative state after a gap.

## Explicit bounded sampling workers

An owner may approve `mcp.sampling_worker` alongside a sampling-enabled interaction policy. This is an optional text-only worker profile, not a second chatbot, unrestricted tool-choosing loop or mandatory model dependency. It pins a separately approved direct HTTP connection, exact model, maximum tokens, request template and field mappings, plus response text/model/stop-reason mappings. The recipe must also explicitly request the worker connection in its capability list; a server cannot smuggle new model authority through a sampling request.

At most one job is dispatched per runtime service tick. Model requests become ordinary saved intents in the same run, share its root/provider limits and billing reservations, and never receive unrelated client context or credentials. A committed model result survives restart and is submitted once to the original continuation. Actual stop reason and model identity are validated, not fabricated. Text-in/text-out API shapes are configured explicitly; richer models require a separately reviewed adapter or agent handoff.

An uncertain model request stops for reconciliation. The trusted setup window exposes its intent and evidence-based observed-result / confirmed-no-effect / cancellation controls. Restarting does not repeat inference; a replacement attempt needs confirmation and still consumes the common budget. Without an approved worker, ordinary caller-assisted sampling remains available and incurs no APR-created inference by default.

## Read-only MCP Apps profile

A desktop preview may display self-contained MCP Apps from completed, authorized steps. The related tool's approved `_meta.ui.resourceUri`, exact `ui://` resource and server/transport identities must match. The app resource is `text/html;profile=mcp-app`, with bounded HTML and associated tool result. `apr_inspect` view `mcp_app` requires `run_id`, `resource_step_id` and `tool_step_id`; it reads existing results and never fetches, calls a tool or grants access.

The frontend requires explicit preview consent. Two one-use native custom-protocol documents (host and child) are served in a separate window with no Tauri capability entry or runtime attachment; navigation admits only those exact per-preview URLs, and ordinary recipe-window CSP stays unchanged. Within that window it uses an opaque-origin sandboxed iframe with scripts only, restrictive prepended CSP, no parent DOM/native command access and no network, nested frames, forms, popups, downloads or device permissions. Its small host bridge implements 2026-01-26 UI initialization, the selected tool result, bounded resize and teardown. Host tool calls, links and unsupported RPC are denied. Third-party Apps needing CDNs, external network or interactive host tools do not fit this deliberately read-only profile. An iframe is not an OS sandbox and does not establish general CPU/memory isolation.

## Durable billing, invoices and reservations

Migration 12 separates billing records from disposable run history. Every external attempt has a ledger identity, connection revision, run/root/assignment, timestamp, currency, reservation, observation and provenance. Retention cannot erase charges; new ledgers are included in normal backups. Prior retained effect records are backfilled without inventing missing prices. The accounting grain is the actual intent, including child calls, failed outputs, retries and sampling workers.

`billing_period` / `apr_inspect` view `billing` accepts a half-open `[starts_at_ms, ends_at_ms)` interval and aggregates the entire retained ledger for the attached assignment, not one UI page. Currency, missing/partial coverage, estimates, provider-reported amounts and owner-imported invoice amounts remain separate. These are alternative observations, not additive charge categories. Old `analytics` call-page subtotals stay explicitly page-only. Operational run metrics retain their original history-retention meaning.

Owner invoice import accepts a normalized `Invoice` with invoice/account/connection/currency/period identities and per-line exact `intent_id` plus decimal-string `amount_nanos`. Import is idempotent; conflicting lines or scope mismatches are rejected. An unpriced intent can receive a verified owner-imported amount without inventing its estimate. Imported data is explicitly owner-reported, not asserted to be authenticated directly with the provider. Provider account APIs can be ingested through separately approved recipes and reviewed into this format. Unmatched pooled charges, taxes, negative refunds and ambiguous cross-account allocation are not silently distributed to calls.

A `BudgetPolicy` declares one currency, a start/end interval, a decimal-string limit, and owner-reviewed `per_connection_max_nanos` reservations. Reservation and dispatch intent commit atomically before any external call. Unknown or unbounded connections cannot pass the policy by claiming zero. Pending/uncertain work keeps its reservation; confirmed no-effect releases it. Observed charges can increase the accounted amount and block subsequent calls. Concurrent recipes therefore cannot each reuse the same remaining allowance.

This is an enforceable APR dispatch budget under the approved per-call upper bounds, not a guarantee that an external provider can never bill more than those bounds. The owner must constrain request limits and rates accordingly. Existing unpriced history, out-of-band calls, subscriptions and inaccurate provider quotes prevent a universal invoice ceiling. No workflow can approve its own rate, currency, invoice or budget increase.

## Rate refresh

Author-time analysis of user-approved payloads/pricing remains the primary cost setup mechanism. It produces request/response/constant metering rules with schema and pricing evidence. There is no new model call per API response.

The owner can additionally enable a public JSON `PriceSource` per connection: exact HTTPS endpoint, JSON pointer to a compatible rate object and interval (at least one hour). The service fetches the configured source only when enabled, validates the candidate and creates a digest-bound rate review. Approval may change prices only: it rechecks the binding/source revision and cannot widen endpoint, model, schema, credential or execution authority. Changed rates affect new approved binding revisions; historical runs remain pinned. Unsupported HTML pricing pages and provider-specific tier rules require agent analysis and a new reviewed contract, not guessed extraction.

## Updates and native releases

Desktop 0.2.0 has trusted-window update checking and user-approved installation. A publisher public key and HTTPS update endpoint are compiled into release builds. Development builds with neither make no update request. Automatic metadata checks are opt-in, run at most daily while the app is running and never install automatically. Tauri's artifact signature verification remains mandatory, and its normal version comparison rejects older versions.

Before installation, a durable migration-14 maintenance gate blocks new dispatch and configuration mutation, requires active runs to be resolved, takes a private backup and stops the service. Download/verification/install failure leaves the gate paused for explicit owner recovery. An update cannot silently cancel pending work or resume a restored grant. Linux in-app replacement is for AppImage; Debian installations use their system package workflow.

`.github/workflows/desktop.yml` builds and lints macOS Apple Silicon/Intel and Ubuntu amd64 development packages with locked dependencies. The distinct manual release workflow is restricted to reviewed main and the release environment, checks an owner-selected LICENSE and matching version, and requires real publisher configuration. Compilation occurs before signing secrets enter the signing step. Apple certificate/keychain, notarization and updater signatures are verified as applicable. No fallback silently publishes unsigned bytes. `tools/release_manifest.py` assembles a static feed and checksums from selected signed bundles with architecture-specific paths; publication/hosting is a separate owner action.

Signing/notarization, publisher identity, real OAuth login, paid-provider accuracy and complete native credential journeys remain external qualification requirements until their actual checks run. Source code or a test token cannot substitute for them. There is no Windows implementation in this extension.

## Contract references checked 2026-09-18

- [MCP authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization), [MRTR](https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr), and [subscriptions](https://modelcontextprotocol.io/specification/2026-07-28/basic/utilities/subscriptions).
- [MCP Apps overview](https://modelcontextprotocol.io/extensions/apps/overview) and [Apps protocol types](https://github.com/modelcontextprotocol/ext-apps/blob/main/src/spec.types.ts).
- [Tauri updater](https://v2.tauri.app/plugin/updater/) and [macOS signing/notarization](https://v2.tauri.app/distribute/sign/macos/).
