# Third-party notices

For a short overview of source adaptations, dependencies and design influences, see [Credits](CREDITS.md). The notices and inventories here retain their documented scope and license texts.

Checked against the current lockfile on 2026-10-22. APR uses Cargo packages as unmodified dependencies. Their license terms apply to their respective code; APR's original software license remains undecided.

The [machine-readable inventory](third_party/dependency-notices.json) records the package-selected `cargo tree` default `apr-cli` normal/build dependency graph for `aarch64-apple-darwin`: 201 components, their Cargo checksums and feature metadata, and 339 preserved license-file entries. Root or explicitly declared license files are copied from Cargo's exact package sources. The [supplement inventory](third_party/distribution-supplements.json) pins official upstream commits and text hashes for four published crates that omit their license files. Historical notice directories remain preserved; the current inventory identifies active versions.

## Direct workspace dependencies

| Dependency | Version | Upstream license expression | Preserved texts |
| --- | --- | --- | --- |
| clap | 4.6.7 | MIT OR Apache-2.0 | [Directory](third_party/licenses/clap-4.6.7/) |
| fs2 | 0.4.3 | MIT/Apache-2.0 | [Directory](third_party/licenses/fs2-0.4.3/) |
| jsonschema | 0.33.0 | MIT | [Directory](third_party/licenses/jsonschema-0.33.0/) |
| keyring | 3.6.3 | MIT OR Apache-2.0 | [Directory](third_party/licenses/keyring-3.6.3/) |
| libc | 0.2.191 | MIT OR Apache-2.0 | [Directory](third_party/licenses/libc-0.2.191/) |
| rmcp | 3.4.0 | Apache-2.0; transition notice below | [Directory](third_party/licenses/rmcp-3.4.0/) |
| rusqlite | 0.39.0 | MIT | [Directory](third_party/licenses/rusqlite-0.39.0/) |
| schemars | 1.2.2 | MIT | [Directory](third_party/licenses/schemars-1.2.2/) |
| serde | 1.0.228 | MIT OR Apache-2.0 | [Directory](third_party/licenses/serde-1.0.228/) |
| serde_json | 1.0.149 | MIT OR Apache-2.0 | [Directory](third_party/licenses/serde_json-1.0.149/) |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | [Directory](third_party/licenses/sha2-0.10.9/) |
| signal-hook | 0.3.18 | Apache-2.0/MIT | [Directory](third_party/licenses/signal-hook-0.3.18/) |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | [Directory](third_party/licenses/tempfile-3.27.0/) |
| thiserror | 2.0.18 | MIT OR Apache-2.0 | [Directory](third_party/licenses/thiserror-2.0.18/) |
| time | 0.3.47 | MIT OR Apache-2.0 | [Directory](third_party/licenses/time-0.3.47/) |
| tokio | 1.53.1 | MIT | [Directory](third_party/licenses/tokio-1.53.1/) |
| toml_edit | 0.25.15+spec-1.1.0 | MIT OR Apache-2.0 | [Directory](third_party/licenses/toml_edit-0.25.15+spec-1.1.0/) |
| ureq | 3.4.2 | MIT OR Apache-2.0 | [Directory](third_party/licenses/ureq-3.4.2/) |
| uuid | 1.22.0 | Apache-2.0 OR MIT | [Directory](third_party/licenses/uuid-1.22.0/) |
| zeroize | 1.8.2 | Apache-2.0 OR MIT | [Directory](third_party/licenses/zeroize-1.8.2/) |
| zip | 9.0.5 | MIT | [Directory](third_party/licenses/zip-9.0.5/) |

## Bundled native dependency

`rusqlite` 0.39.0 uses `libsqlite3-sys` 0.37.0 under [MIT](third_party/licenses/libsqlite3-sys-0.37.0/LICENSE). Its `bundled` feature compiles SQLite 3.51.3, with source id `2026-03-13 10:38:09 737ae4a34738ffa0c3ff7f9bb18df914dd1cad163f28fd6b6e114a344fe6d618`. SQLite's [public-domain notice](third_party/licenses/sqlite-3.51.3/PUBLIC-DOMAIN-NOTICE.txt) is copied from that amalgamation. SQLCipher is not enabled.

## SIMD supplemental notices

