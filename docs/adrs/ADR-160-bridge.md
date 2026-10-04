# ADR-160: bridge, linking external WeftOS nodes to the Seed store

**Status:** Proposed
**Date:** 2026-10-02
**Cog:** `bridge` 0.1.0 (amended in 0.2.0, see Amendments 1 to 3)

## Context

COG-007 decided to bridge external WeftOS nodes into the Seed's memory rather than port the Cognitum agent to other hardware. This ADR records the cog that does it.

## Decision

1. **`bridge` is an HTTP receiver on the Seed.** It listens on `0.0.0.0:8048` (configurable), accepts `POST /ingest`, and exports `/status`, `/sources` and `/guide`. It reads no hardware.
2. **A reading is `{source, cog, ts_ms?, vector, metrics?}`.** `source` and `cog` are bounded strings; `vector` is 1–8 finite numbers, clamped to 0–1 and padded to the store's 8 dimensions.
3. **Each `(source, cog)` gets its own store id** from `base_store_id` up to `max_sources`. The same pair always maps to the same id.
4. **It writes to the Seed store over loopback** (`POST 127.0.0.1:80/api/v1/store/ingest`, `dedup: true`), stopping the response read at Content-Length (the agent keeps the socket open — ADR-158 finding). `metrics` are kept in the report and log, not the store.
5. **The source node sends; the bridge receives.** The WeftOS-side sender `weft-bridge-send` polls a cog's `/status` (which now carries the cog's 8-float `vector`) and posts to the bridge. The bridge is generic: it relays an opaque 8-dim point and does not interpret it.
6. **Optional shared token.** With `token` set, a reading must carry `X-Bridge-Token`. Default is open on the LAN; `127.0.0.1` keeps it on the Seed.
7. **Ships an ADR-104 sensor guide** at `/guide` (seven pages, a flow diagram).

## Consequences

- An external board running our cogs contributes to the Seed memory with no proprietary binary moved; it is a source, not a Seed (COG-005 holds).
- The sensor cogs now expose their store vector in `/status`, which is also useful for debugging.
- Phase 2 (not here): an outbound mode that republishes the Seed's own sensor stream to the WeftOS mesh.
- Trust is coarse in v1: reaching the port (plus an optional shared token) is the whole gate. Per-node identity is future work; the guide's Security page says so.

## Alternatives

- **Native ESP32 packet to UDP 5006** from the external node (no bridge cog inbound). Kept as a shortcut for simple data; the bridge is preferred for richer payloads, several sources, per-source ids and a token. The bridge avoids 5006 to dodge the cogs#14 contention bug.
- **Port the agent** (rejected in COG-007).

## Amendment 1 (0.2.0): per-node identity and outbound relay

**Date:** 2026-10-02. Supersedes decision 6 and the "Trust is coarse" consequence. Records COG-011.

