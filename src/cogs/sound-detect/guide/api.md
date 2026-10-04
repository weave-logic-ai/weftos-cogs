# API reference

> Every field the cog reports, the store vector, the export endpoints, and the settings.

## stdout report (one per window)

| Field | Type | Meaning |
|---|---|---|
| `status` | string | `sound`, `quiet`, `no_source`, `no_samples` |
| `present` | bool | sound in this window |
| `events_per_min` | number | threshold crossings over the last 60 s |
| `events_in_window` | int | events within this window |
| `total_events` | int | since the cog started |
| `quiet_s` | number/null | seconds since the last event |
| `activity_pct` | number | % of window samples above the threshold |
| `level_v` | number | mean OUT volts this window |
| `peak_v` | number | peak OUT volts this window |
| `threshold_v` | number | the active threshold |
| `source` | string | ADC/sim description |

## Store vector (id 23)

`[present, activity, events_per_min/60, peak/3.3, quiet_s/60, level/3.3, threshold/3.3, 0]`, each clamped 0..1.

## Export (`http://<seed>:8049`)

| Endpoint | Returns |
|---|---|
| `GET /status` | the latest report |
| `GET /raw` | `{source, fs, samples:[{t_ms, v}]}` — the last ~15 s of OUT level |
| `GET /healthz` | `{"ok":true}` |

CORS is open (`*`) so a browser build can read it.

## Settings (cog.toml / CLI)

| Setting | CLI | Default |
|---|---|---|
| interval | `--interval` | 5 s |
| window | `--window` | 5 s |
| sample_rate | `--sample-rate` | 1000 Hz |
| threshold | `--threshold` | 1.5 V |
| channel | `--channel` | 1 (A1) |
| i2c_bus / i2c_addr | `--i2c-bus` / `--i2c-addr` | 1 / 72 |
| simulate | `--simulate` | off |
