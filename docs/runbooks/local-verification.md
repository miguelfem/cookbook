# Local verification and packaging without GitHub Actions

GitHub Actions is optional, not the development environment. These commands use
this machine's Rust, Python, Node and native build dependencies. They never start
a hosted workflow, install a client integration, connect a provider, or publish
a package. Windows is excluded.

## Verify the implementation

Install the Rust version in `rust-toolchain.toml`, Node/npm, a C compiler and Python
3.11 or later. Prepare the documentation dependency in your own environment:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements-check.txt
.venv/bin/python tools/local_verify.py
```

The script runs formatting, strict all-target/all-feature Clippy, default,
fault-injection and all-feature workspace tests, full documentation/schema
validation, distribution tests, desktop form regressions and JavaScript syntax
checks. It fails if a required check or prerequisite
is missing; a skipped platform is not a pass. Synthetic runtime tests may start
controlled loopback services but never use a live provider account.

Each invocation writes private logs and `report.json` to a new local temporary
directory. Use `--output /absolute/new-evidence-directory` to select a destination.
Existing evidence is never overwritten. The report includes the Git commit,
working-tree content fingerprint, dirty state, exact commands, exit codes and
scope. A source edit during verification invalidates single-revision evidence.
An extracted source archive without Git has an explicitly unknown commit identity.

The local runner defaults to four compiler jobs and four test threads to avoid
large host CPU counts overwhelming constrained machines. Override these with
`CARGO_BUILD_JOBS` and `RUST_TEST_THREADS`; assertions and execution limits are not
weakened. `APR_TEST_PYTHON` can pin an absolute existing Python interpreter.

`--plan` prints the commands without running them. `--offline` prevents Cargo/npm
from fetching uncached dependencies and also applies to nested Cargo builds. A
machine with missing dependencies must prepare them before using offline mode.

## Verify the desktop on its actual platform

Install [Tauri's native prerequisites](https://v2.tauri.app/start/prerequisites/).
macOS needs Xcode command-line tools; Linux needs GTK 3 and WebKitGTK 4.1 development
libraries. A browser fixture or core-only test does not establish a native build.

```sh
.venv/bin/python tools/local_verify.py --desktop
```

This adds preparation of the matching APR sidecar and frontend assets, desktop
formatting, strict native Clippy and desktop tests. Complete live user-account,
credential, native UI and signed-update acceptance remain separate trials. Native
macOS validation must run on macOS, not be inferred from a Linux result.

## Build a development installer

```sh
.venv/bin/python tools/build_desktop_local.py
```

For the explicit optional adapter, add `--with-jev`. This selects both native and
sidecar Cargo features; `packages.json` records `included_features`. It supplies
no account configuration or execution grant. Default builds exclude Jev even if
the shell inherited `COOKBOOK_INCLUDE_JEV=1`.

The host architecture selects Apple Silicon or Intel `.app`/`.dmg` on macOS, or a
Debian package on Linux amd64/arm64. Architecture support in the script is not
itself qualification on that architecture. Compilation and bundling are separate
stages and both must pass. A dedicated ignored target directory preserves local
build caches. Only newly produced installers are copied into the evidence folder,
with `packages.json` and SHA-256 checksums; stale packages are never presented as
new builds. No package is automatically installed or uploaded.

Preview builds strip publisher keys/feed settings and signing secrets from child
environments. They do not acquire a publisher identity or update endpoint merely
because those variables exist in your shell. `--offline` applies to Cargo/npm;
platform bundler helpers can require separately cached tools or network access.
It is not a network sandbox. Publisher notarization is not an offline operation.

## Locally sign and notarize a release

Choose the product's `LICENSE` before release. Provision your publisher key and
HTTPS update endpoint, and on macOS an existing local Developer ID signing identity
plus notarization credentials as described in the [desktop runbook](desktop.md).
Keep secrets in your local secret mechanism/environment, never command arguments,
source control, screenshots or chat.

```sh
.venv/bin/python tools/build_desktop_local.py --signed
```

Missing license, key or notarization configuration blocks release. The signing key
is inherited only by the explicit bundle stage, not dependency installation,
compilation or linting. On macOS, the script verifies the application signature
and stapled notarization ticket. Matching updater-signature files are required;
there is no unsigned fallback. The collector checks presence, provenance and
hashes, not a new independent cryptographic verification of the updater signature.
Tauri verifies that signature during installation.

Publication stays explicit: assemble reviewed architecture-specific bundles with
`tools/release_manifest.py`, then publish them to your chosen HTTPS host. Neither
local command requires GitHub Actions or changes your keychain's default settings.

## Save changes without starting hosted checks

The existing automatic workflows listen to `push`/`pull_request`. A development
commit can include `[skip actions]` to avoid those triggers. This does not apply to
other workflow event types; inspect the configured triggers before using it. Do
not weaken branch protections or synthesize green CI results: required checks can
remain pending while local evidence is reviewed. Do not restart Actions to obtain
a source archive, compiler or patch transport.

Reference: [GitHub's skip-workflow documentation](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs).
