# Design-package validation

## 2026-09-21 — Repository overview, attribution and evaluation planning

Replaced the chronological README with a product overview, architecture diagram,
local demo/installation paths and explicit development status. Updated contributor
instructions and desktop navigation names; added issue/PR templates, source and
design credits, planned visual slots, an evaluation protocol, a results registry
and a report template. Licensing remains undecided by owner instruction; repository
visibility was read as private and was not changed.

Documentation/full-schema validation passed for 83 Markdown files, 508 local links
and 34 JSON files. New linked heading anchors, code fences, icon source and issue
template frontmatter passed focused checks. Independent review refined client
availability gates, provider-call ceilings and separation of unsupported evaluator
fixtures. Source adaptations were checked against existing pinned provenance and
license texts. No runtime/UI implementation, installed app or workflow file changed;
no application build, runtime suite, product evaluation, paid call or hosted Action
was run for this documentation pass. Earlier entries below retain their historical
scope and are not current release claims.

## 2026-09-16 — synthetic public sample content

The previous personal example was removed from current source, documentation, contract fixtures and the simulated inspector. The public sample now uses generic document review. Full schema/fixture checks pass against the executable local-review process, and the mockup's inline JavaScript passes `node --check`. The earlier visual observations below describe the earlier revision; no fresh visual or real-client result is claimed for this content change.

2026-09-14. This record concerns repository artifacts, not a running APR implementation.

## Checked

- The standard-library checker passed local Markdown/HTML link checks, duplicate HTML IDs, JSON parsing, fixture identity consistency, bounded expansion, and DAG/reference checks.
- Full validation with `jsonschema==4.23.0` passed the draft 2020-12 schemas, the original illustrative manifest/input, run envelopes, and submitted artifact-result shape.
- The interactive HTML mockup loaded without recorded browser console errors.
- Desktop layout was visually inspected. A narrow viewport override of 390 × 844 was also inspected; the browser reported a 375px layout width excluding its scrollbar and no horizontal overflow.
- Process navigation, text filtering, empty search results, process detail, run detail, connection detail, and project switching were exercised.
- Pausing disabled the simulated result action; continuing restored the saved waiting state. Simulated result submission reached the completed package state.
- The handoff dialog opened, the copy control reported success, Escape dismissed dialogs, and focus returned visibly to the initiating control.
- The temporary viewport override was reset. The preview was left in the initial waiting state.

The UI records, artifact names, connection status, and execution transitions are simulated. No image, review ZIP, provider connection, or real run was created.

## Reproduce the document checks

```sh
python3 tools/check_docs.py
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-check.txt
.venv/bin/python tools/check_docs.py --full-schema
```

The full validator is optional for reading the repository. The lightweight checker is not a complete runtime manifest validator.

## Still unverified

Every runtime acceptance criterion in the product spec: service durability, real client setup/handoff, process execution, secret confinement, operation authorization, external effects/reconciliation, distribution, and savings/footprint benchmarks. These belong to M0–M5.

## 2026-09-16 — General routing design revision

Broadened the canonical product specification, routing/composition proposal, Jev adapter proposal, evaluation plan, inspector requirements, and cross-device handoff to cover workflows across domains. The product document is v0.2; existing machine-readable contracts remain v0.1 and do not implement the extension.

The standard-library documentation checker and `git diff --check` passed after this revision. The new illustrative Choice request was checked against the public TypeSafe documentation and parsed as JSON locally. No provider inference was executed. No runtime code, machine-readable contract, or HTML mockup changed; the earlier full-schema and visual checks were not repeated for this documentation revision.

## 2026-09-16 — Public repository and complete implementation plan

GitHub returned `visibility: PUBLIC` and `isPrivate: false` after the requested visibility change. The existing remote and commit history were preserved. Before publication, a bounded scan of 53 historical text blobs found no matches for the selected private-key/token/credential-URL patterns or personal absolute paths. The remote had one branch and no Actions runs, issues, or pull requests at that inspection.

The implementation package contains 52 uniquely identified work packages, 15 planned verification suites, and 16 product acceptance criteria. Local checks verified identifier uniqueness, the acceptance/suite inventory, closed fenced blocks in new core planning documents, document links, existing JSON fixtures, and whitespace. Licensing remains undecided.

These are publication and documentation checks. No Rust runtime, client adapter, provider feature, inference call, CI workflow, installer, or planned runtime suite was implemented or executed in this pass. Machine-readable schemas and the HTML mockup are unchanged.
