# Dependency maintenance — 2026-10-22

Updated runtime ZIP 4.6.1 → 9.0.5 and libc 0.2.183 → 0.2.191, plus locked Rustls 0.23.46, rustls-webpki 0.103.16 and webpki-roots 1.0.10. Desktop dependencies now use Tauri 2.12.5, tauri-build 2.7.4, updater 2.14.0 and patched root certificates; the resolved desktop update removed the five unmaintained UNIC crates. No credential binding, recipe, schedule, live runtime state or native integration was changed.

Verified `cargo fmt --all -- --check`, Clippy with all workspace targets/features and warnings denied, 417 standard workspace tests, and 460 fault-injection tests. A freshly built debug APR sidecar satisfied the desktop resource prerequisite; native desktop `cargo check --locked` then passed. This does not establish signed packaging, an installed-app update, Linux behavior, or live provider acceptance.

Remaining source advisory entries: Linux GTK still requires GLib 0.18.5 (RUSTSEC-2024-0429; fixed upstream in 0.20+), and proc-macro-error/zbus_names/zvariant/zvariant_derive remain unmaintained upstream dependencies. The updater's ZIP 4.6.1 entry is Windows-only; Windows remains excluded. The runtime uses ZIP 9.0.5. No incompatible transitive substitution, warning suppression, or updater extraction removal was applied.

Refreshed the source distribution notice inventory against the updated locked macOS graph: 201 components and 339 preserved license-file entries, with no missing root/declared texts. Historical notices remain preserved; this is not a new release or legal certification.

Existing dirty source was verified unchanged. Dependency and documentation changes remain local and uncommitted; no new installer or release was produced.
