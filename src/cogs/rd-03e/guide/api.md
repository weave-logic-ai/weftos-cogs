# API reference

> Every field the cog reports, the store vector, the export endpoints, and the settings.

## stdout report (one per window)

| Field | Type | Meaning |
|---|---|---|
| `status` | string | `present`, `clear`, `no_frames`, `no_source` |
| `present` | bool | a target is present this report |
| `distance_cm` | int | distance to the target in cm (0 when no target) |
| `state` | int | raw module state: 0 none, 1 present, 2-8 vendor gesture codes |
| `state_name` | string | `no_target`, `present`, or `gesture` |
| `detections_per_min` | int | presence onsets over the last 60 s |
| `total_detections` | int | presence onsets since the cog started |
| `quiet_s` | number/null | seconds since the last detection |
| `source` | string | serial/sim description |

## Store vector (id 24)

`[present, distance_cm/600, state/8, detections_per_min/60, quiet_s/60, 0, 0, 0]`, each clamped 0..1.

## Export (`http://<seed>:8050`)

| Endpoint | Returns |
|---|---|
| `GET /status` | the latest report |
| `GET /raw` | `{source, fs, samples:[{t_ms, v}]}` — the last ~15 s of distance trace |
| `GET /guide` | this guide bundle (`{toml, pages, images}`) |
| `GET /healthz` | `{"ok":true}` |

CORS is open (`*`) so a browser build can read it.

## Spatial evidence (optional build)

A cog built with the cargo feature `spatial-evidence` can also write each reading as a `spatial.evidence.v1` `radar_range` line for the spatial engine (WeftOS-spatial ADR-107 §7.1): something reflects at this range, somewhere inside the beam. The standard build has none of this and ignores these flags. Even in a feature build it stays off until `--spatial-out` is given. They are command-line flags only, not `cog.toml` settings.

| Flag | Meaning |
|---|---|
| `--spatial-out export` | Serve the last 30 s as JSON lines at `GET /spatial?seconds=N` on the export (continuous mode only) |
| `--spatial-out FILE` | Append JSON lines to `FILE` (not under `/dev`, `/proc`, `/sys`) |
| `--radar-pose x,y,z,yaw_deg[,pitch_deg]` | Antenna position in `room_enu` metres (east, north, height); heading in degrees counter-clockwise from east (0 east, 90 north); tilt, negative = down (0 if omitted) |
| `--spatial-region ID` | Region id, for example `region/urth/meso/test-room` |
| `--spatial-source-id ID` | Device id, default `rd-03e`. Never a person |
| `--spatial-uncertainty-m U` | 1σ range uncertainty, default 0.1 m (assumed, not measured) |
| `--spatial-max-hz F` | Most lines per second, default 2, up to 20 |

- Only `state` 1 (present) with a distance above 0 becomes a line. No-target frames send nothing, and neither do the gesture codes (2-8), whose distance meaning the vendor does not document.
- `fov_deg` is `[40, 90]`: the datasheet's ranging beam, azimuth ±20° and elevation ±45° (Ai-Thinker Rd-03E Specification V1.0.0, §1 and §7.2), with the module mounted on a wall as shown there. These are vendor figures, not measured here.
- `targets` is always 1: the module reports one target, the nearest.
- `proof` is `MEASURED` for UART frames and `SYNTHETIC` under `--simulate`.

**No pose, no evidence.** Without `--radar-pose` or `--spatial-region` the cog keeps running but writes no evidence. Reports then have `"spatial":"no_pose"` (or `"no_region"`) and a `reasons` entry saying why. While emitting it is `"spatial":"emitting"`; a failed file write gives `"write_error"`. `no_source` lines and builds without the feature have no `spatial` field. A bad spatial flag exits with status 2.

```json
{"schema":"spatial.evidence.v1","type":"radar_range","t_ns":1759500004100000000,
 "frame":"room_enu","region":"region/urth/meso/test-room","source_id":"rd03e-1",
 "uncertainty_m":0.3,"provenance":{"receipt":"rd03e-1:000007","producer":"rd-03e@0.1.0",
 "proof":"MEASURED"},"position":[0.1,1.83,1.0],"yaw_deg":0.0,"fov_deg":[40.0,90.0],
 "range_m":2.45,"targets":1}
```

Tests (10 more with the feature): the ADR-107 example line reproduced (same keys and order, with the datasheet beam); yaw 0 → +x and yaw 90 → +y; pitch sent only when set; only present frames with a distance, and the rate limit; refusal without a pose or region; `SYNTHETIC` versus `MEASURED`; pose and flag bounds; the file sink; and the `/spatial` ring. Lines from `--once --simulate` parse with `weftos-spatial-core` (feat/spatial-workspace `fc4b0ad28`) and ingest with its `RadarRangeModel`.

## Settings (cog.toml / CLI)

| Setting | CLI | Default |
|---|---|---|
| interval | `--interval` | 1 s |
| window | `--window` | 2 s |
| port | `--port` | autodetect (`/dev/ttyUSB0`) |
| baud | `--baud` | 256000 |
| simulate | `--simulate` | off |
| once | `--once` | off |
