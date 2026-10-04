# Connecting it to the Seed

The cog runs on the Seed, reads the OUT line, prints one JSON line per window to stdout, and POSTs
an 8-float vector to the Seed store at `127.0.0.1:80/api/v1/store/ingest`.

## GPIO access

Reading a GPIO line needs permission on `/dev/gpiochip0`. On the Seed the cog's user should be in
the `gpio` group (Raspberry Pi OS creates it). Check:

```
ls -l /dev/gpiochip0
groups
```

## Run it

```
# One window, then exit:
cog-ld1040c-motion --once --gpio 17

# Continuous, one line per second, with the HTTP export on the Seed:
cog-ld1040c-motion --interval 1 --gpio 17 --api-bind 127.0.0.1:8053
```

## HTTP export

In continuous mode the cog serves a read-only export (default `127.0.0.1:8053`):

- `GET /status` — the latest report
- `GET /raw` — the recent OUT-line trace (0/1 per sample)
- `GET /guide` — this guide
- `GET /healthz` — `{"ok":true}`

Bind to `0.0.0.0:8053` only if you intend to expose motion state to the LAN.
