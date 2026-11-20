# Inactive client configuration templates

The current inactive templates and shared skill have moved to the [canonical integrations directory](../../integrations/README.md). Follow the [scoped MCP setup runbook](../../docs/runbooks/client-integration.md) for commands that work with the implemented runtime. They remain outside client discovery locations and are not installed automatically.

- [Codex TOML template](../../integrations/codex/config.toml.template)
- [Claude MCP JSON template](../../integrations/claude/mcp.json.template)
- [Shared skill](../../integrations/skills/apr/SKILL.md)

Resolve the executable and create a private local assignment attachment before selecting a client configuration. Do not commit private attachment material or reuse another machine's absolute paths.

The future installer must merge only its own entries, preserve existing settings, distinguish config success from live connection, and leave client trust/approvals under the user's control.
