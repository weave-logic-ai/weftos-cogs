# ADR-165: rd-03e, Ai-Thinker RD-03E 24 GHz presence and ranging radar over UART

**Status**: Proposed
**Date**: 2026-10-03
**Cog**: `rd-03e` 0.1.0

The number 165 is provisional (the next free number after ADR-161 in this worktree) and is
reassigned when the cog is exported upstream. This ADR records decisions already present in
the code (`src/cogs/rd-03e/`); it does not add behaviour.

## Context

The cog reads an Ai-Thinker RD-03E, a 24 GHz FMCW radar module built around the S3KM111L SoC
(`cog.toml` description, `guide/guide.toml` `sensor`). It reports whether a target is present
and one distance. It is a cheap presence layer next to the multi-target `ld2450-radar` cog
(ADR-161) and the vitals radar `ld6002-radar` cog (ADR-157). The guide's "What it's *not* for"
section says multi-target X/Y tracking is the RD-03**D**, not this part.

The target host is a Cognitum Seed (Raspberry Pi Zero 2 W). Two attachment routes are
documented in `guide/wiring.md`:

- the Pi header UART (`/dev/serial0`, pins 8 and 10), which is also the Linux serial console
  out of the box; or
- a 3.3 V USB-serial adapter (`/dev/ttyUSB0`), which needs no device change.

`cog-sensor-sources` has no UART source. As with the other radar cogs, this cog opens a
caller-supplied device path itself. Unlike `ld2450-radar` and `ld6002-radar` it does not use
the `serialport` crate; it carries its own small reader (`src/serial.rs`) over `libc`.

## Protocol sources

| Source | Used for |
|---|---|
| Ai-Thinker "Rd-03E Specification V1.0.0" §1 and §7.2, as cited in `guide/sensor.md` and `src/spatial.rs` | Ranging beam: azimuth ±20°, elevation ±45°, wall-mounted orientation |
| Ai-Thinker / openelab product pages, as cited in `guide/sensor.md` | Band, range, accuracy, UART settings (treated as datasheet claims) |
| Community captures, as stated in `cog.toml` and `src/main.rs` | The 5-byte report frame layout |

The vendor documents are not committed. The report frame layout is **not** taken from a vendor
document in this repo: `cog.toml` and the `src/main.rs` header both say "confirmed by
community captures". That is a weaker source than ADR-161's vendor protocol PDF, and is
recorded as an open question below.

Protocol facts as the code and guide state them:

| Fact | Value | Source |
|---|---|---|
| UART | 256000 baud, 8N1, 3.3 V TTL; the module streams on power-up, no command needed | `cog.toml`; `guide/sensor.md`; `guide/wiring.md` |
| Report frame | 5 bytes: `AA`, distance low, distance high, state, `55` | `src/main.rs` header, `parse_frames` |
| Distance | `buf[1] \| (buf[2] << 8)`, unsigned, centimetres | `src/main.rs` `parse_frames` |
| State | 0 no target, 1 present, 2 to 8 vendor motion/gesture codes (meanings not documented here) | `src/main.rs` `state_name`; `guide/api.md` |
| Checksum | none; header `AA` and footer `55` are the only integrity checks | `src/main.rs` header comment |
| Example captures used as tests | `AA 2D 00 00 55` is 45 cm, no target; `AA 36 00 01 55` is 54 cm, present | `src/main.rs` test `parses_confirmed_example_captures` |
| Range (vendor claim) | moving human to about 6 m; micro-motion to about 3.5 m | `guide/sensor.md` |
| Accuracy (vendor claim) | ±5 cm from 30 to 350 cm, ±5 % from 350 to 600 cm | `guide/sensor.md`; `src/spatial.rs` |
| Ranging beam (vendor claim) | azimuth ±20°, elevation ±45° | Specification V1.0.0 §1, §7.2, via `src/spatial.rs` |
| Power | 3.3 V only; never the 5 V pins | `guide/wiring.md`; `guide.toml` `avoid` |

