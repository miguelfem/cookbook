# CookBook APR desktop

Tauri 2 developer preview around the same headless Rust runtime. Main window: Home, Recipes, Runs, Connections, Costs and Schedules. Separate trusted window: agent connection, tools/accounts, permission review, OS-vault key entry, revocation and rebind.

Read [the desktop runbook](../../docs/runbooks/desktop.md) and [connection architecture](../../docs/architecture/recipe-connections.md). Build with `npm ci && npm run build` after preparing Rust and the platform's Tauri prerequisites. The native runtime currently supports macOS/Linux, not Windows. Signing/notarization and live-provider qualification are separate from compilation and synthetic tests.

No embedded chatbot, visual graph authoring, analytics tracker, remote content, implicit paid fallback or general shell bridge is included. The agent writes recipes and cost analysis; the app makes execution and authority visible.
