# API reference

> Agent endpoints, the cog's frame export, report fields, config keys, flags and the store vector.

Replace `<seed>` with `169.254.42.1` (USB), or the Seed's LAN or tailnet address.

## Agent API (port 80)

Plain HTTP on port 80, with `Access-Control-Allow-Origin: *`. If your Seed refuses writes, add `Authorization: Bearer <pairing token>`. Port 8443 is self-signed TLS and browsers reject it.

| Method and path | Does |
|---|---|
| `GET /api/v1/apps` | Installed apps, with `running` |
| `GET /api/v1/apps/sen0628-tof/config` | Current config |
| `PUT /api/v1/apps/sen0628-tof/config` | **Replaces the whole config and restarts the cog.** Send every key. |
| `POST /api/v1/apps/sen0628-tof/start` | Start in continuous mode |
| `POST /api/v1/apps/sen0628-tof/stop` | Stop |
| `POST /api/v1/apps/sen0628-tof/console` | Run one allowed command: `{"command":"--once --simulate"}` |
| `GET /api/v1/apps/sen0628-tof/logs` | The cog's stdout, one report per line |

## Cog export (port 8047)

Read-only, GET only, `Access-Control-Allow-Origin: *`. It keeps the last 30 s of frames. It binds `0.0.0.0` by default and starts before the sensor is found.

| Route | Returns |
|---|---|
| `/` | Plain-text index |
| `/status` | The latest report (JSON). `{"status":"starting"}` before the first one. |
| `/frame` | The latest frame: `{"t_ms","side","mm":[...]}`. Empty `mm` before the first. |
| `/frames?seconds=N` | `{"source","frames":[...]}` for the last N s. N up to 30; default 5. |
| `/raw.csv?seconds=N` | CSV: `t_ms,side,z0..zN`. N up to 30; default 5. |
| `/guide` | This guide as JSON, served by the cog for companion apps |

Frames are row-major: X runs left to right, Y runs top to bottom, and the index is `y * side + x`. Values are millimetres. 0 means no target.

An abridged `/status` while running (values illustrative):

```json
{"status":"ok","source":"sen0628@/dev/i2c-1:0x33/8x8","mode":"8x8","frames":30,
 "frame_rate_hz":9.9,"read_errors":0,"valid_pct":96.9,
 "nearest":{"mm":1180,"x":2,"y":5},"median_mm":2410.0,"motion_mm":14.2,
 "background_learned":true,"presence":true,"occupied_zones":[34,35,42,43],
 "sectors":{"left":2390,"middle":1180,"right":2450},"mean_zone_noise_mm":8.4}
```

And a shortened `/frame`, 4x4 (illustrative):

```json
{"t_ms":1790000000000,"side":4,"mm":[2400,2410,2395,2380,2300,1210,1190,2350,
 2290,1200,1185,2340,2250,2260,2270,2280]}
```

## Report fields

| Field | Meaning |
|---|---|
| `status` | `ok`, `no_targets`, `no_frames`, or `no_source` (with `error`) |
| `source`, `mode` | Where frames came from, and `8x8` or `4x4` |
| `frames`, `frame_rate_hz` | Frames in this window, and the measured rate |
| `read_errors` | Failed frame reads since start |
| `valid_pct` | Percent of zones with a valid reading (20 mm to `max_range_mm`) |
| `nearest` | `{mm, x, y}` of the closest valid zone, or `null` |
| `median_mm` | Median of the valid zones, or `null` |
| `motion_mm` | Mean absolute change between consecutive frames, over zones valid in both |
| `background_learned` | `true` once the background is learned |
| `presence` | `true` when the background is learned and at least 2 zones are occupied |
| `occupied_zones` | Indices of zones more than `presence_mm` closer than the background |
| `sectors` | `{left, middle, right}`: nearest valid mm in each, or `null` |
| `mean_zone_noise_mm`, `zone_noise_mm` | Noise in mm: the mean, and one value per zone (`null` under 2 valid reads) |
| `background_mm` | The learned background per zone, or `null` |
| `frame` | The latest frame: `{t_ms, side, mm}` |
| `export`, `timestamp` | The `/frame` URL, or `null`; Unix seconds |

A `no_source` report has only `status`, `error` and `timestamp`. A `no_frames` report has `status`, `source`, `frames` (0), `read_errors` and `timestamp`.

## Config keys

| Key | Default | Range |
|---|---|---|
| `interval` | 1 | 1-60 s |
| `window` | 3 | 1-8 s |
| `mode` | `8` | `8` or `4` |
| `rate_hz` | 10 | 1-15 Hz |
| `i2c_bus` | 1 | 0-9 |
| `i2c_addr` | 51 | 48-51 |
| `max_range_mm` | 3500 | 100-4000 mm |
| `presence_mm` | 150 | 30-1000 mm |
| `learn_seconds` | 5 | 1-60 s |
| `api_bind` | `0.0.0.0:8047` | host:port |
| `simulate` | false | bool |

