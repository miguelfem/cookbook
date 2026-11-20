# Optional computer use

APR scaffolds finite manifest 0.2 workflows and pinned Python helpers. The native macOS backend operates the shared physical desktop; the [isolated browser backend](../runbooks/background-browser.md) uses a private, leased headless Chromium session per run. Both use ordinary native effect steps, optional `jev.evaluate`, exact routing/join steps and durable caller requests. Recorded browser plans can repeat observed steps without model calls. There is no credential-bearing Python client or inherited agent browser profile. See [native setup](../runbooks/computer-use.md), [execution](execution.md), and [the implementation plan](../plans/computer-use-and-schedules.md).

## Observation, decision and action

Each iteration observes the allowed foreground macOS application's window, extracts bounded OCR text/coordinates and minimal accessibility metadata, and produces a finite action catalog. APR sends that structured state to either the configured Jev connection or a saved `computer_decision` caller request. The caller returns a choice and confidence; it must not execute the proposed action itself. Screen text is untrusted data, not permission or instructions for the runtime.

The native action step accepts only a current catalog choice with sufficient confidence. It rechecks application/window/focus identity, observation age and UI state before acting. Low confidence, unavailable capabilities, unknown targets, stale observations and session limits stop automatic execution and retain a caller-review path. The process permits 1–12 iterations and a 1-second to 1-hour elapsed budget. A changed screen is an observation, not proof that the requested goal succeeded.

The action catalog contains at most 22 text targets, four explicitly supplied non-secret text values and six controls, remaining within APR's current 32-choice provider contract. Text values are a frozen allowlist; there is no automatic writer model, arbitrary generated URL, shell command or provider fallback. Secure text fields are refused. Native observations omit accessibility field values and capture only the selected window; screenshots are temporary rather than published as artifacts. Extracted UI text and decisions remain in ordinary run state. With Jev selected, that projected text is sent to the configured hosted provider, so local execution does not imply local-only inference or data storage at the provider.

## Completion and explicit pause

By default the graph ends at a `computer_review` caller request. Choosing `done` does not mark the goal achieved. The caller verifies the result with its own available tools and returns observed evidence or a stopped outcome. APR does not obtain tools from Codex or Claude Code automatically.

An optional frozen `success_text` predicate supports independent completion: an exact OCR block in the allowed foreground window must match the supplied text, including a fresh observation after the last action. The output records that observed condition. The author must choose a condition that establishes success for their task; an arbitrary page heading is not proof that a business operation completed. If the condition is absent, the normal durable review remains pending. This permits unattended scheduled jobs for tasks with an explicit observable completion condition, while preserving caller assistance for unfamiliar cases.

## Recovery and authorization

Every observation and action is a distinct trusted-native step, with its own saved intent and no automatic retry. Native scripts are conservatively classified as writes. If execution is interrupted after dispatch, the run enters `needs_reconciliation`; the existing evidence-based reconciliation interface decides whether a result was observed or a no-effect retry is justified. Completed steps are reused after restart. No catch-all loop replays earlier clicks.

Scaffolding writes a new directory only. It never reads a screen, installs Python packages, grants interpreter/OS access, configures Jev or begins a task. Registration freezes the helper source and constant policy. The owner separately authorizes a pinned interpreter and, if selected, a protected connection. The default decision source is caller; default Rust builds have no Jev adapter or Python UI dependencies. The app allowlist is a helper check, not an OS sandbox: the existing trusted interpreter still has the owner's operating-system privileges.

Desktop jobs share the physical foreground application and cursor. Fresh observations reduce stale actions but do not provide independent concurrent desktops. The helper does not launch apps, unlock the machine or bypass OS permissions. Use a dedicated session for unattended work; unexpected focus/UI changes produce a stop or caller review. Per-event checks bound typing time, while paired key/button releases must complete even if the window changes after a press.

## Upstream reuse and evidence

macOS event posting, accessibility lookup and OCR block merging adapt Aaron Levin's [typesafe-computer-use](https://github.com/awlevin/typesafe-computer-use), commit `7d120f1944d0156773d5089b81f8e46d108a6c8d`. The complete MIT license and adaptation notes ship in every generated bundle and the [third-party notices](../../THIRD_PARTY_NOTICES.md). APR supplies the finite workflow, protected broker, policy checks and durable recovery. Upstream demo economics are not APR measurements.

Tests use synthetic screen backends, real temporary SQLite stores, process termination and a controlled provider. They establish orchestration and validation behavior; they do not establish native OCR/action accuracy, Jev model quality, arbitrary-app compatibility or token savings. Live desktop and provider qualification remain separate acceptance work.

### Review before an upstream update

The adapted worker is a deliberately pinned source snapshot, not a submodule or
silently updateable dependency. To inspect the public upstream without changing
APR, run `python3 tools/check_computer_upstream.py check --json`; it resolves
`main` to an immutable commit and downloads only the exact GitHub repository's
bounded archive. For a human review bundle, provide an immutable 40-character
commit and a new destination, for example:

```sh
python3 tools/check_computer_upstream.py stage \
  --revision cc7b5066ae1a07b5e3182e8f87a9b5b6dfdcffc1 \
  --output work/agent-bootstrap-20260921/upstream-review
```

The command refuses an existing destination, verifies the upstream MIT license,
records old/new revisions and SHA-256 values for the adapted source files and
license, and writes a unified diff. It does not execute upstream Python, install
dependencies, alter `computer_worker.py`, update saved recipes, or change the
provenance pin. Review and explicitly integrate any source change separately;
the checker performs no polling or automatic update.
