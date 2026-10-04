# Output and store vector

## JSON per window (stdout)

One object per line. Example (continuous, motion held):

```json
{
  "status": "present",
  "source": "OUT on /dev/gpiochip0 line 17",
  "simulated": false,
  "present": true,
  "warming": false,
  "motion_events": 1,
  "active_fraction": 0.74,
  "seconds_since_motion": 0.3,
  "detections_per_min": 5,
  "total_detections": 42,
  "samples": 100,
  "window_s": 1.0,
  "motion_amplitude": null,
  "signal": null,
  "noise": null,
  "export": "http://127.0.0.1:8053/status",
  "timestamp": 1790000000
}
```

| Field | Meaning |
|---|---|
| `status` | `no_source`, `warming`, `present`, or `clear` |
| `present` | current held OUT state |
| `warming` | true during the first ~9 s after start (readings invalid) |
| `motion_events` | rising edges (motion onsets) within this window |
| `active_fraction` | fraction of the window OUT was high (0.0–1.0) |
| `seconds_since_motion` | seconds since the last motion onset (null if none yet) |
| `detections_per_min` | motion onsets in the last 60 s |
| `motion_amplitude` / `signal` / `noise` | optional UART telemetry; null unless `--uart` parses a frame |

A missing OUT line gives `"status":"no_source"` with null values — never a fabricated reading.

## Store vector (POST to the Seed)

When motion data is available the cog POSTs `{"vectors":[[27, [8 floats]]],"dedup":true}` to
`127.0.0.1:80/api/v1/store/ingest`. The 8 floats (all clamped 0.0–1.0) are:

| Index | Value | Normalisation |
|---|---|---|
| 0 | present | 0 or 1 |
| 1 | motion events per minute | ÷ 30 |
| 2 | active fraction | already 0–1 |
| 3 | seconds since motion | ÷ 60 (quiet minutes), 1.0 if none |
| 4 | motion amplitude (UART) | ÷ 255, else 0 |
| 5 | signal (UART) | ÷ 1024, else 0 |
| 6 | warming | 0 or 1 |
| 7 | reserved | 0 |

The per-minute (÷30), quiet-time (÷60), signal (÷1024) and amplitude (÷255) constants are initial
estimates and should be tuned once real-world ranges are observed.
