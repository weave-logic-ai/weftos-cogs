# API reference

> The bridge's HTTP routes, config and store behaviour.

Replace `<seed>` with `169.254.42.1` (USB), or the Seed's LAN or tailnet address.

## Agent endpoints (port 80)

Plain HTTP, `Access-Control-Allow-Origin: *`. A bearer token may be required for writes (`Authorization: Bearer <pairing token>`).

| Endpoint | What it does |
|---|---|
| `GET /api/v1/apps` | installed cogs, with `running` |
| `GET` / `PUT /api/v1/apps/bridge/config` | **PUT replaces the whole config and restarts the cog.** Send every key. |
| `POST /api/v1/apps/bridge/start` \| `/stop` | lifecycle |
| `POST /api/v1/apps/bridge/console` | run an allowed command: `--once`, `--once --simulate`, `--help` |
| `GET /api/v1/apps/bridge/logs` | stdout and stderr |

## Bridge endpoints (port 8048)

| Endpoint | Returns |
|---|---|
| `POST /ingest` | accept one reading (see **Protocol**; signed when an allowlist is set). `{"ok":true,"stored":"pi5/sen0628-tof -> store id 30"}` or `{"ok":false,"error":...}`. Auth refusals are `{"ok":false,"error":"<code>","detail":...}` with 400, 401, 403 or 503 (see **Security**) |
| `GET /status` | counts always (`sources`, `readings`, `rejected`, `store_errors`, `auth_refused`, `throttled`, `busy_refused`, `auth_mode`, `bind_scope`); per-source detail (names, vectors, metrics) and allowlist health (`allowlist_keys`, `allowlist_error`, `allowlist_age_s`) only for loopback callers or a validly signed GET |
| `GET /sources` | `{ "pi5/sen0628-tof": 30, ... }` for privileged callers (as `/status`); otherwise `{"sources": N}` |
| `GET /guide` | this guide as JSON |

Example `/status` (illustrative):

```json
{"status":"ok","sources":1,"readings":42,"rejected":0,"store_errors":0,
 "base_store_id":30,
 "detail":[{"key":"pi5/sen0628-tof","store_id":30,"count":42,"age_ms":600,
            "last_vector":[0.34,1,1,0.1,0.08,0.68,0.68,0.34],
            "last_metrics":{"nearest_mm":1193,"presence":true}}]}
```

## Config

| Key | Default | Range / meaning |
|---|---|---|
| `interval` | 5 | 1–60 s between status reports |
| `base_store_id` | 30 | first source's store id; later sources get the next ids |
| `max_sources` | 16 | 1–64 distinct `(node, cog)` sources accepted |
| `allowlist` | "" | path to the `NODE PUBKEY_HEX` file; when set, ingest needs a signed request |
| `allow_shared_token` | false | accept the deprecated `X-Bridge-Token` as a fallback |
| `token` | "" | the deprecated shared secret; ignored (and ingest refused) unless `allow_shared_token` |
| `auth_window` | 60 | 5 to 600 s: allowed clock skew, and replay-nonce lifetime |
| `no_store` | false | do not write to a Seed store (receiver on a WeftOS node) |
| `relay_to` | "" | `http://host:8048/ingest`: republish this Seed's sensor stream there |
| `node_id`, `key_file` | "" | this node's allowlist name and Ed25519 key file, for the relay |
| `seed_url` | `127.0.0.1:80` | the agent whose `/api/v1/sensor/stream` is relayed |
| `relay_cog` | `seed-stream` | the `cog` field of relayed readings |
| `relay_buffer` | 256 | readings held while the receiver is unreachable |
| `api_bind` | `127.0.0.1:8048` | `0.0.0.0:8048` accepts the LAN (needs an allowlist or token, else it refuses to start) |
| `insecure_open` | false | allow unauthenticated writes on a non-loopback bind (bench only) |
| `status_auth` | false | loopback gets no special `/status` detail (use behind a local TLS terminator) |
| `simulate` | false | inject one synthetic source |

## CLI

`--once`, `--interval N`, `--base-store-id N`, `--max-sources N`, `--api-bind host:port`, `--simulate`, `--allowlist FILE`, `--allow-shared-token`, `--token T` (deprecated; or `BRIDGE_TOKEN` in the environment), `--insecure-open`, `--status-auth`, `--auth-window S`, `--no-store`, `--relay-to URL`, `--node-id NAME`, `--key-file FILE`, `--seed-url host:port`, `--relay-cog NAME`, `--relay-buffer N`.

Subcommand: `cog-bridge keygen <node> <key-file>` writes a new secret key (mode 0600, never overwrites) and prints the allowlist line.

## The sender (weft-bridge-send, on the source node)

`--from <cog export url>`, `--to <bridge url>`, `--source <node name>`, `--interval N`, optional `--token T`, `--cog <id>` (default read from the export). It polls `<from>/status`, takes `vector` and a short metric set, and posts to `<to>/ingest`. The WeftOS sender does not sign yet, so against an allowlisted bridge use `--allow-shared-token` during migration, or relay with `cog-bridge --relay-to`.
