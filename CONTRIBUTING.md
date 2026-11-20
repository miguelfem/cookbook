# Contributing to CookBook

CookBook is a development preview with a Rust runtime, native desktop packages and Codex/Claude Code integrations. Start with the [local demo](docs/runbooks/local-demo.md) or [desktop build guide](docs/runbooks/local-verification.md). Current qualification and limitations live in the [validation record](docs/logs/runtime-validation.md).

The project license remains undecided. Review [Credits](CREDITS.md) and [Third-party notices](THIRD_PARTY_NOTICES.md); dependency licenses do not choose terms for CookBook's original code. Repository access is currently limited to collaborators.

## Useful contributions

- Clearer first-use instructions and reproducible bug reports.
- Focused fixes with a test for the behavior that failed.
- Synthetic recipes demonstrating a real repeated task.
- Reproducible [evaluation reports](docs/evaluations/README.md), including negative and inconclusive results.
- Accessible screenshots, recordings and diagrams using the [visual contribution guide](docs/media/README.md).

Discuss a large feature or architectural change in an issue first. For a bug, include the app/source revision, platform, smallest safe reproduction, expected behavior and actual result. Use synthetic inputs; redact credentials, account identifiers and private project content before sharing logs.

## Make a change

1. Read [AGENTS.md](AGENTS.md), [Start here](docs/00_START_HERE.md), and the relevant documents in the [index](docs/INDEX.md).
2. Work on a focused branch. Preserve unrelated files, existing recipe versions, scoped permissions and explicit caller handoffs.
3. Update the document that owns the behavior: specs for requirements, architecture for mechanisms, plans for future work, logs for observed results.
4. Run checks appropriate to the change and report their actual results. Keep live-provider tests separate from account-free validation.
5. Open a pull request explaining the problem, resulting behavior and verification. List material limitations and any upstream code adapted.

## Local checks

Prepare documentation dependencies once:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-check.txt
```

For documentation or contract changes:

```sh
.venv/bin/python tools/check_docs.py --full-schema
```

For runtime changes, the required checks are:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo test --workspace --features apr-cli/fault-injection --locked
```

For UI changes, exercise the real interface against disposable state and run the affected UI checks. For an integrated verification or package, follow the [local verification guide](docs/runbooks/local-verification.md). Documentation-only changes do not need a new installer or the runtime suites. GitHub Actions is not required; a hosted check is not a substitute for native evidence.

Default builds and core tests need no Jev or inference account. Use controlled fixtures for provider failures. Live calls require explicit account/spend authorization; never require contributor secrets to pass a pull request.

## Attribution and evidence

When adapting code, retain the original copyright/license text, identify the upstream revision and files, describe the adaptation, and update [Third-party notices](THIRD_PARTY_NOTICES.md) and [Credits](CREDITS.md). Credit design influences separately. Do not copy upstream results into CookBook's benchmark table.

Keep credentials, local grants, runtime databases, continuation tokens, private documents and machine-specific client configuration out of Git. Publish only reviewed evidence with source identity, scope and unknowns. A model-generated label is not automatically ground truth, and a synthetic provider success is not a live-account test.