- `interval`: seconds between reports. `window`: seconds of frames in one `--once` run, after the mode set-up.
- `i2c_addr`: 48=0x30, 49=0x31, 50=0x32, 51=0x33.
- `api_bind`: `127.0.0.1:8047` keeps the export on the Seed.

## CLI flags

`--once`, `--interval`, `--window`, `--mode` (`8` or `4`), `--rate-hz`, `--i2c-bus`, `--i2c-addr` (48-51 or `0x30`-`0x33`), `--max-range-mm`, `--presence-mm`, `--learn-seconds`, `--api-bind`, `--simulate`, `--help`.

## Spatial evidence (optional build)

A cog built with the cargo feature `spatial-evidence` can also write depth frames as `spatial.evidence.v1` `tof_depth` lines for the spatial engine (WeftOS-spatial ADR-107). The standard build has none of this and ignores these flags. Even in a feature build it stays off until `--spatial-out` is given. They are command-line flags only, not config keys.

| Flag | Meaning |
|---|---|
| `--spatial-out export` | Serve the last 30 s as JSON lines at `GET /spatial?seconds=N` on the export (continuous mode) |
| `--spatial-out FILE` | Append JSON lines to `FILE` (not under `/dev`, `/proc`, `/sys`) |
| `--tof-pose x,y,z,yaw_deg,pitch_deg[,roll_deg[,x_sign]]` | Lens position in `room_enu` metres; heading in degrees counter-clockwise from east (0 east, 90 north); tilt (negative = down); roll about the lens axis, clockwise looking out (180 if the board is upside down); `x_sign` -1 if your hand on the right shows up in the left columns → see **Mounting** |
| `--spatial-region ID` | Region id, for example `region/urth/meso/test-room` |
| `--spatial-source-id ID` | Device id, default `sen0628-tof`. Never a person |
| `--spatial-uncertainty-m U` | 1σ range uncertainty, default 0.05 (assumed, not measured) |
| `--spatial-max-hz F` | Most lines per second, default 1, up to 15 |

**No pose, no evidence.** Without `--tof-pose` or `--spatial-region` the cog keeps running but writes no evidence. Reports then have `"spatial":"no_pose"` (or `"no_region"`) and a `reasons` entry saying why. While emitting it is `"spatial":"emitting"`; a failed file write gives `"write_error"`. Builds without the feature have no `spatial` field.

One line per frame, for example (4x4):

```json
{"schema":"spatial.evidence.v1","type":"tof_depth","t_ns":1759500004000000000,
 "frame":"room_enu","region":"region/urth/meso/test-room","source_id":"sen0628-1",
 "uncertainty_m":0.02,"provenance":{"receipt":"sen0628-1:000042",
 "producer":"sen0628-tof@0.1.0","proof":"MEASURED"},"position":[4.8,0.05,1.6],
 "yaw_deg":90.0,"pitch_deg":-35.0,"fov_deg":[60.0,60.0],"grid":[4,4],
 "range_mm":[0,0,0,0,3290,3105,3110,3302,2512,1190,1185,2530,2005,1072,1066,2011],
 "valid":[false,false,false,false,true,true,true,true,true,true,true,true,true,true,true,true]}
```

- Zones are in the same order as `/frame`: top row first, left column first, looking out of the lens.
- `valid` is the cog's own rule: 20 mm up to `max_range_mm`.
- `proof` is `MEASURED` for real frames and `SYNTHETIC` under `--simulate`.

## Console

Only these commands are accepted. The run is capped at 15 s and 64 KiB of output.

`--once`, `--once --simulate`, `--once --mode 4`, `--help`

## Store vector

Each report is sent by the cog to the agent, loopback only: `POST 127.0.0.1:80/api/v1/store/ingest` with `{"vectors":[[22,[8 floats]]],"dedup":true}`. Each float is clamped to 0-1. No report is stored for `no_frames`.

| # | Value |
|---|---|
| 0 | nearest mm `/ max_range_mm` (1 if none) |
| 1 | valid share (0-1) |
| 2 | presence (1 or 0) |
| 3 | occupied zones `/` total zones |
| 4 | `motion_mm / 500` |
| 5 | left sector mm `/ max_range_mm` (1 if none) |
| 6 | middle sector mm `/ max_range_mm` (1 if none) |
| 7 | right sector mm `/ max_range_mm` (1 if none) |

## The guide

`GET http://<seed>:8047/guide` returns `{"toml": "...", "pages": {id: markdown}}`. It is this guide, compiled into the cog.

## Data flow

```diagram
flow
```
