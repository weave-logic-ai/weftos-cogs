# API & CLI

## Command line

```
cog-mentra-live [--once] [--interval N] [--simulate]
                [--serial <adb-serial> | --addr <ip:port>]
                [--node-id <id>] [--host <url>] [--adb <path>]
```

| Flag | Default | Meaning |
|---|---|---|
| `--once` | off | one snapshot, then exit |
| `--interval N` | 5 | seconds between polls/heartbeats (continuous) |
| `--serial S` | autodetect | ADB serial for a USB device |
| `--addr ip:port` | — | ADB-over-TCP target (`adb connect` first); wins over `--serial` |
| `--node-id ID` | `mentra-01` | fleet id the glasses join as |
| `--host URL` | `$WEFTOS_HOST` → `http://127.0.0.1:9480` | weft-cog-host for the heartbeat |
| `--adb PATH` | resolved | explicit adb binary |
| `--simulate` | off | synthetic telemetry, no hardware |

`--help` prints this and the embedded `cog.toml`.

## HTTP export (default `:8061`)

Plain HTTP, `Access-Control-Allow-Origin: *`, so a browser/egui companion can poll it.

| Route | Returns |
|---|---|
| `GET /status` | the latest telemetry snapshot (the same JSON printed to stdout) |
| `GET /raw` | recent battery trace: `{source, serial, model, battery_pct:[{t_ms,v}]}` |
| `GET /guide` | this guide bundle (`{toml, pages, images}`) |
| `GET /healthz` | `{"ok":true}` |

## Outputs

- **stdout**: one JSON telemetry object per interval.
- **store vector**: cog id `26`, 8 floats (see **What it reports**), POSTed to `127.0.0.1:80/api/v1/store/ingest`.
- **fleet heartbeat**: `POST <host>/fleet/heartbeat` with `{id, kind:"glasses", sensor, battery, fw, ip}` while online.
