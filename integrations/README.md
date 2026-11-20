# Project client integration

`apr integrate` manages a local Codex or Claude Code MCP entry and APR skill. No settings change during build or tests. Create an assignment attachment first using the [client runbook](../docs/runbooks/client-integration.md), then use absolute paths:

```sh
apr --state-dir /private/apr-state integrate preview \
  --client codex --project "/path/to/project with spaces" \
  --attachment-file /private/apr-state/attachments/client.json \
  --output-file /private/apr-state/plans/codex.json
apr --state-dir /private/apr-state integrate apply \
  --plan-file /private/apr-state/plans/codex.json
apr --state-dir /private/apr-state integrate doctor \
  --client codex --project "/path/to/project with spaces"
```

Use `--client claude` for Claude Code. Preview returns the generated server entry, paths, change names and preimage hashes. It never prints existing settings or attachment contents. Omit `--output-file` for a read-only preview, or save to a new private file beneath the state directory for apply. `--executable` selects an installed APR copy; otherwise setup uses the running binary.

| Client | Local config entry | Project skill |
| --- | --- | --- |
| Codex | `.codex/config.toml`, `mcp_servers.apr` | `.agents/skills/apr/SKILL.md` |
| Claude Code | `.mcp.json`, `mcpServers.apr` | `.claude/skills/apr/SKILL.md` |

Unrelated values survive all operations; unrelated TOML comments survive too. Managed setup rejects tracked config files, unsafe/symlinked paths, stale previews, and unmanaged existing APR entries or skill bodies. It does not alter global settings, Git ignores, client project trust or tool-approval policy.

`integrate update` accepts the same setup arguments as preview and only changes an existing owned installation. `integrate uninstall --client CLIENT --project PATH` removes unchanged owned material and preserves other settings, attachments and runtime history. User edits to the owned entry/skill produce a conflict. Private before/after snapshots and journals support retrying an interrupted apply, update or uninstall without silently discarding later edits. Keep the exact saved plan for apply retries.

Doctor separately reports configuration, client executable/version, and authenticated APR service readiness. It may start the APR service, but never a model session. It leaves client trust `unknown` and actual client connection `unverified`; successful doctor is not full Codex/Claude acceptance. Use `--client-executable PATH` to choose the diagnostic executable. Both model-driven client journeys remain separate acceptance gates.

The following files remain **inactive manual templates**:

- [Codex configuration](codex/config.toml.template)
- [Claude configuration](claude/mcp.json.template)
- [Canonical skill](skills/apr/SKILL.md)

Each selected attachment binds one assignment. Two conversations using it share that assignment. Do not commit active descriptors or generated device-specific paths; attachment files remain inside their protected runtime state directory. Native same-user file access is outside APR's interface boundary.