Unknown, deliberately not interpreted:

- What states 2 to 8 mean, and whether the distance field is meaningful in those states. The
  cog reports `state` raw and names every value above 1 `gesture`.
- Whether the module can be configured over its RX line (the guide says the config wire is
  optional and the cog "only reads"); the command set, if any, is not documented in this repo.
- Which target the single distance refers to when several are in view. `src/spatial.rs`
  states "the module reports one target, the nearest", which the repo does not source.
- The frame rate. `guide/sensor.md` says "a few tens of Hz"; the simulator uses about 20 Hz.
  Neither is a measurement.
- Accuracy, hold time after the person stops, and beam shape in practice. The "Measured"
  section of `guide/sensor.md` is still marked pending.

## Decision

`rd-03e` opens the serial device, reassembles frames continuously on a background thread,
and prints one JSON line per `--interval` window (`--window` with `--once`).

**Framer** (`parse_frames`, `src/main.rs`). One accumulator buffer. It discards bytes before
the next `AA`; with fewer than 5 bytes it waits; if byte 4 is `55` it emits a frame and drains
5 bytes; otherwise it drops one byte and rescans (the resync rule used by `ld6002-radar`).
A partial frame at the end is kept for the next read. The buffer is trimmed to its last 4096
bytes if it ever grows past that. Because a lone `AA` or `55` can occur inside the distance
field, a wrong-aligned frame whose byte 4 happens to be `55` would be accepted; with no
checksum that cannot be detected. This is a known limit of the format, not of the parser.

**Decoding.** Distance is the unsigned little-endian 16-bit value in centimetres, passed
through unchanged (no clamp, no plausibility filter). `distance_cm` is 0 when the module
reports no target. `present` is `state != 0`. Presence onsets (a transition from state 0 to
non-zero) are counted in a 60 s window to give `detections_per_min`, with `total_detections`
since start and `quiet_s`, the time since the last onset.

**Output line** (`build_report`):

```json
{"status":"present","source":"RD-03E on /dev/ttyUSB0 @ 256000 8N1","present":true,
 "distance_cm":183,"state":1,"state_name":"present","detections_per_min":2,
 "total_detections":5,"quiet_s":12.4,"frames":40,"window_s":2.0,"samples":40,
 "export":"http://0.0.0.0:8050/raw","timestamp":1791071255}
```

- `status` is `no_frames` (port open, nothing decoded yet), `present`, or `clear`. When the
  device cannot be opened the line is `{"status":"no_source","error":...,"timestamp":...}`.
- `state_name` is `no_target`, `present` or `gesture`; the raw `state` is always kept.
- `quiet_s` is `null` until the first onset has been seen.
- This cog predates the `weavelogic.cog-output.v0` envelope proposed in ADR-157 and used by
  ADR-161. It emits the older flat object above and does not carry `schema`, `health`,
  `quality`, `reasons` or `sensor_class`. Adopting the envelope is a follow-up, not a
  decision made here.

**Sources.**

- *UART.* `Serial::open` (`src/serial.rs`) sets raw 8N1 at the requested baud. On Linux it
  uses `termios2` with `BOTHER` (`TCGETS2`/`TCSETS2`), which accepts 256000, a rate that has
  no standard `Bxxxx` constant. On macOS it uses raw `termios` then the `IOSSIOSPEED` ioctl.
  `VMIN` is 0 and `VTIME` is 1, so reads time out after 0.1 s and the loop keeps waiting.
- *Port.* `--port` names the device. If it is omitted the cog picks the first existing of
  `/dev/ttyUSB0`, `/dev/ttyACM0`, `/dev/tty.usbserial-0001`, `/dev/tty.usbserial`,
  `/dev/tty.SLAB_USBtoUART`, falling back to `/dev/ttyUSB0`.
- *Simulate.* `--simulate` (`RadarSim`) generates protocol bytes, not decoded values, at about
  20 Hz: a target sweeping 50 to 350 cm that drops to no target for part of the cycle. The
  bytes go through the same `parse_frames` as real UART data.
