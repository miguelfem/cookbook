# Connect a service once, approve its operations separately

Ask your project agent to inspect the installed `connections` guide, existing
bindings and every page of the shared tool library. It should assess the official
service MCP before choosing an API adapter. An existing connection in Codex or
another client does not transfer credentials to CookBook.

## Browser connection

An author-mode agent can inspect public authorization metadata:

```json
{"action":"connection.discover","resource":"https://service.example/mcp","scopes":[]}
```

Send that through `apr_manage`, including `workspace` for a global client connection.
The owner CLI equivalent is `apr execution oauth-discover --resource URL`. Neither
command registers a client, opens a browser, grants a tool, or reads private data.
A blank client ID selects a configured exact-issuer public client, then a hosted CookBook identity if the server advertises support, then advertised dynamic registration. The result identifies
`authentication_required`, `client_registration_unsupported`, or
`discovery_unavailable`. Unsupported registration is an integration gap for CookBook to solve. It must not
turn into a request for the user to register an OAuth app, edit configuration or
silently substitute a different credential.

For an agent-prepared connection, open **Access requests** and approve the
reviewed operation: the browser handles login, or one labeled field accepts the
required key. You do not need to learn OAuth or edit connection JSON.

To add a service yourself, open **Settings & access → Connect a service**, enter
its name and HTTPS MCP address, choose **Browser sign-in**, and select **Connect
service**. CookBook discovers public sign-in metadata automatically. Review the
service, issuer and scopes, then complete login in your browser. CookBook saves
the account and reads bounded tool metadata. No discovered operation is enabled.
Advanced settings retain manual metadata/client configuration for integration
work; it is not the expected end-user journey.

For a service that documents an API-key connection, choose **API key** and paste
the key in the password field. CookBook saves it once in the operating-system
credential store before listing tools. An agent-prepared request specifies the
credential type and header; manual setup defaults to a bearer token and offers
`x-api-key` / `api-key` under advanced settings. The key never belongs in chat.
If discovery fails, the account remains saved; retry with that account instead of
pasting the key again. Saving a key does not itself prove that the provider accepts it.

Browser sign-in expires after five minutes. If it expires, start a new sign-in
from CookBook instead of approving the old browser page. Cancellation and expiry
do not indicate a problem with the local credential file.

Keep **Automatic (recommended)** selected for MCP protocol. Metadata-only discovery detects a supported version and pins it into the proposed tool. Execution keeps that version; it never retries a tool with a different protocol. Auth failures, throttling, malformed responses and timeouts remain failures. If
sign-in succeeds but metadata discovery fails, the account stays saved in
**Tools & accounts**. Retry with the same OAuth configuration and select the saved
account; this also works after restarting CookBook. If several accounts match,
choose which one to inspect. **Sign in to another account** opens a fresh browser
flow. Account authorization still grants no individual tool operation.

If an older development build reports incompatible client metadata or an
authentication-method mismatch, update CookBook and rediscover the **same exact
endpoint** on **Browser sign-in** before retrying. Review the new configuration.
CookBook now prefers the standard Basic method when the provider supports it;
post-only and public clients remain supported. It does not change an existing
approval or fall back to a project key. Persistent provider rejection still needs
investigation; repeated registration is not an automatic recovery policy.

Select the needed operation, constrain its input schema and submit the proposal.
For a database read, pin the reviewed query and bound the business inputs; do not
grant caller-supplied arbitrary SQL. The agent can submit the same public proposal
with `connection.propose`. In **Access requests**, select the saved account and
approve the exact operation for **Only this workspace**, or explicitly share that
tool with **All projects**. A saved account can support later compatible proposals
without signing in again. Changed destination, native command, OAuth audience or
scopes require separate setup.

`apr_inspect {"view":"connection_contract","name":"BINDING"}` returns the
approved schema without credentials. Run one bounded read before declaring the
connection verified. Account metadata alone is not proof of current provider health.

## Let your agent prepare the integration

When a service needs a CLI, SDK or an API shape outside the direct JSON connector,
ask the project agent to read the installed `connections` and `adapters` guides and prepare a
reviewed adapter with `apr adapter prepare` and `apr adapter check`. It can write the source, test synthetic cases, diagnose the exact
runtime environment and assemble the access proposal using its authorized tools.
You handle login/key entry and review the actual access or software installation.
An unsupported OAuth client identity still needs CookBook integration work;
wrapping the same server in another process does not resolve it.

The shipped scaffold freezes source, spec, fixed command and declared dependency hashes; its readiness check never runs the target CLI or installs packages. The agent still writes the service-specific translation and tests its contract. A universal automatic
adapter installer is not shipped. See the [coverage plan](../plans/connection-coverage.md)
for current support and the remaining launch work.

## When an API adapter is appropriate

Record the missing MCP capability or required correctness guarantee. A local stdio
server is an adapter running with your OS privileges; it can still require a
project API key. Its MCP transport does not make it a hosted browser connection.
Include a nonsecret `credential_label` in new bearer/header proposals, such as
`Project API key for the archive service`, and explain the destination, authority,
operation and fallback reason. Never paste credentials into the agent conversation.

