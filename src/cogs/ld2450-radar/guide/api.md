# API reference

> The output line, the read-only export, config keys, flags and console commands.

## Output line

One JSON object per window on stdout. An abridged line (values illustrative):

```json
{"schema":"weavelogic.cog-output.v0","cog":"ld2450-radar","version":"0.1.0",
 "timestamp":1790713871,"timestamp_ms":1790713871540,"window_start_ms":1790713870521,
 "sensor_class":"radar","source":{"kind":"ld2450-uart","device":"/dev/serial0","verified":true,"simulated":false},
 "health":"ok","quality":1.0,"reasons":[],"frame":"radar_local",
 "target_count":1,"max_target_count":1,"latest_frame_ms":1790713871498,
 "targets":[{"slot":1,"x_m":-0.782,"y_m":1.713,"speed_mps":-0.16,"resolution_mm":320}],
 "firmware":null,"tracking_mode":null,"frames":10,"frame_rate_hz":9.8,
 "parse_errors":0,"bad_tail":0,"bad_ack_len":0,"implausible_targets":0,
 "resync_bytes":0,"bytes_read":300,"acks":0}
```

| Field | Meaning |
|---|---|
| `health` | `ok`, `degraded` (parse errors or implausible targets) or `no_source` |
| `quality` | Bytes in valid frames ÷ all bytes consumed; `null` when `no_source`. Link integrity only, not position accuracy |
| `reasons` | Why health is what it is: `no_bytes`, `parse_errors`, `simulated`, `config_error: …` and so on |
| `source` | `kind` (`ld2450-uart`, `ld2450-simulated`, `ld2450-replay`), `device`, `verified` (real UART frames this window), `simulated` |
| `frame` | Always `radar_local` → see **Placement** |
| `targets` | The latest frame's targets: `slot` 1-3, `x_m`, `y_m`, `speed_mps`, `resolution_mm` |
| `target_count` | Targets in the latest frame; `null` if no frame arrived |
| `max_target_count` | Most targets in any frame of the window |
| `latest_frame_ms` | When the frame behind `targets` was parsed (Unix ms) |
| `frames`, `frame_rate_hz` | Report frames this window, and per second (`null` for a replay or an unopened device) |
| `parse_errors` | `bad_tail` + `bad_ack_len` |
| `implausible_targets` | Targets dropped as corrupt (y ≤ 0 or beyond 10 m) |
| `resync_bytes`, `bytes_read` | Bytes skipped looking for a frame; bytes read this window |
| `firmware`, `tracking_mode`, `acks` | Filled only when commands were sent |
| `timestamp`, `timestamp_ms`, `window_start_ms` | Unix seconds; window end and start in Unix ms |

An empty slot is not a target. With nobody in view `targets` is `[]` and `target_count` is 0. With no radar, `target_count` is `null`.

`resolution_mm` is passed through as the radar reports it. Hi-Link calls it the size of one distance gate and says it is fixed.

## Export (port 8052, continuous mode)

Read-only, GET only, `Access-Control-Allow-Origin: *`. It binds `127.0.0.1:8052` by default because it shows where people are. Set `api_bind` to `0.0.0.0:8052` only on a trusted network. It is not started by `--once`.

| Route | Returns |
|---|---|
| `/` | Plain-text index |
| `/status` | The latest output line. `{"health":"starting"}` before the first |
| `/targets` | `{"t_ms","targets":[...]}` of the latest frame |
| `/frames?seconds=N` | `{"source","frame","frames":[{t_ms,targets}]}` for the last N s (N up to 30, default 5) |
| `/guide` | This guide as JSON |

## Config keys

| Key | Default | Range |
|---|---|---|
| `interval` | 1 | 1-3600 s |
| `device` | `/dev/serial0` | a path under `/dev/` |
| `baud` | 256000 | 9600, 19200, 38400, 57600, 115200, 230400, 256000, 460800 |
| `query_firmware` | false | bool |
| `tracking_mode` | `keep` | `keep`, `single`, `multi` |
| `api_bind` | `127.0.0.1:8052` | host:port |
| `simulate` | false | bool |

