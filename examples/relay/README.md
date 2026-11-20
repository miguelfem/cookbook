# CookBook relay templates

Host a webhook that can accept jobs while your CookBook device is offline. The relay stores requests in SQLite. A local worker polls over HTTPS, starts an approved saved recipe through CookBook's fixed-attachment MCP connection, and records the CookBook run ID.

These are untested starter templates for a single owner. Use one relay, one persistent volume, and one local worker assignment. The [loopback outage benchmark](../../docs/evaluations/2026-11-19-fresh-threads-and-standalone.md#model-free-schedule-and-relay) is separate from these templates.

## Set the relay credentials

Generate two independent random tokens, one for job producers and one for the worker. Run this command twice:

```sh
python3 -c 'import secrets; print(secrets.token_urlsafe(32))'
```

| Variable | Value |
| --- | --- |
| `APR_RELAY_RECIPES` | Comma-separated recipe aliases, such as `save-note` |
| `APR_RELAY_SUBMIT_TOKEN` | Bearer token for creating jobs and checking relay status |
| `APR_RELAY_WORKER_TOKEN` | Separate bearer token for claiming jobs and acknowledging handoff |

Store the values in your host's private environment configuration. The worker's CookBook attachment stays on the device that runs recipes. Use item IDs or scoped artifact references for private inputs and keep full reports on the CookBook host.

## Docker Compose on a VPS

From this directory, copy [.env.example](.env.example) to `.env`, restrict it to your user, and fill in the three values.

```sh
cp .env.example .env
chmod 600 .env
# Fill in the tokens in .env before starting the service.
docker compose up -d --build
```

[compose.yaml](compose.yaml) persists the queue at `/data/relay.sqlite3` in the `relay-data` volume and binds the host port to `127.0.0.1:8080`. Route your HTTPS domain to that address through your VPS reverse proxy. For example, a Caddy site can use:

```caddy
relay.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

## Railway with a persistent volume

The [Railway infrastructure template](.railway/railway.ts) points to this directory, mounts a volume at `/data`, and checks `/healthz`. Use a dedicated Railway project and environment for this template. Create the three variables above as **shared variables** in that environment before applying the configuration.

From this directory, use the current Railway CLI, link your relay project, and install the TypeScript SDK:

```sh
railway link
npm install --no-save --package-lock=false railway
railway config plan
railway config apply
```

In the relay service's Networking settings, generate a public HTTPS domain. Use that origin in the worker configuration. The Dockerfile listens on Railway's injected `PORT`. [Railway IaC commands](https://docs.railway.com/cli/config) · [Railway volumes](https://docs.railway.com/volumes/reference)

## Connect the CookBook worker

1. On the execution host, register the recipe as shown in the [first-project walkthrough](../../docs/runbooks/first-project.md). Record its immutable `sha256:…` process reference and the assignment that owns it.
2. Create a private run-only attachment for that assignment. Point `APR_BIN` and `APR_STATE` at the same executable and state directory as the registered recipe. [Fixed-attachment setup](../../docs/runbooks/client-integration.md#create-an-explicit-attachment)

   ```sh
   "$APR_BIN" --state-dir "$APR_STATE" attachment create \
     --assignment ASSIGNMENT_ID --mode run \
     --output-file "$APR_STATE/attachments/relay.json"
   ```

3. Copy [worker.example.json](worker.example.json) to `worker.json` on the execution host. Set the HTTPS origin, the local executable and state directory, the attachment file, and the alias-to-pin map. Make the aliases match `APR_RELAY_RECIPES`. Set `APR_RELAY_WORKER_TOKEN` in the worker's environment and start it with Python 3.11 or later.

   ```sh
   python3 worker.py worker.json
   ```

The worker uses one stable CookBook request key per job. An expired claim can be retried with that key after a worker restart. Drain queued jobs before changing the recipe pins, the assignment, or the runtime state directory. Jobs that require the local device wait for it to reconnect. Use an online execution host for recipes that must run while the device is unavailable.

## Trigger a recipe

Run this from a webhook sender or scheduler with your submit token in its environment. This synthetic request assumes the saved `save-note` recipe expects a `note` field.

```sh
curl --fail-with-body --silent --show-error https://relay.example.com/jobs \
  -H "Authorization: Bearer $APR_RELAY_SUBMIT_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{"request_key":"example-note-001","recipe":"save-note","input":{"note":"Hello from the relay"}}'
```

HTTP `202` returns a `job_id`. Use the same `request_key` and input when retrying the same event. A different input under that key returns HTTP `409`.

```sh
curl --fail-with-body --silent --show-error https://relay.example.com/jobs/JOB_ID \
  -H "Authorization: Bearer $APR_RELAY_SUBMIT_TOKEN"
```

The relay reports `queued`, `claimed`, or `submitted`. `submitted` means CookBook returned a run ID. Use the [local inspector](../../docs/runbooks/local-inspector.md) to follow the recipe and read its saved outputs.
