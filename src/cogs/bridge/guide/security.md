# Security

> Who can write to the Seed's memory through the bridge, and how to limit it.

The bridge writes into the Seed's store. Anything that can reach its port and send an accepted reading becomes a contributor to the Seed's memory. Treat the port as a trust boundary.

## Per-node identity (recommended)

Each sending node has its own Ed25519 keypair. The bridge keeps an **allowlist** of node public keys and accepts a reading only from a listed node.

1. **Make a key on the sending node** (the secret never leaves it):

   ```bash
   cog-bridge keygen pi5 /var/lib/cognitum/apps/bridge/node.key
   # prints the allowlist line:  pi5 1afb097a...  (64 hex characters)
   ```

   The key file is created with mode 0600 and `keygen` refuses to overwrite one.
2. **Add that line to the bridge's allowlist file** (one `NODE PUBKEY_HEX` per line, `#` comments allowed), and set the bridge config `allowlist` to its path. The file is re-read when it changes; a bad edit keeps the previous list and logs the error. Removing a line revokes the node.
3. **The sender signs every request** over the method, path, node, timestamp, a random nonce and the SHA-256 of the body, and sends `X-Bridge-Node`, `X-Bridge-Timestamp`, `X-Bridge-Nonce` and `X-Bridge-Signature`. See **Protocol** for the exact bytes.
4. **A node may only send its own name.** A request signed by `pi5` must carry `"source": "pi5"`, so one compromised node cannot write into another node's store ids.

### What is checked, and the refusal for each

| Check | Refusal (`error` code, HTTP status) |
|---|---|
| allowlist is set but the request is unsigned | `missing_signature` 401 |
| auth headers present but malformed (bad node name, timestamp not digits or beyond year 2100, bad nonce or signature) | `malformed_auth` 400 |
| node name not in the allowlist | `unknown_node` 401 |
| timestamp more than `auth_window` seconds (default 60) from the bridge clock | `stale_timestamp` 401 (the message says how far off) |
| timestamp older than the bridge process start | `predates_restart` 401 |
| signature does not verify (wrong key, or the body or path changed; strict Ed25519) | `bad_signature` 401 |
| nonce already used by this node inside the window | `replayed_nonce` 401 |
| node has 512 live nonces already | `nonce_cache_full` 503 |
| node exceeded 8 verified requests a second (burst 16) | `rate_limited` 429 |
| valid signature but `source` is not the node's name | `source_mismatch` 403 |

Replay protection is the timestamp window, a nonce cache, and the restart rule. The nonce cache is in memory, so a restart empties it; to stop a request captured before the restart from being replayed inside the window, the bridge refuses any request whose timestamp is older than its own start (`predates_restart`). A legitimate request caught in a restart gets that refusal once. The relay treats `predates_restart` as transient and retries with backoff, re-signing with a fresh timestamp and nonce each attempt, so the reading is not lost; a sender whose clock runs a few seconds behind sees it for those seconds after a restart. `stale_timestamp` is retried too, at most 10 times in a row, with a warning about clock skew, and then that reading is dropped, since a persistent skew will not fix itself.

The cost of a refused request is bounded. Nothing is remembered until the signature verifies, so unauthenticated traffic cannot fill the nonce cache or spend a node's rate budget; a replay of a captured request is refused before it counts against the node's rate. Per-node limits (rate, 512 nonces) mean one noisy or compromised node cannot evict another node's nonces or starve it. The bridge and the senders need clocks within the window of each other. Refusals are counted in `/status` (`auth_refused`) and logged with the reason.

### What a stolen node key exposes

A node key lets its holder write readings **as that node only**: `source` must equal the node name, the node is limited to 8 cogs (store ids), 8 requests a second and 512 live nonces, and the other nodes' data and budgets are untouched. It does let the holder read the detail view of `/status` and `/sources` with a signed `GET`, as any allowlisted node can (all nodes are mutually trusted readers of the summary data). It is revoked by deleting that node's allowlist line (below). It does not give the holder anything on the Seed beyond adding readings under that node's name.

### The allowlist file

- **Revoke and add by replacing the file atomically**: write a new file next to it, then `mv` it over (write-then-rename). The bridge checks the file's content hash about once a second, so a same-size, same-mtime edit is still seen. A half-written or invalid file is rejected and the previous list stays in force; the error is shown in `/status` as `allowlist.allowlist_error` (with `allowlist_keys` and `allowlist_age_s`), and nothing is re-logged until the file changes again.
- **Revocation is not instant**: it takes effect within about a second of the replace, and a request already past the check finishes.
- The bridge refuses an allowlist (at start; on reload it keeps the old list) that group or others can write (`chmod 644` or tighter), and refuses a node key file that group or others can read (`chmod 600`), as ssh does. Secret key bytes are cleared from memory after use.

## The network edge

The port is reachable before any signature is checked, so the server limits what an unauthenticated peer can cost:

