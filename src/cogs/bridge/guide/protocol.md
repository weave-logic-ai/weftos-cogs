# Protocol

> The reading a sender posts, and how the bridge turns it into a store vector.

## The reading

A sender posts JSON to `POST /ingest` on the bridge:

```json
{
  "source": "pi5",
  "cog": "sen0628-tof",
  "ts_ms": 1790900000000,
  "vector": [0.34, 1.0, 1.0, 0.1, 0.08, 0.68, 0.68, 0.34],
  "metrics": { "nearest_mm": 1193, "presence": true }
}
```

| Field | Required | Meaning |
|---|---|---|
| `source` | yes | the node's name; 1-64 of `A-Za-z0-9._-` (no `/`); with signing it must equal the node's name |
| `cog` | yes | the originating cog id; same charset; one source may use up to 8 distinct cogs |
| `vector` | yes | 1 to 8 numbers; the cog's store vector |
| `ts_ms` | no | the reading's time; the bridge uses now if absent |
| `metrics` | no | named values kept in the bridge report and log, not in the store |

## What the bridge does

1. **Validates:** `source` and `cog` present and not too long; `vector` is 1–8 finite numbers. Each number is clamped to 0–1 and the vector is padded to 8, because the Seed store is 8-dimensional.
2. **Assigns a store id.** The first `(source, cog)` pair gets `base_store_id` (default 30); the next pair gets 31, and so on, up to `max_sources`. The same pair always maps to the same id, so a source's points land together in the store.
3. **Writes it** to the Seed store over loopback (`POST 127.0.0.1:80/api/v1/store/ingest`, `dedup: true`).
4. **Records** the source, the last vector, the metrics and a count, shown in `/status` and `/sources`.

## Why only 8 numbers

The Seed store holds 8-dimensional vectors. A full ECG waveform or an 8×8 depth frame does not fit and should not: the summary vector is what belongs in memory, and the detailed signal stays on the originating board's own export (its `/frames`, `/raw.csv`). This is the same rule every sensor cog follows.

## Signed requests

When the bridge has an allowlist, every `POST /ingest` carries four headers:

| Header | Value |
|---|---|
| `X-Bridge-Node` | the node's name, as in the allowlist (`[A-Za-z0-9._-]`, up to 64) |
| `X-Bridge-Timestamp` | sender time, Unix milliseconds |
| `X-Bridge-Nonce` | 16 to 64 random alphanumerics, never reused (the cog uses 32 hex) |
| `X-Bridge-Signature` | Ed25519 signature, 128 hex characters |

The signature covers these UTF-8 bytes, joined by single `\n` characters, with no trailing newline:

```text
weft-bridge-v1
POST
/ingest
<node>
<timestamp ms>
<nonce>
<lowercase hex SHA-256 of the exact request body bytes>
```

`path` is the request target as sent. The signature is made with the 32-byte seed in the node's key file (standard Ed25519, RFC 8032). `cog-bridge keygen` makes the key and prints the public half for the allowlist. The same code signs the relay's requests (`relay.rs`), so it is the reference implementation for any other sender.

## Outbound relay (phase 2)

With `relay_to` set, the bridge also republishes **this Seed's own sensor stream** to another bridge receiver. Each tick it reads `GET <seed_url>/api/v1/sensor/stream`, takes the first eight `samples[].normalized` values as the vector, adds `healthy`, `sample_count` and `sample_rate_hz` as metrics, and posts `{"source": node_id, "cog": relay_cog, ...}` signed with `key_file`.

- **Bounded buffer.** Readings wait in a buffer of `relay_buffer` (default 256). If the receiver is down and the buffer fills, the oldest are dropped (counted in the log), so memory stays flat.
- **Backoff.** A failed send is retried after 1 s, then 2, 4, 8, ... up to 60 s (each with up to 20% random jitter so nodes do not retry in lockstep), oldest reading first; one success resets it. A backlog drains at about 5 requests a second, inside the receiver's per-node limit. A permanent refusal (HTTP 4xx other than 408 or 429, such as `unknown_node`) drops that reading instead of retrying it forever.
- **Receiver.** Any bridge: another Seed, or the bridge run on a WeftOS node with `no_store = true`, which keeps readings in its `/status` for WeftOS to read.

## Sources

```diagram
flow
```

Each distinct node-and-cog pair is one source with one store id. Running the same cog on two boards gives two sources (`pi5/sen0628-tof`, `orangepi/sen0628-tof`), each with its own id.