- *Replay.* This cog has no `--replay`. Captured-byte replay exists in `ld2450-radar` (ADR-161)
  and is not present here.

**`no_source` is fail-honest.** If the device cannot be opened, the cog prints one
`no_source` line. With `--once` it then exits 0. In continuous mode it retries every
`--interval` seconds (clamped to 1 to 5 s) and the export stays up, so a companion app can
watch `no_source` turn into a live report. No frame is invented.

**Store ingest.** After each report whose status is not `no_frames`, the cog POSTs one 8-value
vector to `127.0.0.1:80/api/v1/store/ingest` as `{"vectors":[[24, [...]]],"dedup":true}` with
a 5 s timeout. Id 24 is this cog's slot (`STORE_ID`; 21 ecg, 22 tof, 23 sound). The vector is
`[present, distance_cm/600, state/8, detections_per_min/60, quiet_s/60, 0, 0, 0]`, each clamped
to 0..1; `quiet_s` counts as 1.0 until the first onset. A failed ingest is logged to stderr
and does not stop the cog. Note that `ld6002-radar` and `ld2450-radar` deliberately do not
ingest; this cog does, and its reports and vector therefore also carry presence data to the
Seed's store.

**Export.** A read-only HTTP/1.1 server (`src/export.rs`) starts before the device opens, on
`--api-bind` (default `0.0.0.0:8050`, `cog.toml` `bind_loopback_only = false`):

| Route | Returns |
|---|---|
| `GET /status` | the latest report, or `{"status":"starting"}` |
| `GET /raw` | `{source, fs, samples:[{t_ms, v}]}`, the last 15 s of distance in cm (0 when no target) |
| `GET /guide` | the compiled-in guide bundle (`{toml, pages, images}`) |
| `GET /healthz` | `{"ok":true}` |
| `GET /spatial?seconds=N` | feature builds only: last N seconds (at most 30) of `radar_range` JSON lines |

Responses carry `Access-Control-Allow-Origin: *`. With `--once` and a source that cannot be
opened, the export is not started.

**Guide.** A WeftOS ADR-104 guide ships in `guide/` (`guide.toml` plus six pages: start,
sensor, wiring, setup, troubleshoot, api), compiled in by `src/guide.rs` together with
`pizero2w-pinout.jpg` and served at `/guide`. `guide.toml` declares `medical = false`.

**Configuration** (`cog.toml`, `parse_opts`). `--interval` 1 to 60 s (default 1); `--window` 1
to 30 s (default 2, used with `--once`); `--port`; `--baud` 9600 to 921600 (default 256000,
advanced); `--simulate`; `--once`; `--api-bind`. An out-of-range number prints a warning to
stderr and uses the default; this cog does not refuse bad arguments with exit 2 (the spatial
flags do, below). `--help` is handled by `cog-sensor-sources::handle_help` from `cog.toml`.

**Console.** `allowed_commands` is `--once`, `--once --simulate`, `--help`; `max_runtime_secs`
15; `output_limit_bytes` 65536. Nothing the console can run writes to the radar; the cog never
writes to it at all.

**Category: `presence`** (`cog.toml`). Hardware requirement: `pi-zero-2w`, `v0-appliance`.
Binary `cog-rd-03e-arm`.

### Spatial evidence (cargo feature `spatial-evidence`, off by default)

`src/spatial.rs` turns readings into WeftOS-spatial ADR-107 §7.1 `spatial.evidence.v1`
`radar_range` lines: something reflects at this range, bearing unknown, somewhere inside the
beam. This differs from ADR-161, where a two-axis radar yields `radar_track_point` lines with
a position. Here the module gives a range only.

Without the feature, `mod spatial` and the `--spatial-*` flags are not compiled and the
default build has the same dependencies (`Cargo.toml`: `spatial-evidence = []`, no new
crates). The wire shape is mirrored with local serde structs, with no WeftOS dependency.

