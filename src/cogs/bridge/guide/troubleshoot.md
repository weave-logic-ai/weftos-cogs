# Troubleshoot

> Symptom, likely cause, fix.

| Symptom | Likely cause | Fix |
|---|---|---|
| `/status` stays at zero sources | nothing is sending | start `weft-bridge-send` on the other board; check its `--to` points at this Seed's `:8048` |
| Sender logs connection refused | the bridge isn't running, or `api_bind` is loopback | start the bridge; set `api_bind` to `0.0.0.0:8048` (the default is `127.0.0.1:8048`), with an allowlist |
| `/ingest` returns `401` | the response `error` says why | `missing_signature`: sender is unsigned; `unknown_node`: add its `keygen` line to the allowlist; `bad_signature`: wrong key file; `stale_timestamp`: fix the sender's clock; `replayed_nonce`: the sender reused a nonce; `predates_restart`: the request was signed before the bridge last restarted, retry; `shared_token_disabled`: sign requests or set `allow_shared_token` |
| `/ingest` returns `403 source_mismatch` | a node sent a `source` other than its own name | set the sender's source to its node name |
| bridge exits at start: "refusing to start" | no allowlist and no token on a non-loopback bind | add an allowlist, bind `127.0.0.1`, or (bench only) `insecure_open` |
| `/ingest` returns `415` | `Content-Type` is not `application/json` | send the header (the signed sender does) |
| `/ingest` returns `429` or `503` | per-node or per-IP rate, per-IP (4) or global (32) connections, or `clock_not_set` | slow down; the relay backs off by itself. `/status` counts `throttled` and `busy_refused` apart from `auth_refused` |
| `/ingest` returns `503 clock_not_set` | the bridge has no real clock yet (no RTC, waiting for NTP) | wait for time sync; senders retry |
| remote nodes stopped reaching the bridge after upgrading | 0.2 binds `127.0.0.1` by default | see **Setup**, 'Upgrading from 0.1' |
| `/ingest` returns `431` | headers over 8 KiB per line or 64 lines | send only the headers a reading needs |
| `/ingest` returns `400` | malformed reading | `source`, `cog` and a 1–8 number `vector` are required; see **Protocol** |
| Reading accepted but `store_errors` rises | the Seed store rejected the write | check the agent is healthy (`/api/v1/status`); the store dimension must be 8 |
| A source is `rejected` | more than `max_sources` distinct `(node, cog)` pairs | raise `max_sources`, or stop unused senders |
| Two boards overwrite each other | they use the same `source` name | give each node a distinct `--source` |
| Export unreachable from the app | the cog isn't running, or `api_bind` is loopback | start the cog; the bridge export is on `:8048` |
| Config changes "lost" | the agent's config `PUT` replaces the whole config | the companion app merges for you; by hand, send every key |
| High `age_ms` on a source | the sender stopped, or the source cog stopped | check both are running on the other board |

## Check the loop by hand

```bash
# 1. the bridge is up (on the Seed; add -H ... or bind 0.0.0.0 for remote)
curl http://127.0.0.1:8048/status
# 2. post a reading yourself
curl -X POST http://127.0.0.1:8048/ingest -H 'Content-Type: application/json' \
  -d '{"source":"test","cog":"manual","vector":[0.5,1,1,0,0,0,0,0]}'
# 3. it should appear
curl http://127.0.0.1:8048/sources
```
