# Documentation index

Updated 2026-09-22.

| Document | Authority |
|---|---|
| [Start here](00_START_HERE.md) | Reading order and repository state |
| [Product](specs/product.md) | Evergreen product requirements and acceptance |
| [Design system](specs/design-system.md) | User-facing layout, interaction, copy, accessibility |
| [Runtime](architecture/runtime.md) | Implementation boundaries and recovery |
| [Scheduled jobs](architecture/scheduled-jobs.md) | Persisted triggers, immutable templates, no-overlap policy and recovery |
| [Computer use](architecture/computer-use.md) | Optional bounded UI processes, provider/caller decisions and per-action recovery |
| [Schedule setup](runbooks/scheduled-jobs.md) | CLI/MCP/inspector controls and optional login-service files |
| [Computer-use setup](runbooks/computer-use.md) | Credited scaffold, native preparation, independent completion and caller assistance |
| [Background browser](runbooks/background-browser.md) | Reference-derived browser recipes, isolated sessions, preparation and compatibility limits |
| [General workflow routing](architecture/workflow-routing.md) | Implemented rules, typed choices, branches, child runs and joins; limits and future extensions |
| [Classifier learning](architecture/classifier-learning.md) / [user guide](runbooks/classifier-learning.md) | Optional Other collection, category proposals and evaluated version changes |
| [Decision providers](architecture/decision-providers.md) | Provider-neutral interface, default account-free behavior, and optional Jev build/enablement |
| [Protocol](architecture/protocol.md) | Tool contract, identities, states, errors |
| [Security and configuration](architecture/security.md) | Trust boundary, grants, credentials, config |
| [Client integrations](architecture/client-integrations.md) | Client-specific adapters and compatibility |
| [Jev integration proposal](architecture/jev-integration.md) | Optional semantic-decision provider for general routing and review; evaluation plan |
| [Measurement](architecture/measurement.md) | Benchmark design and performance targets |
| [Connection coverage](plans/connection-coverage.md) | Built-in MCP/API routes, agent-prepared adapters and remaining launch work |
| [Evaluation plan](plans/evaluations.md) | Staged usability, reuse, model-choice, routing and improvement pilots |
| [Evaluation results](evaluations/README.md) / [report template](evaluations/report-template.md) | First internal client results, failed acceptance gates, unresolved providers and chart-ready metrics |
| [Repository visuals](media/README.md) | Screenshot, walkthrough and evidence-chart slots |
| [Credits](../CREDITS.md) / [third-party notices](../THIRD_PARTY_NOTICES.md) | Source adaptations, dependencies, design influences and retained licenses |
| [Source audit](architecture/source-audit.md) | External evidence and remaining verification |
| [Implementation plan](plans/implementation.md) | Complete work packages, prerequisites, sequencing, release and operational work |
| [Remaining implementation](plans/remaining-implementation.md) | Concrete interfaces, storage changes, execution sequence and acceptance for completing the missing modules |
| [Task tracker](plans/task.md) | Current milestone, completed design work, and implementation status |
| [Computer-use and scheduling plan](plans/computer-use-and-schedules.md) | Requested extension work packages and acceptance evidence |
| [Verification plan](plans/verification.md) | Failure scenarios, CI/client/provider matrix, and acceptance evidence |
| [Decisions](logs/decisions.md) | Accepted direction, rationale, unresolved decisions |
| [Design validation](logs/design-validation.md) | Document/schema checks and mockup inspection evidence |
| [Runtime validation](logs/runtime-validation.md) | Executable checks, crash boundaries, resources, and remaining limits |
| [Hosted checks](../.github/workflows/ci.yml) | Active macOS/Ubuntu build, lint, tests, schemas, synthetic outcomes and development packaging |
| [Hosted qualification record](logs/evidence/2026-09-16-hosted-ci.json) | Final successful source revision, platform results and retained earlier failures |
| [Codex schema compatibility](logs/codex-json-schema-compatibility.md) | Actual MCP schemas through the published Codex normalizer, regressions and client limits |
| [Try CookBook APR](runbooks/local-demo.md) | Open the real interface with fresh local sample data |
| [Local CLI walkthrough](runbooks/local-cli.md) | Build and run the implemented M0 fixture |
| [Scoped MCP setup](runbooks/client-integration.md) | Assignment attachments, shared skill, manual client templates, and current transport boundaries |
| [Cross-device handoff](runbooks/continue-on-another-device.md) | Reproduce current state and continue |
| [Local distribution and samples](runbooks/distribution.md) | Development packaging, explicit-prefix install/update/remove, and bounded resource/outcome harnesses |
| [Contributing](../CONTRIBUTING.md) | Contribution workflow, license status, and validation expectations |
| [Local review example](../examples/local-review/README.md) | Executable synthetic review and packaging fixture |
| [Contract fixtures](../examples/contracts/README.md) | Example API payloads |
| [Research triage](../examples/research-triage/README.md) / [Operations intake](../examples/operations-intake/README.md) / [Software verification](../examples/software-verification/README.md) | Synthetic v0.2 routing and caller-handoff fixtures with actual service outcome checks |
| [UI mockup](../design/inspector.html) | Interactive visual interpretation; simulated data |
| [Archived concept](archive/concept-2026-09-14.md) | Historical starting point; noncanonical |