1. **Per-node Ed25519 identity.** Each sending node has a keypair. The bridge holds an allowlist file of `NODE PUBKEY_HEX` lines (`--allowlist`, re-read on change). With an allowlist set, `POST /ingest` must carry `X-Bridge-Node`, `X-Bridge-Timestamp`, `X-Bridge-Nonce` and `X-Bridge-Signature`; the signature covers `weft-bridge-v1`, method, path, node, timestamp, nonce and the SHA-256 of the body (exact bytes in the guide's Protocol page).
2. **Replay protection.** Timestamp within `auth_window` (default 60 s) of the bridge clock, plus a nonce cache that only records nonces whose signature verified, expires entries with the window and is capped at 4096 (full = refuse, 503).
3. **Clear refusals.** Each failure returns `{"ok":false,"error":<code>,"detail":...}`: `missing_signature`, `malformed_auth`, `unknown_node`, `stale_timestamp`, `bad_signature`, `replayed_nonce`, `nonce_cache_full`, `source_mismatch`, `shared_token_disabled`, `bad_token`. Refusals are counted in `/status` and logged.
4. **A node may only send its own name** as `source`, so a compromised node cannot write another node's store ids.
5. **The shared `X-Bridge-Token` is deprecated.** It is honoured only with `--allow-shared-token`. A token with that flag off and no allowlist locks ingest rather than silently accepting it. With neither allowlist nor token the bridge is still open (legacy), with a start-up warning. Token comparison is now case-exact and constant-time (0.1.0 lower-cased it).
6. **TLS is not built in.** The cog stays std-only HTTP. Confidentiality comes from Tailscale/WireGuard (in the Seed base OS, COG-002) or a TLS terminator in front of `:8048`; the guide's Security page lists the options. Signing gives authentication and integrity, not secrecy.
7. **Phase 2 outbound relay.** `--relay-to <url>` republishes the Seed's own `GET /api/v1/sensor/stream` (first eight `normalized` values plus health metrics) to a bridge receiver, signed with `--node-id` and `--key-file`. A bounded buffer (`--relay-buffer`, oldest dropped) and 1 s to 60 s exponential backoff keep a dead receiver from exhausting memory; permanent 4xx refusals drop the reading. `--no-store` lets the bridge run as the receiver on a WeftOS node.
8. **`cog-bridge keygen <node> <key-file>`** writes a secret key (mode 0600, never overwrites) and prints the allowlist line.

Consequences: the WeftOS-side sender `weft-bridge-send` does not sign yet; until it does, an allowlisted bridge needs `--allow-shared-token` for it, or the node relays with `cog-bridge --relay-to`. New dependencies: `ed25519-dalek` and `sha2` (std features only). The relay assumes the Seed's `sensor/stream` returns the JSON `samples` shape that `cog-sensor-sources` parses; bench testing showed a 501 on some firmware, in which case the relay logs `seed: ...` errors and sends nothing.

## Amendment 2 (0.2.0): hardening after review

**Date:** 2026-10-02. Records the COG-011 review fixes. Supersedes decision 1's default bind and Amendment 1 point 5's "still open (legacy)".

1. **Pre-auth resource limits.** At most 32 concurrent connections (extra get `503`); request and header lines capped at 8 KiB, 64 headers, 64 KiB body; one 10 s deadline for the whole request (a `Read` wrapper re-arms the socket timeout from the remaining time, so dripping bytes cannot extend it); per-source-IP throttle (20/s, burst 40, loopback exempt) before parsing; handler threads are started with `Builder::spawn` and socket clones are error-handled, because the cog builds with `panic = "abort"`. Replies are followed by a half-close and a short drain so the peer sees the reply, not a reset.
2. **Fail closed.** The default `api_bind` is now `127.0.0.1:8048`. With no allowlist and no token the bridge refuses to start on a non-loopback bind unless `--insecure-open`. `POST /ingest` sends no CORS headers, requires `Content-Type: application/json` (`415`), and `OPTIONS` is `405`; read-only GETs keep `Access-Control-Allow-Origin: *`.
3. **Per-node budgets.** 8 verified requests/s (burst 16) and 512 live nonces per node, with the global nonce cap as a backstop. Replays are refused before they spend rate budget, so replaying a captured request cannot starve the node. The COG-011 claim that a compromised node exposes only that node is now true: it can write only its own `source`, at a bounded rate, within bounded memory.
4. **Names and quotas.** `source` and `cog` are `[A-Za-z0-9._-]`, 1-64 (no `/`, so `a/b` + `c` cannot collide with `a` + `b/c`). One source may hold at most 8 cogs of store ids. `metrics` over 4 KiB are dropped. The `source == node` binding cannot apply to the legacy token path (no identity); the quotas still do.
5. **Allowlist lifecycle.** Reloaded on content-hash change (checked about once a second), not only mtime. A failed reload keeps the old keys, is not retried until the file changes, and is reported in `/status` (`allowlist_keys`, `allowlist_error`, `allowlist_age_s`). Revoke by atomic replace (write then `mv`). The file must not be group/other writable.
6. **Read privilege.** `/status` and `/sources` give names, vectors and metrics only to loopback callers or a signed `GET`; others get counts. `--status-auth` removes the loopback special case (local TLS terminator).
7. **Verification.** `verify_strict`; the signature check runs outside the auth lock; timestamp arithmetic is `i128` and timestamps are digits-only, at most year 2100; requests with a timestamp older than the process start are refused (`predates_restart`), closing replay across a restart.
8. **Secrets.** A node key file readable by group/others is refused; seeds and key text are zeroized after use. `--token` is visible in `ps`; `BRIDGE_TOKEN` is read from the environment as an alternative.
9. **Relay.** Backoff has +-20% jitter; a backlog drains at about 5 requests/s; Seed reads and the receiver round trip have whole-request deadlines and bounded connects.

Residual risks: the rate limit and nonce cap are per node, so the sum across many allowlisted nodes can still reach the global cap (`nonce_cache_full`); loopback is exempt from the IP throttle; plain HTTP carries readings in clear (see Amendment 1, TLS).

## Amendment 3 (0.2.0): upgrade safety and second review

**Date:** 2026-10-02.

1. **Upgrade from 0.1.** The default bind change to loopback can silently stop remote ingest on an upgraded install. The bridge cannot tell a stored `api_bind` from a default, so it neither guesses the old address nor exits (a supervised cog that exits is restarted in a loop); it starts, logs a `NOTICE` when auth is configured but the bind is loopback, and reports `bind_scope` in `/status`. The guide's Setup page has an "Upgrading from 0.1" section. `[api] bind_loopback_only` stays `false`, with a comment, because the cog serves the network once configured.
2. **Relay classification.** `predates_restart` is transient (retried, re-signed each attempt); `stale_timestamp` is retried at most 10 consecutive times with a clock-skew warning, then the reading is dropped; other 4xx are permanent; 429/408/5xx/connect errors are transient.
3. **Slot exhaustion.** At most 4 concurrent connections per IP (16 loopback); a 3 s header deadline inside the 10 s request deadline; the reply write has its own 5 s overall deadline; `Content-Length` that is unparseable, negative or repeated with different values is `400`, and an over-cap value is `413` before allocation.
4. **Logging and counters.** Refusal logs are limited to one line per reason per second with a suppressed count; per-IP `429` (`throttled`) and connection-cap `503` (`busy_refused`) are counted apart from `auth_refused`.
5. **No RTC.** Signed requests get `clock_not_set` (503) until the clock passes a floor (2026-05-28); the relay waits; the restart reference time is taken when the clock first becomes real.
6. A `CacheFull` refusal no longer spends a rate token.
7. Residual risk (later item): per-IP keying is per address; for IPv6 it should key by /64, since one host can rotate through a /64 to dodge the per-IP connection and rate limits.
