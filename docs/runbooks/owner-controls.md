# Local owner controls

These commands run through the local owner CLI. They are not attachment or MCP tools. Replace paths and IDs before use; ordinary configuration belongs in JSON files, while credentials enter only through protected stdin and the private local TOML credential store.

## Saved keys without recurring vault prompts

CookBook manages `STATE_DIR/credentials.toml` automatically. It is plaintext with owner-only permissions, not an encrypted vault. Never paste its contents into a chat or include it in project files. To inspect storage without printing keys:

```sh
apr --state-dir /private/apr-state execution credential-store-status
```

For an existing installation, explicitly import previously saved OS credentials:

```sh
apr --state-dir /private/apr-state execution migrate-platform-credentials
```

The OS may request consent to read each old entry once. Migration skips credentials already present, aborts if an account changes, preserves old entries for rollback, and never changes grants or reconnects accounts. Normal recipe calls only read the local file. CookBook backups deliberately omit it; reconnect accounts when restoring elsewhere.

## Configuration and execution

```sh
apr --state-dir /private/apr-state config project-set \
  --project PROJECT_ID --process-ref PROCESS_REF --values-file /path/to/values.json
apr --state-dir /private/apr-state permission promote \
  --source SOURCE_ASSIGNMENT --target TARGET_ASSIGNMENT --enabled true
apr --state-dir /private/apr-state execution grant-python \
  --assignment ASSIGNMENT_ID --interpreter /absolute/path/to/python3
apr --state-dir /private/apr-state execution configure-connection \
  --assignment ASSIGNMENT_ID --name example --connection-file /path/to/connection.json
```

Project values are nonsecret process defaults. Promotion permission is explicit, same-project and directional; use `--enabled false` to revoke it. Native Python execution is an owner-approved capability, not a host sandbox. The interpreter and source are pinned by the runtime. **Settings → Python recipes** provides the native review flow. Approving an identical enabled interpreter preserves its revision; changed or revoked pins still block older frozen work. The agent may prepare the environment but cannot grant itself access.

For an authenticated connection, append `--secret-stdin` and pipe the credential from your secure source. Input must be 1–8192 printable ASCII bytes without whitespace or a trailing newline. No CLI argument accepts a secret. The runtime generates a protected credential reference rather than trusting one from the connection JSON. Tests use nonsecret connection configuration and invalid synthetic stdin; they do not create personal platform credentials or call providers. See [execution architecture](../architecture/execution.md) for the binding contract.

```sh
apr --state-dir /private/apr-state execution revoke \
  --assignment ASSIGNMENT_ID --kind connection --name example
apr --state-dir /private/apr-state execution reconcile --request-file /path/to/reconciliation.json
apr --state-dir /private/apr-state execution rebind RUN_ID
```

Revocation or a changed binding prevents later dispatch under the old grant. Reconciliation records observed evidence for an uncertain external effect; it must happen before rebinding a run with unresolved effects. Rebinding acknowledges current approved bindings and does not erase uncertain execution history.

## Backup and restore

```sh
apr --state-dir /private/apr-state backup --output /private/new-backup-directory
apr --state-dir /private/new-restored-state restore /private/new-backup-directory
apr --state-dir /private/new-restored-state project relocate PROJECT_ID /absolute/path/to/clone
```

Backup and restore destinations must be absolute, new paths. Restore never overwrites an existing store. Backups contain private workflow data and verified artifacts, but exclude client attachment files and protected credentials. Restored attachments and execution grants are disabled. Renew waiting caller handoffs and reconcile uncertain effects before continuation; reapprove and rebind device-local execution separately.

Relocation binds the saved project identity to the selected local directory. It revokes that project's old attachments and execution grants and blocks queued/running work. It does not silently trust a new checkout or execute frozen work. Managed client setup on the new device remains a separate step.

## Storage and retention

```sh
apr --state-dir /private/apr-state storage usage
apr --state-dir /private/apr-state storage quota --bytes 1073741824
apr --state-dir /private/apr-state storage quota --unlimited
apr --state-dir /private/apr-state storage pin-run RUN_ID --pinned true
apr --state-dir /private/apr-state storage pin-artifact ARTIFACT_ID --pinned true
apr --state-dir /private/apr-state storage retention-preview \
  --terminal-before-ms UNIX_MILLISECONDS --orphan-before-ms UNIX_MILLISECONDS \
  --output-file /private/apr-state/plans/retention.json
apr --state-dir /private/apr-state storage retention-apply \
  --plan-file /private/apr-state/plans/retention.json
```

Preview computes a concrete deletion plan without deleting data. With `--output-file`, it saves a new private plan and prints counts plus its path; omit that flag to print the full plan. Inspect the saved plan before applying. Apply rechecks current references, pins and object identities and refuses a stale plan. A plan file is bounded to 16 MiB and must remain inside the private state root when created by the CLI.

Retention deletes only eligible terminal work and unreferenced/unpinned objects under the saved policy. Use `--pinned false` to remove an explicit pin. Back up valuable history before deletion.

`storage quota --bytes N` caps the committed immutable object bytes in this state directory, including unreferenced objects. The one-gibibyte example can be replaced with the desired limit; `--unlimited` explicitly removes it. Lowering a quota below current usage preserves existing reads and deduplicated publications but blocks new growth. `storage usage` reports `quota_mode`, `quota_bytes` and `quota_used_bytes`. Database files, staging files and native programs' own filesystem use are outside this artifact quota.