- **Runtime switch.** Off even in a feature build unless `--spatial-out export|FILE` is given.
  `export` serves the last 30 s at `GET /spatial?seconds=N` on the existing export, in
  continuous mode only (`--once` with `export` exits 2). `FILE` appends JSONL to a plain path
  (not under `/dev/`, `/proc/`, `/sys/`; no `..`; no NUL). No new socket is opened. The other
  `--spatial-*` flags without `--spatial-out` are an error (exit 2).
- **Pose.** `--radar-pose x,y,z,yaw_deg[,pitch_deg]`: antenna position in `room_enu` metres
  (each within ±1000 m), boresight yaw in degrees counter-clockwise from room +x east
  (-360 to 360; 0 faces east, 90 faces north), and optional pitch in degrees above horizontal
  (-90 to 90, negative tilts down, 0 if omitted). Unlike ADR-161 there is no `x_sign` field:
  with no lateral measurement there is nothing for it to apply to, and no transform is done.
  The line carries the mount pose and the cog does not move the range into the room. The
  spatial engine places the evidence using `position`, `yaw_deg`, `pitch_deg` and `fov_deg`.
- **What becomes a line.** Only `state` 1 with a distance above 0. No-target frames send
  nothing (ADR-107 asks for this), and neither do the gesture states 2 to 8, whose distance
  meaning is not documented. Lines are rate-limited to `--spatial-max-hz` (default 2, above 0
  up to 20) using the frame's receive time.
- **Beam.** `fov_deg` is `[40, 90]` (azimuth ±20°, elevation ±45°, full width), from the
  vendor specification for the wall-mounted orientation. These are vendor figures, not
  measured here. `targets` is always 1.
