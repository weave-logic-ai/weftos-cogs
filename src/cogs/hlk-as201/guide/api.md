# API reference

> Every field the cog reports, the store vector, the export endpoints, and the settings.

## stdout report (one JSON line per interval)

| Field | Type | Meaning |
|---|---|---|
| `status` | string | `moving`, `still`, `no_frames`, or `no_source` |
| `accel_g` | {x,y,z} | acceleration per axis, g |
| `gyro_dps` | {x,y,z} | angular rate per axis, deg/s |
| `euler_deg` | {roll,pitch,yaw} | orientation, degrees |
| `mag_ut` | {x,y,z} | magnetic field per axis, µT |
| `quaternion` | {w,x,y,z} | orientation quaternion |
| `temp_c` | number | sensor temperature, °C |
| `pressure_hpa` | number | barometric pressure, hPa (10-axis parts) |
| `height_m` | number | barometric height, m (10-axis parts) |
| `mag_accuracy` | number | 0 = uncalibrated/interference … 3 = calibrated |
| `gyro_mag_dps` | number | magnitude of the angular-rate vector |
| `motion_events_per_min` | number | motion crossings over the last 60 s |
| `quiet_s` | number/null | seconds since the last motion event |
| `frame_protocol` | string | `HLK-AS201 Hi-Link protocol (confirmed, datasheet V1.1 2025-08-01)` |

## Store vector (id 25)

`[roll, pitch, yaw, ax, ay, az, gyro_mag, moving]`, each normalized 0..1.

## Export (`http://<seed>:8051`)

| Endpoint | Returns |
|---|---|
| `GET /status` | the latest report JSON |
| `GET /raw` | a recent angular-rate (motion) trace for a scope |
| `GET /guide` | this guide |
| `GET /healthz` | `{"ok":true}` |

CORS is open (`*`) so a browser build can read it.

## Settings (cog.toml / CLI)

| Setting | CLI | Default |
|---|---|---|
| interval | `--interval` | 1 s |
| window | `--window` | 2 s |
| port | `--port` | `/dev/ttyUSB0` |
| baud | `--baud` | 115200 |
| motion | `--motion` | 25 deg/s |
| simulate | `--simulate` | off |
| once | `--once` | — |
| help | `--help` | — |