- **Connections:** at most 32 at once, and at most 4 from any one IP (16 from loopback, where a local terminator may front many clients), so one host cannot take every slot; the rest get an immediate `503`. One legitimate node needs one connection, two while a retry overlaps a slow reply.
- **Requests:** header lines of at most 8 KiB, at most 64 headers (`431` beyond), a body of at most 64 KiB (`413`), `Content-Type: application/json` required on `POST /ingest` (`415`), a 3-second budget for the request line and headers, one 10-second budget for the whole request however slowly the peer sends, and a 5-second budget for writing the whole reply (so a client that reads a byte at a time cannot hold a slot while the 44 KB guide drains). An unparseable, negative or conflicting `Content-Length` is a `400`, and a length over the cap is a `413`, both decided from the headers before any body buffer exists.
- **Per source IP:** 20 requests a second (burst 40) for non-loopback peers, before any parsing (`429`). Loopback is exempt; a local TLS terminator should apply its own limits.
- **No browser access to ingest:** `POST /ingest` sends no CORS headers, refuses non-JSON content types, and answers `OPTIONS` with `405`, so a web page cannot drive it from someone's browser. Read-only `GET` routes still send `Access-Control-Allow-Origin: *`, but see the next point.
- **Read detail is privileged:** `/status` and `/sources` return counts only to anyone else; node names, vectors, metrics and allowlist health go to loopback callers or to a `GET` carrying the signed headers above. Set `status_auth` when a local TLS terminator makes every client look like loopback, so only signed requests see detail.

Refusal logging is limited to one line per reason per second with a count of the suppressed lines, so a flood cannot flood the log. Per-IP `429`s and connection-cap `503`s are counted apart from authentication refusals (`throttled`, `busy_refused`, `auth_refused` in `/status`).

## The clock (no RTC)

A Seed or Pi has no battery-backed clock and starts at 1970 until NTP runs. Signatures depend on time, so until the system clock is past a fixed floor (2026-05-28) the bridge answers signed requests with `clock_not_set` (503, transient) rather than judging them against a bogus clock, and the relay waits instead of sending readings stamped 1970. The "process start" used by the restart-replay rule is taken when the clock first becomes real, so replay protection is in force from that moment. `/status` shows `allowlist.clock_synced`. Once the clock is set, ordinary skew beyond `auth_window` is `stale_timestamp`.

## Start-up posture (fail closed)

With no allowlist and no token the bridge is **open**. It refuses to start in that state unless it is bound to loopback (the default, `127.0.0.1:8048`) or you pass `insecure_open`. So setting `api_bind = 0.0.0.0:8048` without an allowlist is an error, not a silent hole. The reverse case, auth configured but the bind still loopback (an upgrade from 0.1), logs a `NOTICE`; see **Setup**, 'Upgrading from 0.1'.

## The shared token is deprecated

`token` (the old `X-Bridge-Token`) is a single secret shared by every sender. It names no one and cannot be revoked per node, and a request that uses it has no node identity, so the `source` binding does not apply to it (the per-source cog quota and name checks still do). Passed as `--token` it is visible to every user in `ps`; set the `BRIDGE_TOKEN` environment variable instead, or better, do not use it. It is **off by default**: with `token` set and `allow_shared_token` false, the bridge refuses all ingest (`shared_token_disabled`) rather than silently accepting it. To keep old senders working while you migrate, set `allow_shared_token = true`; with an allowlist set, signed and token requests are then both accepted. Turn it off once every sender signs.

## The link: TLS options

The cog speaks plain HTTP. Signing gives authentication and integrity (a tampered body or replayed request is refused), but **not confidentiality**: readings, which can include heart rate or presence, cross the wire in clear. Pick one:

1. **Tailscale / WireGuard (recommended).** Tailscale is in the Seed base OS (COG-002). Send to the Seed's tailnet address (`http://100.x.y.z:8048`); the tunnel encrypts and authenticates the link, and the allowlist adds per-node identity on top.
2. **A TLS terminator.** Put Caddy, nginx or stunnel in front of `:8048` on the Seed and bind the bridge to `127.0.0.1:8048`. The sending side then needs a TLS client; `cog-bridge --relay-to` and `weft-bridge-send` are plain HTTP, so point them at a local `stunnel` client or use option 1. Behind a terminator every client appears as loopback, so also set `status_auth` and rely on the terminator for per-client rate limits.
3. **A trusted wired link only.** USB-gadget (`169.254.42.1`) or a private switch you control.

TLS is not built into the cog: it would add a TLS stack (rustls plus a crypto backend) to a cross-compiled armhf binary for a link that Tailscale already covers.

## Other limits

- **Keep it on the Seed if you don't need remote sources.** The default `api_bind = 127.0.0.1:8048` accepts only local readings.
- **Readings are clamped, not trusted.** The vector is forced to 8 numbers in 0 to 1; `source` and `cog` are 1-64 of `A-Za-z0-9._-` (no `/`, so two names cannot collide into one store key); `metrics` over 4 KiB are dropped. A valid reading from an allowed node is stored as given.
- **`/guide` and the counts are open.** `GET /guide` and the summary counts in `/status` and `/sources` need no signature.
- **Not a Seed enrolment.** A node is a *source*, not a Seed. It gets no pairing and no mesh membership (COG-005: no fake Seed).
- **Protect the secret key.** Anyone holding `node.key` can write as that node. Never commit it; revoke by deleting the allowlist line.

This is not a medical device, and the metrics it carries may include health data (heart rate) or where people are (presence). Protect the Seed and the network accordingly.