`uuid-simd` and `vsimd` 0.8.0 declare MIT and identify upstream commit `d74c030d9dc4f3cae02146d1f497ff62726ef09a`. Their published crates omit the root license. The [exact upstream MIT text](https://github.com/Nugine/simd/blob/d74c030d9dc4f3cae02146d1f497ff62726ef09a/LICENSE), including **Copyright (c) 2021 Nugine**, is preserved for [uuid-simd](third_party/licenses/uuid-simd-0.8.0/LICENSE) and [vsimd](third_party/licenses/vsimd-0.8.0/LICENSE).

## Distribution coverage

`tools/distribution.py package` regenerates the resolved inventory for the requested target and optional `jev` feature using locked, offline package-selected Cargo tree resolution and Cargo metadata. The archive contains the lockfile, per-file checksums, preserved project notices including the Lobster adaptation, root/declared dependency notices, and the packaging toolchain's Rust library copyright and license files. Target/features are caller-declared; this does not attest the supplied binary's build. Packaging is deterministic for the same binary, inputs, metadata and toolchain notices.

The checked-in inventory covers the stated graph, not every platform or feature in `Cargo.lock`, and is not a legal certification of every embedded data file. Regenerate and review it for a new lockfile, feature or target. Rust library notices belong to the current packaging toolchain and require matching against the actual release build. Development packaging is allowed while APR licensing is undecided; stable mode requires a nonempty APR `LICENSE` and no missing dependency root/declared texts. Signing, release authorization, platform qualification and license review remain separate gates.

APR does not embed Cloudflare Code Mode or Windmill. Research or design inspiration alone does not constitute copied code.

## Adapted source: TypeSafe computer use

The separate [background browser helper](integrations/computer-use/browser_worker.py)
uses explicitly installed [Playwright Python](https://github.com/microsoft/playwright-python)
(Apache-2.0) and its Chromium distribution. Neither is bundled in the Rust app;
their distribution licenses/notices remain with those separately installed packages.
CookBook does not claim ownership of those projects or their browser engine.

APR's optional [computer-use worker](integrations/computer-use/computer_worker.py) adapts macOS event posting, accessibility lookup and OCR block merging from **Aaron Levin's [typesafe-computer-use](https://github.com/awlevin/typesafe-computer-use)**, pinned to commit `7d120f1944d0156773d5089b81f8e46d108a6c8d`. The original files are `typesafe_computer_use/macos.py` and `typesafe_computer_use/perception.py`.

The complete [MIT license](third_party/licenses/typesafe-computer-use-7d120f19/LICENSE), including **Copyright (c) 2026 Aaron Levin**, is preserved in the repository, embedded in the CLI's scaffold, and copied into each generated process bundle. [Provenance](third_party/typesafe-computer-use.json) records the revision and license digest. [Adaptation notes](integrations/computer-use/NOTICE.md) describe APR's window capture, action bounds, app allowlist and fresh-observation checks. APR uses its own durable process graph and protected provider broker; upstream credential handling and the autonomous runner are not embedded. Optional Python dependencies are owner-installed separately from the Rust package; their own distribution licenses continue to apply.

## Official Rust MCP SDK

APR's stdio bridge links `rmcp` 3.4.0 and uses `rmcp-macros` 3.4.0, both published from upstream commit `fd7811fdaa9fefa1c8034534b4d7a31c97204f89`. Their Cargo metadata declares Apache-2.0. The [exact upstream root license](https://github.com/modelcontextprotocol/rust-sdk/blob/fd7811fdaa9fefa1c8034534b4d7a31c97204f89/LICENSE) additionally preserves legacy contributions whose MIT relicensing consent is pending, plus a CC-BY-4.0 notice for documentation. The complete [transition notice, Apache text and legacy MIT text](third_party/licenses/rmcp-3.4.0/LICENSE) are retained together for both packages, including **Copyright (c) 2024–2025 Model Context Protocol a Series of LF Projects, LLC.** Neither package is modified. No upstream documentation is copied into APR's user guide.

## Adapted source: OpenClaw Lobster

The atomic exclusive publication logic in `crates/apr-runtime/src/artifacts.rs` adapts `writeFileAtomicExclusive` from [OpenClaw Lobster's `src/state/atomic_file.ts`](https://github.com/openclaw/lobster/blob/041124ad4ae5f05179302970a235fe5d34bac45b/src/state/atomic_file.ts), commit `041124ad4ae5f05179302970a235fe5d34bac45b` (package version 2026.9.13).

Upstream copyright: **Copyright (c) 2026 Vignesh**. The complete [MIT license](third_party/licenses/lobster-041124ad/LICENSE) is preserved. APR translates the publication sequence to Rust, uses directory-relative file descriptors and no-follow checks, verifies content-addressed objects, and retains a published object after a later synchronization failure instead of unlinking it. This is a narrow source adaptation; APR does not embed the Lobster workflow engine.
