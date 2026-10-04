# Troubleshooting

## `no_source` / "no adb device attached"

The cog could not reach a device. Check, in order:

- `adb devices` lists the glasses ending in `device`. If `unauthorized`, accept the prompt on the glasses; if `offline`, re-plug or `adb disconnect && adb connect <ip:port>`.
- You passed the right target: `--serial` for USB, `--addr ip:port` for Wi-Fi. With neither, the cog autodetects the *single* attached device — ambiguous if several are plugged in.
- `adb` is actually found. The cog prints the path problem; pass `--adb /path/to/adb` or set `$ADB`.

With no device, `--once` exits after printing `no_source`; a continuous run keeps retrying.

## Node shows offline on the host

- The cog only heartbeats while the glasses are reachable. If `online` is false in the snapshot, fix the ADB link first (above).
- `--host` must point at the weft-cog-host, not the Seed store. Default is `WEFTOS_HOST`, then `http://127.0.0.1:9480`. A `heartbeat: ...` line on stderr shows the POST failing.
- The host ages a node out after its TTL. A node that keeps flapping online/offline usually means an unstable Wi-Fi link — watch `link_rtt_ms`.

## Battery fields are missing but `online` is true

`dumpsys battery` returned nothing parseable (rare). The node still joins on presence; battery simply reads null that tick.

## It "works" in `--simulate` but not for real

`--simulate` never touches ADB — it only proves the report + store + heartbeat path. If sim works and real does not, the problem is the ADB link or the device, not the cog.
