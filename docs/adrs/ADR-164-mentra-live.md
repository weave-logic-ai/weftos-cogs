# ADR-164: mentra-live, a companion bridge that joins Mentra Live glasses to the fleet

**Status:** Proposed
**Date:** 2026-10-03
**Cog:** `mentra-live` 0.1.0

## Context

Mentra Live smart glasses run Android (the MentraOS ASG client). The cog's own description says they cannot run a cog themselves (`src/cogs/mentra-live/cog.toml`, `src/main.rs` header). We want them to show up on the mesh as a node, with honest liveness, before anything is built on top (HUD, sensors, audio).

COG-010 already defines how edge nodes appear: they check in with `POST /fleet/heartbeat` `{id, kind, sensor, rssi, battery, fw}`, the host keeps a TTL'd roster (online if seen within 60 s, dropped after 1 h), and the console Network tab shows it. COG-010 says the transport is the node's choice and the host never reaches out. COG-007 and ADR-160 cover the separate case of external nodes writing into the Seed store. COG-011 covers bridge authentication. The glasses are an Android device, not a microcontroller running `clawft-edge-pad`, so the firmware path in COG-010 decision 3 does not apply to them.

## Decision

1. **The cog runs on a companion that can reach the glasses over ADB**, not on the glasses. The companion is a Mac or the Seed (the guide's Start page names both). Target selection in `src/adb.rs` (`AdbSource::connect`): `--addr ip:port` (ADB-over-TCP, the cog runs `adb connect` first) wins, then `--serial`, then the first device in `adb devices`. The `adb` binary is resolved from `--adb`, then `$ADB`, then known SDK and system paths, then `PATH`.
2. **Telemetry per poll is three things, all read-only.** Presence is `adb get-state == "device"`. Link health is the round-trip time of that `get-state` call. Battery is parsed from `adb shell dumpsys battery`: level (normalised by scale), charging state, plug type (none, AC, USB, wireless, other), pack temperature and voltage. Model and firmware come once from `getprop ro.product.model` and `ro.build.display.id`. Every adb call has a 5 s timeout (`adb.rs`, `run`). Poll interval is 1 to 120 s, default 5 (`cog.toml`, `main.rs`).
3. **Three outputs** (`src/main.rs`, `src/export.rs`):
   - a JSON report per poll on stdout;
   - an 8-float store vector `[online, battery/100, charging, on_power, rtt/2000, temp/60, voltage/5, reserved]`, clamped to 0-1, POSTed to `127.0.0.1:80/api/v1/store/ingest` as store id 26 with `dedup: true`, reading the reply only to Content-Length (the ADR-158 finding). Sent only while the glasses are online;
   - a read-only HTTP export on `0.0.0.0:8061` (`[api] bind_port`, `--api-bind`): `/status` (latest report), `/raw` (battery percent trace, last 1800 s), `/guide`, `/healthz`, with `Access-Control-Allow-Origin: *`.
4. **The fleet heartbeat is how the glasses join the mesh.** After each successful poll where the glasses are online, `src/heartbeat.rs` posts `{id, kind: "glasses", sensor: <model>, fw, battery?, ip?}` to `<host>/fleet/heartbeat`. The host is `--host`, else `$WEFTOS_HOST`, else `http://127.0.0.1:9480`. The node id defaults to `mentra-01` (`--node-id`). `joined` in the report is true only when that post got a 2xx. A failed heartbeat is logged and is not fatal. No heartbeat is sent while the glasses are offline, so the host's TTL ages the node out and liveness is never faked. `rssi` is not sent.
5. **Fail-honest.** With no `adb` or no device, `--once` prints `status: no_source` and exits; continuous mode keeps retrying the connect and keeps the node offline. A poll error is reported as `no_source` and marks the state offline. `--simulate` swaps in synthetic glasses (battery drains and recharges, always online) for testing without hardware.
6. **Console limits.** `allowed_commands = ["--once", "--once --simulate", "--help"]`, `max_runtime_secs = 20`, `output_limit_bytes = 65536`. 20 s is enough for one-shot use: it covers a connect plus identity reads plus battery poll at 5 s per adb call in the normal case, and the continuous loop is not meant to run under the console.
7. **Category.** `cog.toml` says `category = "wearable"`: a body-worn device that carries its own sensors (glasses, watches, rings, bands). This is a new category, registered in `scripts/cog_lint.py` and defined in COG-008 (Categories). It is deliberately not `network` (the `bridge` cog's category): the cog links the glasses, but what it represents is the wearable, not the link.
8. **It ships an ADR-104 guide**: `guide/guide.toml` plus five pages (start, connect, telemetry, troubleshoot, api), served at `GET :8061/guide`. It is a software bridge, so the guide's diagram is a signal flow (glasses, ADB, cog, store and fleet, node online) instead of a pinout.

## Why a cog and not firmware

- The glasses are an Android device. There is no cog runtime or `clawft-edge-pad` target on them, and COG-005 says we do not port the Cognitum agent to other hardware. Reading them over a standard debug channel needs nothing installed on the glasses beyond ADB being enabled.
- The telemetry comes from stock Android commands (`get-state`, `dumpsys battery`, `getprop`), which a companion process can run. Firmware would add a second codebase to maintain for the same readings.
- COG-010 decision 2 lets the transport be the node's choice. A companion posting on the node's behalf fits that without changing the host.
- The cog gets the existing cog packaging, guide and console contract for free.

## Consequences

- Liveness means "the companion can reach the glasses over ADB", not "the glasses reached the host". If the companion dies, the node ages out even if the glasses are fine. The report distinguishes the two cases with `online` versus `joined`.
- The export on `0.0.0.0:8061` is reachable on the LAN. It carries serial, model, firmware, IP and battery data, with CORS open to any origin. Set `--api-bind 127.0.0.1:8061` on untrusted networks.
- The heartbeat is unauthenticated plain HTTP, as COG-010 specifies for the roster ("whether heartbeats should be authenticated ... deferred"). Anyone who can reach the host can post a heartbeat under any id. This cog adds no signing; the COG-011 scheme used by `bridge` is not applied here.
- ADB-over-Wi-Fi means debugging access to the glasses is exposed on the network while enabled. Same-network use only.
- The store vector is one point per poll for one node under a fixed id (26). A second pair of glasses on the same Seed would share id 26 unless the id is made per-node.
- The cog polls with `adb` subprocesses, so a companion needs `adb` installed. The guide's Connect page covers lookup order and the `unauthorized` prompt.
- Telemetry-first: no HUD, sensor, camera or audio path exists yet. Nothing in this cog reads the glasses' sensors.

## Spatial evidence: not applicable

This cog has no `spatial.evidence.v1` adapter. Its outputs are device liveness, battery and link health. None of it is a position, range or bearing in the room, so no ADR-107 record type fits. The glasses' on-board sensors might eventually matter, but this cog does not read them, and any such adapter would belong to the sensor cogs built on top of the joined node.

## Open questions

Not established from the repo, so left open rather than stated as fact:

- Whether the Mentra Live firmware allows ADB over USB and over TCP by default, and how ADB debugging is enabled on it. The guide tells the user to accept an on-device prompt, but no hardware verification is recorded.
- That `ro.product.model` and `ro.build.display.id` return meaningful values on the glasses, and that `dumpsys battery` reports every field (temperature and voltage are handled as optional).
- Real-world ADB-over-Wi-Fi stability, reconnect behaviour after the glasses sleep, and the battery cost of polling every 5 s.
- Whether the 5 s default interval and 60 s host TTL give sensible online/offline transitions on real hardware.
- Whether the cog has run on a Seed (the `v0-appliance` hardware requirement is declared) or only on a Mac, and whether `adb` is available on the Seed image. The binary name `cog-mentra-live-arm64` is declared, but no ARM verification is recorded (see the ARM-on-Pi rule).
- Per-node store ids and heartbeat authentication for more than one pair of glasses.
- Whether `rssi` should be sent in the heartbeat (the COG-010 roster has a field for it).
- Which later cog owns the HUD, sensor and audio paths.

## Alternatives

- **Firmware or an app on the glasses that posts its own heartbeat:** it would give true end-to-end liveness, but it means modifying the MentraOS ASG client, which is outside this repo. It remains possible later.
- **Use the MentraOS cloud or SDK to read device status:** not examined in this repo, so not evaluated here.
- **Route through the `bridge` cog (ADR-160):** `bridge` relays opaque 8-dim vectors into the store and has no fleet roster semantics, and the glasses need a roster entry. This cog posts to the COG-010 host endpoint directly.
- **Report heartbeats even when ADB is down:** rejected, because it would fake liveness (the explicit fail-honest rule in `cog.toml`).
