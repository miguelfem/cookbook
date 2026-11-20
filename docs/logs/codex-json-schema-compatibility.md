# Codex MCP JSON-value compatibility check

Checked 2026-09-16. This is schema/protocol verification, not another model run or an approval-policy change.

## Cause and fix

The real APR SDK tools/list response advertised RunArgs.input as boolean JSON Schema `true`. The bundled Codex smoke produced a JSON-encoded string for an object-valued process input, but approval policy `never` blocked the tool before APR received it. No run was created by that smoke.

The [official Codex normalizer](https://github.com/openai/codex/blob/8971fc25a96c5368f6e7bd8e66799a2dec476f9a/codex-rs/tools/src/json_schema.rs#L71-L75) explicitly converts boolean schemas to type:string.

APR now emits an explicit recursive AprJsonValue schema for arbitrary JSON values, and typed object maps for configuration and provenance. This changes schema metadata only: serde_json::Value remains the wire representation; there is no JSON-string decoding, no API field change, and no change to approvals, annotations, byte limits, token handling, or tool count.

## Independent normalization probe

Downloaded and compiled the unmodified official Codex json_schema.rs plus types.rs, compaction.rs, and traversal.rs at commit 8971fc25a96c5368f6e7bd8e66799a2dec476f9a in a temporary standalone probe. Its Cargo files and dependencies were outside the APR repository. Fed all five schemas emitted by the real APR MCP bridge in disposable state through parse_tool_input_schema (including normalization, definition pruning, size policy, and typed deserialization).

All five parsed. The run, resume, and manage schemas retained their recursive local references and all six JSON types. Draft 2020-12 validation of normalized schemas accepted null, booleans, numbers, strings containing JSON-looking text, and nested arrays/objects. This checks the pinned published Codex source, not the exact bundled binary's entire model request path.

Emitted schema sizes measured by Python json.dumps (diagnostic, not token counts): discover 507 B; inspect 2551 B; manage 2206 B; resume 2617 B; run 836 B.

## Regression and final checks

The new real-MCP regression failed before the fix because inputSchema.properties.input was `true`. It passes after the fix and verifies every tool schema avoids boolean schema nodes for values. Real run/handoff/submit requests round-trip nested objects/arrays, JSON-looking strings, booleans, numbers, and null unchanged. Ordinary saved configuration and per-run overrides also round-trip nested raw values.

Final focused checks: 7 CLI MCP integration tests passed; 3 apr-mcp unit tests passed; strict apr-cli/apr-mcp all-target/all-feature clippy passed. No model was called, no approval was bypassed, no personal settings were edited, and no upstream probe source or dependency was added to APR.

## Linux cached-variant discrepancy

A subsequent Linux ARM64 default-feature gate passed, while the existing all-feature variant failed the explicit-schema assertion. Reproduced with the focused MCP test. Direct tools/list inspection of the failing executable showed apr_run.input was still boolean true and its definitions contained only RunMode, proving it had the previous schema implementation. Rebuilding apr-cli alone kept the failure. Touching the copied apr-mcp and CLI source mtimes forced Cargo to rebuild apr-mcp plus apr-cli, and the identical focused all-feature test then passed (1 test, 0.86 s). Source content and assertions were unchanged; host/container SHA-256 values matched afterward.

The source copied into the container had preserved timestamps older than cached compilation artifacts. Some copied files were root-owned, so timestamp refresh required container root; all cargo/test execution remained the unprivileged aprtest user. Refresh all copied workspace source timestamps or package-clean local workspace crates before final feature-variant gates to avoid this stale-build condition. No runtime/schema code change was required.

- `crates/apr-mcp/src/lib.rs` SHA-256: `c7f0477003ad518e9baf7e18720c26787ec38bfc4679389acb4e31f9ffb97ed9`
- `crates/apr-cli/tests/mcp.rs` SHA-256: `388f742b79c23af309bc882d94ada385b90e459c3a102cb21586c78c6e9c20e5`