Specs own requirements; architecture owns constraints and mechanisms; plans own execution; logs record decisions and verified outcomes; archive holds superseded material. If two documents disagree, resolve the disagreement rather than adding a third version.

- [Native execution and protected operations](architecture/execution.md)
- [Live local inspector](runbooks/local-inspector.md)

- [Visual workflows and compact agent access](plans/visual-workflows.md) — read-only diagrams, Jev tuning, measured agent surface, nesting and deferred desktop packaging.

- [Workflow capacity](runbooks/capacity.md): owner ceilings, legacy defaults, deeper routing and persistent demo setup.

- [Batch jobs](runbooks/batch-jobs.md): one input with several jobs, complete keyed results, pinned composition and bounded parallel native/API execution.

- [Packaged parallel-batch evidence](evaluations/2026-09-22-parallel-batches.md): frozen synthetic comparison, exact outputs, measured worker overlap, caller exchanges and native installation.

- [Configurable worker capacity](evaluations/2026-09-22-worker-capacity.md): native 1–64 worker controls, observed overlap/refill, failed pilots and installed package identity.
- [README candidate evaluations](evaluations/2026-09-22-readme-candidate.md): actual E01–E05 outcomes, assisted setup, client usage, provider uncertainty and failed routing gates.

- [Local analytics and connection metering](architecture/metering.md): optional usage extraction, fixed-point estimates, provenance, scoped views and explicit billing limitations.

## Recipes and desktop

- [Recipe authoring and execution-boundary review](architecture/recipe-authoring.md); [executable review examples](../examples/execution-boundary-review/README.md).
- [Connecting services](runbooks/connecting-services.md).
- [Reusable tools and stored accounts](architecture/connection-library.md).
- [Manual recipe improvement](architecture/recipe-improvement.md); [installed agent guide](../integrations/knowledge/recipe-improvement.md) — bounded evidence, frozen controlled evaluation, and explicit version adoption.

- [Install and first project](runbooks/first-project.md) — normal installation, agent setup and an account-free first recipe.

- [Recipe connections, cost analysis and owner approvals](architecture/recipe-connections.md).
- [Tauri desktop build and setup](runbooks/desktop.md).

## Desktop 0.2 completion

- [Current macOS/Linux execution, billing and update contract](architecture/macos-linux-completion.md).
- [Completion work and qualification boundaries](plans/macos-linux-completion.md).
- [Local native verification and packaging](runbooks/local-verification.md); publisher signing is separate.

- [Local verification and packaging without GitHub Actions](runbooks/local-verification.md).