- `query_firmware` sends enable-config, query tracking mode, read firmware, end-config. It changes nothing on the radar.
- `tracking_mode` `single` or `multi` sends the mode command. **That changes the radar's configuration**; Hi-Link lists tracking mode among the stored settings. `keep` sends nothing.

## CLI flags

`--once`, `--interval`, `--device`, `--baud`, `--query-firmware`, `--tracking-mode`, `--api-bind`, `--simulate`, `--replay FILE`, `--help`.

`--replay FILE` decodes captured raw UART bytes (up to 16 MiB) as fast as they parse, about `interval × 10` frames per line, then exits. It cannot be combined with `--simulate` or with commands.

## Console

Only these commands are accepted. The run is capped at 15 s and 64 KiB of output.

`--help`, `--once`, `--once --simulate`, `--once --interval 5`, `--once --device /dev/ttyUSB0`, `--once --query-firmware`

## Spatial evidence (optional build)

A cog built with the cargo feature `spatial-evidence` can also write each target as a `spatial.evidence.v1` `radar_track_point` line for the spatial engine (WeftOS-spatial ADR-107). The standard build has none of this. Even in a feature build it stays off until `--spatial-out` is given. These are command-line flags only; they are not in the Seed's config screen.

| Flag | Meaning |
|---|---|
| `--spatial-out export` | Serve the last 30 s as JSON lines at `GET /spatial?seconds=N` on the export (continuous mode) |
| `--spatial-out FILE` | Append JSON lines to `FILE` (not under `/dev`, `/proc`, `/sys`) |
| `--radar-pose x,y,z,yaw_deg,pitch_deg,x_sign` | Mount position in `room_enu` metres, heading in degrees counter-clockwise from east (0 east, 90 north), tilt in degrees (negative = tilted down; 0 if level), and `1` or `-1` from the step-right check → see **Placement**. The 5-number form without tilt still works (tilt 0). Every evidence line also carries a `sensor` block with this pose and the radar's ±35° × ±60° beam, so the spatial engine can place the person within the beam's height at that distance |
| `--spatial-region ID` | Region id, for example `region/urth/meso/test-room` |
| `--spatial-source-id ID` | Device id, default `ld2450-radar`. Never a person |
| `--spatial-uncertainty-m U` | 1σ position uncertainty, default 0.3 (assumed, not measured) |

**No pose, no evidence.** Without `--radar-pose` or `--spatial-region` the cog keeps running but writes no evidence. Every output line then has `"spatial":"no_pose"` (or `"no_region"`) and a `reasons` entry saying why. While emitting it is `"spatial":"emitting"`; a failed file write gives `"write_error"`. Builds without the feature have no `spatial` field.

One line per target, for example:

```json
{"schema":"spatial.evidence.v1","type":"radar_track_point","t_ns":1759500001250000000,
 "frame":"room_enu","region":"region/urth/meso/test-room","source_id":"ld2450-1",
 "uncertainty_m":0.15,"provenance":{"receipt":"ld2450-1:000123",
 "producer":"ld2450-radar@0.1.0","proof":"MEASURED"},"track":2,"position":[2.1,1.4,0.0]}
```

- `position` uses the placement page's transform, rounded to the millimetre. z is 0.0: the radar is 2-D.
- `track` is the radar's slot, not a person.
- `proof` is `MEASURED` for real frames and `SYNTHETIC` under `--simulate`.
- There is no `velocity`. The radar's speed is along the line of sight and its sign is not yet checked.
- Not available with `--replay`.

## Not sent to the store

The cog does not post to the Seed's `/api/v1/store/ingest`, matching `ld6002-radar`.

## Data flow

```diagram
flow
```
