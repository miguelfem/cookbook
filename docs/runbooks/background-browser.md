# Background browser recipes

First solve and verify interactive website work in Codex's in-app browser (`iab`)
through `cua_repl`. Retain its inputs, actual actions, page changes and output with
the [reference template](../../examples/reference-first/README.md). This guide's
isolated Chromium backend is for explicitly requested reusable APR browser recipes;
it is not the agent's default browsing surface or an implicit fallback.
Derive and validate the reusable recipe against that evidence. CookBook does not
automatically record or compile conversations.

| Work | Execution | Shares your desktop? |
| --- | --- | --- |
| Known API/CLI operation | Approved connector/worker | No |
| Interactive agent website work | Codex in-app browser (`iab`) | No |
| Explicitly requested APR website recipe | Isolated headless Chromium | No |
| Unresolved decision within an APR browser recipe | Same recipe browser with caller or Jev choice | No |
| Native macOS app | OCR/accessibility worker | Yes |

The browser starts with fresh cookies/storage per run, has no user Chrome profile
or debugging port, and needs no Screen Recording/Accessibility permission.
It is trusted native Python code: destination restrictions are helper controls,
not an OS network sandbox. Isolating a full native desktop would require a separate
OS session/VM; CookBook does not provision one.

## Prepare once

When the owner explicitly requests the APR browser backend, the agent can create
a private Python 3.11+ environment, install the pinned
[requirements](../../integrations/computer-use/requirements-browser.txt), then run
`python -m playwright install chromium --only-shell`. Use that environment's Python
and approve it through CookBook's normal native-code review. Recipes never install
dependencies or grant themselves access. Select the actual absolute Chromium
executable when scaffolding; its SHA-256 is frozen and checked before launch.

## Preserve observed steps

Save a JSON action array **after observing these actions and results**:

```json
[{"action":"click","label":"Complete local review","expected_text":"CookBook review complete"}]
```

Create a new bundle with the installed CLI, substituting your reviewed paths/URL:

```sh
apr computer browser-scaffold recipes/browser-review \
  --start-url https://your-approved-service.example/review \
  --origin https://your-approved-service.example \
  --browser-executable /absolute/path/to/chrome-headless-shell \
  --recorded-plan reviewed-actions.json \
  --success-text 'CookBook review complete'
```

Register, preflight and compare the result with the reference through author MCP.
The source, origins, actions and expected conditions are frozen in the recipe.
Each click has its own durable effect intent and exactly one attempt. Recorded
plans choose only a unique freshly observed label, then check its visible
postcondition. Their successful path makes zero model calls. Missing/ambiguous
controls or failed postconditions return review. Page labels do not prove unchanged
business behavior: retain meaningful output checks before adopting revisions.

Omit `--recorded-plan` for caller decisions, or select `--decisions jev` with a
separately authorized connection. Interactive choices support observed clicks and
scrolling. This first helper does not expose free text, password entry, uploads,
downloads, arbitrary page code or persistent login profiles. Add required origins
explicitly. Unlisted destinations and **all HTTP redirects/Refresh responses** are
refused before delivery to Chromium: redirect-dependent flows are not yet supported.
Use a reviewed final URL; do not broaden the origin list to conceal this limitation.

Sessions are bound to the runtime-issued run ID, frozen policy and initial goal.
One run cannot borrow another's saved handle. State survives individual steps,
but expires at the declared deadline with a bounded cleanup grace. Cancellation
prevents new APR actions; the lease closes a browser whose cleanup was skipped.
Restart never silently recreates a lost page or repeats previous clicks. An
interrupted action requires evidence-based reconciliation.

Evaluate reference solving/authoring separately from recurring reuse. Compare
complete results with the original agent workflow and an equivalent prepared
script. Zero internal model calls is an execution property; dollar and whole-client
speed savings still need matched measurements.

See [Playwright context isolation](https://playwright.dev/python/docs/browser-contexts)
and [headless installation](https://playwright.dev/python/docs/browsers).
