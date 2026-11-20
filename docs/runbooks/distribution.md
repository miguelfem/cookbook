# Local distribution and repeatable samples

Updated 2026-09-16. These tools produce development packages and install into an explicit local prefix. They do not publish a release, sign binaries, configure clients, start background services, or remove saved runtime state. Python 3.10+, Cargo and the packaging toolchain's Rust library notices are required. Final source passes macOS ARM64 and Ubuntu x86_64 CI, including real default-binary packaging on both. Distinct-binary installation/update/removal has direct local macOS evidence; the hosted installation tests use fixtures. An earlier Linux ARM64 baseline also passed the core suites. Linux desktop credential sessions and client journeys need separate qualification. See the [validation and provenance record](../logs/runtime-validation.md).

## Build and package

Build the default executable with `cargo build --locked --release -p apr-cli`. Locate the binary under the selected Cargo target directory. Default builds exclude Jev and fault injection. An explicitly requested Jev binary is built with `--features jev` and packaged with `--feature jev`.

From the repository root, use the actual binary and target:

```sh
python3 tools/distribution.py package \
  --apr /absolute/path/to/apr \
  --target aarch64-apple-darwin \
  --output /absolute/existing/directory/apr-development.tar.gz
```

The output file must be absent. The archive includes the executable, lockfile, dependency and file-hash manifests, preserved upstream notices and current Rust library notices. Package-selected Cargo tree resolution excludes dev-only dependencies and workspace-only test feature unification; Cargo metadata supplies source and license details. Packaging is offline, so the requested dependencies must already be present in Cargo's cache. Repeating with identical binary, source files, metadata and toolchain notices produces identical archive bytes.

`package.json` labels target/features as caller-declared and the Rust notices as belonging to the packaging toolchain. The tool does not prove how a supplied binary was built. `--mode stable` requires a nonempty project `LICENSE` and no missing root/declared dependency texts. APR's license remains undecided. A development package is not a signed, qualified, open-source release; see [third-party notices](../../THIRD_PARTY_NOTICES.md) for coverage.

## Review, install, update and remove

Choose an explicit prefix outside your source checkout and runtime state. Planning only reads the installation and package, then writes the requested new plan file:

```sh
python3 tools/distribution.py plan-install \
  --archive /absolute/existing/directory/apr-development.tar.gz \
  --prefix /absolute/install/prefix \
  --output /absolute/existing/directory/install-plan.json
```

Inspect the JSON plan. Apply that exact file:

```sh
python3 tools/distribution.py apply /absolute/existing/directory/install-plan.json
```

The executable becomes `PREFIX/bin/apr`, a relative symlink to an immutable hash-named release under `PREFIX/share/apr/releases/`. The installer records owned files and hashes, serializes operations with a permanent lock, and atomically switches the executable pointer. It does not add the prefix to `PATH` or change a client's executable configuration. Use the [client setup workflow](client-integration.md) separately when refreshing client paths.

To update, create and apply a new install plan for the new archive and the same prefix. Previous owned releases remain for explicit review/removal. Existing unowned executables, edited managed files, added content inside an owned release, changed plans and stale installation snapshots become conflicts. Retry an interrupted operation with the same plan; do not delete ownership records to bypass a conflict.

Removal uses a separate plan:

```sh
python3 tools/distribution.py plan-remove \
  --prefix /absolute/install/prefix \
  --output /absolute/existing/directory/remove-plan.json
python3 tools/distribution.py apply /absolute/existing/directory/remove-plan.json
```

Removal deletes only the owned executable pointer and recorded releases. Its durable journal permits retry after interruption. Unowned files, runtime state, client settings and the permanent installer lock remain. State deletion is a separate owner action; this tool provides none.

## Measure local behavior

The resource sample records current idle service RSS, request latency and executable bytes. It does not measure peak RSS, installed footprint or model usage:

```sh
python3 tools/benchmark_service.py --apr /absolute/path/to/apr --iterations 20
```

The outcome harness runs [research triage](../../examples/research-triage/README.md), [operations intake](../../examples/operations-intake/README.md) and [software verification](../../examples/software-verification/README.md) through real CLI setup and an owned foreground service:

```sh
python3 tools/benchmark_outcomes.py --apr /absolute/path/to/apr --iterations 3
```

Each domain has an independent case and a caller-assisted case. The harness supplies canned answers, verifies saved outputs and report bytes, checks exact branch skipping, then replays request/submission identities and checks durable receipts and event counts. It reports registration/run duration, step/handoff counts, artifact bytes, protocol traffic and metadata-only storage totals. Initial and repeat runs use already-authored definitions; authoring and human review effort are not measured.

Both harnesses use disposable projects and private state, stop only their own service child, and print no attachment or continuation tokens. Runtime endpoint locks remain as permanent coordination files. Protocol bytes are not token counts. Model tokens, model cost and provider cost are unknown, represented as `null`; these tests do not establish comparative quality or savings. Real client/model acceptance and matched economics remain in the [verification plan](../plans/verification.md).

Run the install/recovery tests without modifying real settings:

```sh
python3 -m unittest discover -s tools/tests -p test_distribution.py -v
```
