# What it reports

Each interval the cog emits one telemetry snapshot. The MVP scope is **presence + battery + link** — the signals that prove the node is alive and healthy.

| Field | Meaning | Source |
|---|---|---|
| `online` | the glasses are reachable right now | `adb get-state` == `device` |
| `battery_pct` | charge level, 0–100 | `dumpsys battery` level/scale |
| `charging` | pack is charging or full | `dumpsys battery` status 2 or 5 |
| `plugged` | `none` / `usb` / `ac` / `wireless` | `dumpsys battery` plugged |
| `battery_temp_c` | pack temperature, °C | `dumpsys battery` temperature ÷ 10 |
| `battery_voltage_v` | pack voltage, V | `dumpsys battery` voltage ÷ 1000 |
| `link_rtt_ms` | ADB round-trip time | timed `get-state` |
| `joined` | a fleet heartbeat succeeded this tick | `/fleet/heartbeat` reply |

## The store vector

The same snapshot is pushed to the Seed store as 8 floats (cog id **26**), each normalized to 0–1:

```
[ online, battery/100, charging, on_power, rtt/2000, temp/60, voltage/5, reserved ]
```

`reserved` is held for a later signal (e.g. Wi-Fi RSSI or head-motion once the IMU path lands).

## The heartbeat

When the glasses are online, the cog posts `{id, kind:"glasses", sensor:<model>, battery, fw, ip}` to the host's `/fleet/heartbeat`. That is what makes the node *join* and stay online. Nothing is posted while the glasses are offline, so the host's own TTL takes the node offline without any "last known" fiction.

## Beyond the MVP

This cog deliberately stops at liveness. The next layers — HUD frames, the glasses IMU, mic/audio level, the camera — attach to the node this cog establishes.