- **Envelope.** `region` from `--spatial-region`; `source_id` from `--spatial-source-id`
  (default `rd-03e`, a device, never a person; both ids match `[A-Za-z0-9._:/-@]{1,128}`);
  `uncertainty_m` from `--spatial-uncertainty-m`, default 0.1 m (an assumption derived from
  the vendor's ±5 cm claim, not measured); `t_ns` is the receive time; `provenance.receipt`
  is `<source_id>:<6-digit sequence>`, `producer` is `rd-03e@<version>`, `proof` is
  `MEASURED` for UART frames and `SYNTHETIC` under `--simulate`. `position` is the pose
  rounded to 1 mm; `pitch_deg` is omitted when 0; `range_m` is `distance_cm / 100`.
- **Refusal.** With `--spatial-out` but no pose, or no region, the cog keeps running, writes
  no evidence, and sets `spatial` to `"no_pose"` or `"no_region"`, with a
  `spatial_evidence_refused: <why>` entry in `reasons`, on every report. While emitting,
  `spatial` is `"emitting"`; a failed file write gives `"write_error"` with a
  `spatial_evidence_error` reason. `no_source` lines and default builds have no `spatial`
  field.

## Non-claims

- The cog reports one distance and a raw state. No position, bearing, identity, count or
  velocity. Whether the distance is the nearest target is not established.
- `clear` means the module reported state 0, which may include a person sitting still; the
  guide says micro-motion is detected but that is a vendor claim, unmeasured.
- Gesture codes 2 to 8 are not interpreted.
- No frame is fabricated. Simulated output carries the source text `simulated RD-03E` and
  `SYNTHETIC` proof in spatial lines. (Unlike `ld2450-radar`, the stdout report has no
  `verified` or `simulated` flag; the `source` string is the only marker.)
- Vendor range, accuracy and beam figures are quoted, not verified.

## Verification

Nothing is MEASURED on hardware. `guide/sensor.md` "Measured" is still pending, and no real
RD-03E has been read by this cog on a Seed or laptop in the repo's record.

- Unit tests in `src/main.rs` (5): option bounds and defaults; the two example captures; resync
  after leading noise with a partial frame retained; a bad footer dropping one byte and
  resyncing; the simulator feeding the real parser with a bounded 8-value vector.
- With `--features spatial-evidence`: 9 unit tests in `src/spatial.rs` (a golden line matching
  the ADR-107 example key order; yaw 0 faces +x and yaw 90 faces +y; pitch sent only when set;
  only present frames with a distance, and the rate limit; refusal without pose or region;
  `SYNTHETIC` versus `MEASURED`; pose bounds; argument handling; the file sink) and 1 in
  `src/export.rs` (the 30 s evidence ring and JSONL route).
- There is no `tests/` directory: no CLI end-to-end tests exist for this cog, unlike
  `ld2450-radar`.
- `guide-check` (WeftOS `weftos-sensor-guide`) validates `guide/`.
- No captured radar bytes are committed.

## Consequences

- Freeing the Seed's header UART removes its serial console. That is a device change that
  needs the owner's approval and is not part of this cog. The guide gives the commands
  (`raspi-config nonint do_serial_cons 1`, `do_serial_hw 0`, reboot) in `wiring.md`. The USB
  serial route avoids it.
- Which Pi UART backs `/dev/serial0` on a Seed (mini UART or PL011) is not analysed in this
  repo for this cog. ADR-161 discusses lost bytes on the mini UART at the same 256000 baud;
  the same caution applies, and `frames` against the expected rate is the only check here.
- The export binds all interfaces by default and the store ingest is on by default. Presence
  and range are lower-risk than ADR-161's positions, but this is still occupancy data;
  `--api-bind 127.0.0.1:8050` limits the export to the host.
- A 5 V USB-serial adapter can corrupt the stream and damage the 3.3 V module
  (`guide/troubleshoot.md`); the guide requires a 3.3 V adapter.
- Whether the cog's OS user can open the serial device (`dialout` group) is an open question
  for Cognitum, as in ADR-157.
- The module's own configuration, if any, is untouched: the cog never writes to the UART.
- The hand-rolled `libc` serial reader duplicates code in other cogs; consolidating on one
  reader (or a `cog-sensor-sources` UART source) is future work.

## Open questions

1. The frame layout comes from community captures. Confirm it against the Ai-Thinker
   specification (or a bench capture) and record the section.
2. Meaning of states 2 to 8, and whether distance is valid in them.
3. Real report rate, and whether the "nearest target" claim holds with two people in view.
4. Measured accuracy against a tape measure, hold time after motion stops, and beam shape,
   against the vendor figures. Fill in the pending "Measured" section of `guide/sensor.md`.
5. `guide/sensor.md` referenced a `module.jpg` that was never committed, which `guide-check` rejects (gate step 11). The reference was removed on 2026-10-03; a module photo can be added to `guide/` and bundled in `src/guide.rs` later.
6. `guide/api.md` says the `radar_range` default uncertainty is 0.1 m and `/spatial` is the
   export route; confirm this matches the spatial engine's `RadarRangeModel` expectations.
7. Move the report onto the `weavelogic.cog-output.v0` envelope (ADR-157, ADR-161) and add a
   `--replay` and CLI tests, so this cog matches the radar cogs that came after it.
8. Whether a Seed-resident store ingest for presence is wanted for this cog when the sibling
   radar cogs do not ingest.

## Alternatives

- **The `serialport` crate, as in `ld2450-radar` and `ld6002-radar`:** not used here; the cog
  ships its own `termios2` reader with `libc` only. It works for this one-way stream and keeps
  the dependency list short, at the cost of duplicated code.
- **Querying or configuring the module over its RX line:** not done. The cog is read-only, so
  a mis-wired or absent config wire cannot break it.
- **Using the RD-03D or `ld2450-radar` for multi-target tracking:** a different part and cog;
  this one is for single-range presence.
- **Reporting the range as a position:** rejected for spatial output. One range with unknown
  bearing is `radar_range` evidence, not a track point, so the cog does not invent a lateral
  coordinate.
- **Routing through `cog-sensor-sources`:** it has no UART path; adding one belongs upstream.