Older proposals keep their identity and grants. If a replacement is warranted,
prepare it alongside the original, validate equivalent results and recovery, then
register an explicit immutable recipe revision. Do not bulk-rewrite recipes or
discard idempotency, pagination, file hashes or reconciliation requirements.

## Reuse an account after an adapter update

An agent updating a local adapter must submit a fresh access request. In that
request's **Account** selector, choose **Review updated adapter** for the existing
provider account, then select **Review adapter update**. Compare the code pins,
command, destinations and operation permissions. Confirm the separate account
reuse checkbox and ordinary access checkbox before approving. You do not need to
enter the key again. This option is never selected automatically.

A different executable path, credential environment or authentication contract
requires separate setup. Changed arguments and hashes still need the exact review;
a label alone never proves provider identity. Where old source snapshots are
unavailable, inspect the updated source explicitly: comparing hashes does not
show which code changed. Reviewing metadata creates no grant and calls no provider.

## Account controls and limits

**Tools & accounts** lists saved accounts separately from shared tools. Disabling
one tool leaves the account available. Disconnecting an account stops future calls
through all its grants; already-sent effects may still complete. Revoke provider-side
access in the service’s own settings when needed. Backups omit vault credentials;
restore disables account metadata and grants until new owner review.

## Compatibility boundary

The simple browser/key journey is the product requirement for supported remote
services. CookBook must supply the integration details; a service that requires
DIY OAuth setup is not qualified for that journey.

| Service contract | Current support / evidence |
| --- | --- |
| Remote HTTP with discoverable OAuth and dynamic registration | Shared provider-independent implementation; native Supabase sign-in, saved-account reuse after restart and 23-tool discovery verified. Other providers require their own live acceptance. |
| Remote HTTP with bearer API key, `x-api-key` or `api-key` | One native password field; protected storage and exact-endpoint reuse. All three formats tested against synthetic HTTP providers, including failed discovery and restart. This does not qualify every live API. |
| Public remote HTTP | No-account metadata discovery; tool execution still needs approval. |
| OAuth requiring a pre-registered client or only Client ID Metadata Documents | Exact-issuer public-client profiles and hosted identity discovery/validation are implemented. Maintainers must configure and qualify a real public identity or provider registration; the bundled profile is empty until that happens. Ordinary users should not perform this setup. |
| Local stdio, custom authentication, legacy standalone SSE | Local stdio requires installed trusted software and explicit command review. Custom auth and standalone SSE are outside the simple remote connection promise. |

The supported wire versions are 2026-07-28, 2025-11-25, 2025-06-18 and 2025-03-26;
owner discovery selects a supported version automatically and the approved tool pins it. Advanced setup can still select an explicit revision. Draft 7 and Draft 2020-12 MCP service schemas are
validated without relaxing recipe schemas, endpoint binding, PKCE or permissions.

The current [MCP authorization specification](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/authorization/index.mdx)
prefers Client ID Metadata Documents and retains dynamic registration for backwards
compatibility. Before claiming wider browser support, maintainers must provide
and qualify the appropriate CookBook client identity or provider integration.
The first release can name the connections actually qualified on our own accounts;
it does not depend on recruiting external testers.

Supabase login and metadata discovery are separate from query/equivalence testing;
no database query was part of that acceptance. See the [validation record](../logs/runtime-validation.md).

No token-saving percentage follows from choosing MCP instead of REST. Measure
matched task quality, caller usage, provider usage and orchestration separately
using the [evaluation plan](../plans/evaluations.md).

## Maintainer client identity setup

CookBook's published identity is `https://itscookbook.dev/client.json`.
The [hosting source](../../integrations/oauth/hosting/README.md) is a static Vercel
project; the [bundled profile](../../integrations/oauth/client-identity.json)
supplies this default to future app builds. Existing packages built before this
profile was added need an owner-configured public identity or an updated package.

Use one stable HTTPS JSON address, for example
`https://cookbook-auth.example.com/client.json`. A subdomain on an existing domain
is sufficient. Static hosting must serve `application/json`, without a login,
redirect or browser challenge. It needs no database, account token or client
secret. CookBook receives browser callbacks locally; the static host does not
receive the user's authorization code. Keep control of this host as a release
dependency. Some providers still require their own public-client registration.

Prepare a reviewable hosting directory with the installed binary:

```sh
apr connection-clients prepare-hosting \
  --client-id-url https://cookbook-auth.example.com/client.json \
  --directory /absolute/new/hosting-directory
```

Publish only its `public` directory after the owner approves the real host and
publication. The generated document lists four fixed loopback callback ports;
CookBook must find one available. Verify the live HTTPS URL and exact document
before configuring the runtime with the sibling public identity file:

```sh
apr --state-dir /absolute/runtime connection-clients configure \
  --file /absolute/new/hosting-directory/client-identity.json
apr --state-dir /absolute/runtime connection-clients show
```

For ordinary installations, maintainers ship the verified identity in the bundled
profile rather than asking each user to do this. Optional
`pre_registered_clients` entries contain exact `issuer` and public `client_id`
values, never a secret. Configuration is separate from browser consent and tool
approval. The published document has passed public HTTPS and byte-match checks;
named CIMD provider sign-in/callback flows remain unqualified until tested.
