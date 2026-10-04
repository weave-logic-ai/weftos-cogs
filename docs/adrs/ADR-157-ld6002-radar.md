# ADR-157: HLK-LD6002 60 GHz radar reader cog

**Status**: Proposed
**Date**: 2026-09-29 (target decoding added 2026-10-01, cog 0.2.0)
**Cog**: `ld6002-radar`

The number 157 is provisional (the next free number after upstream's ADR-156) and is
reassigned when the cog is exported upstream.

## Context

The HLK-LD6002 is a 60 GHz FMCW radar module that runs its own breathing, heart-rate,
distance and presence estimation on-chip and streams the results as small binary frames
over a 115200 8N1 UART. On a test board it appears as a CP2104 USB-serial device. It is a
useful ground-truth companion for WiFi-CSI work: its rows can be time-aligned with CSI
recordings taken in the same room.

`cog-sensor-sources` has no UART source. `health-monitor`'s optional `esp32-uart` feature
is the one existing cog that opens a serial device itself with the `serialport` crate
against a caller-supplied path. This cog follows that precedent.

## Decision

`ld6002-radar` opens the radar's serial device, reassembles frames continuously, and
prints one JSON line per `--interval` window (default 1 s).

**Frame format.** Ported from the bench-confirmed host framer:

```
0x01 | id:u16be | len:u16be | type:u16be | hcksum | payload[len] | dcksum
cksum = !(XOR of bytes); hcksum covers the first 7 bytes, dcksum the payload.
A zero-length frame carries no dcksum. len > 640 is treated as desync.
```

On any mismatch the framer drops one byte and rescans. The framer is long-lived across
windows, so a frame split across a window boundary is not lost.

**Decoded types.** Only what the reference decoder defines:

| Type | Payload | Output field |
|---|---|---|
| `0x0A13` | f32 LE ×3: total, breath phase, heart phase | counted as `phase_frames` |
| `0x0A14` | f32 LE breathing rate | `breathing_rate_bpm` (last in window; `<= 0` or non-finite is no reading) |
| `0x0A15` | f32 LE heart rate | `heart_rate_bpm` (same rule) |
| `0x0A16` | byte 0 validity flag, f32 LE at [4..8] | `distance_cm` (only when the flag is set) |
| `0x0A38` | u8 target count | `target_count` |
| `0x0F09` | u16 LE people-exist | `presence` (`byte0 == 1`) |
| `0x0100` | ASCII debug text | counted only |
| `0x0A04` | u32 LE count n, then n × 16-byte records of f32 LE `[x_m, y_m, f3, f4]` | `targets` (see below) |
| `0x0A17` | f32 LE ×2: `x_m`, `y_m` of the nearest target | `nearest` (last valid in window) |

Every other type, including `0x0A18`, is counted in `undecoded_frame_counts` and given
no meaning.

**Target decoding (`0x0A04`, `0x0A17`).** `0x0A04` is named "point cloud" in a sibling
device's firmware header, but no vendor source defines its payload. The layout above was
MEASURED on this radar (one LD6002 on a CP2104 board, 851 `0x0A04` frames, one person in
the room, 2026-10-01):

- x and y are metres in the radar's local frame: x left/right, y away from the radar.
  Evidence: `sqrt(x² + y²) × 100` matched the `0x0A16` distance in cm with a median
  difference of 7.6 cm (p90 13.9 cm) over 850 time-paired frames.
- `0x0A17` equalled the `0x0A04` record in every frame of that capture.
- `0x0A04` arrived at about 9.5 Hz.
- Every frame in the capture had n = 1. Layouts with n > 1 follow from the count prefix
  and the fixed record size but are not yet observed on hardware; they are covered by
  synthetic tests only.

Decoding rules:

- A `0x0A04` payload decodes only if its length is exactly `4 + 16n` and every x and y is
  finite. Anything else increments `target_parse_errors`, adds the reason
  `target_parse_errors`, and contributes no points (no partial frames). Health is not
  downgraded, matching the existing `short_payload` rule.
- Each point is `{t_ms, x_m, y_m, f3_unknown, f4_unknown}`. `t_ms` is the Unix-epoch
  wall-clock millisecond at which the serial read completing the frame was parsed, so
  points within one read share a stamp. Floats are emitted at their shortest f32
  representation (no rounding).
- `targets` keeps points in arrival order up to 64 per window (`MAX_TARGETS`); the rest
  are counted in `targets_dropped`. A 1 s window at the measured rate holds about 10.
  With 64 points a line stays far below the 64 KiB console output limit.
- `nearest` is the last `0x0A17` in the window whose payload is exactly 8 bytes with both
  values finite; otherwise `null`. A malformed `0x0A17` counts toward `short_payload`.
- `frame` is always `"radar_local"`. The output line never carries room coordinates. The
  optional spatial-evidence adapter below takes a mount pose and writes room coordinates
  to a separate file; the default build takes no pose input.

Open unknowns, deliberately not interpreted:

- `f3` was NaN and `f4` was 0.0 in every measured record. Their meaning is unknown. They
  are emitted raw as `f3_unknown` / `f4_unknown` (a non-finite value serializes as
  `null`) and must not be read as height, velocity, Doppler or SNR.
- `0x0A18` carried 2 bytes, observed as `01 00`. Its meaning is unknown and it is not
  claimed to be a target count; it stays in `undecoded_frame_counts`.
- A later live run of cog 0.2.0 on the same radar (30 one-second windows, one person,
  2026-10-01) contradicts "f3 is always NaN": f3 was 0.0 in most records and had the bit
  pattern `0x00000001` (prints as `1e-45`) in a few. That pattern suggests f3 may be an
  integer field rather than a float, but this is not established. It stays raw.
- In that run, 8 consecutive windows carried no `0x0A04` frames while `0x0A17` kept
  arriving at about 10 Hz, so `nearest` is not always backed by a `targets` entry.
  `targets` is then `[]`, which means "the radar sent no point frames", not "nobody is
  there".
- Behaviour with several people in the room (n > 1, record order, whether `0x0A17` then
  differs from the first record) has not been measured.

**Output line.** It uses the `weavelogic.cog-output.v0` envelope, which is a proposal
to Cognitum and not current catalog practice. The catalog convention is kept:
`timestamp` is Unix seconds. `timestamp_ms` (window end) and `window_start_ms` are
Unix-epoch UTC milliseconds that bound the window exactly, so rows can be aligned with
other recordings.

```json
{"schema":"weavelogic.cog-output.v0","cog":"ld6002-radar","version":"0.2.0",
 "timestamp":1790713871,"timestamp_ms":1790713871540,"window_start_ms":1790713866521,
 "sensor_class":"radar","source":{"kind":"ld6002-uart","device":"/dev/cu.usbserial-XXXX","verified":true},
 "health":"ok","quality":1.0,"reasons":[],
 "presence":true,"distance_cm":74.62,"heart_rate_bpm":92.0,"breathing_rate_bpm":1.0,
 "target_count":1,"frame":"radar_local",
 "targets":[{"t_ms":1790713866602,"x_m":-0.21,"y_m":0.72,"f3_unknown":null,"f4_unknown":0.0},"..."],
 "targets_dropped":0,"target_parse_errors":0,"nearest":{"x_m":-0.21,"y_m":0.72},
 "phase_frames":98,"frames":511,"checksum_errors":0,"oversize_frames":0,
 "resync_bytes":0,"bytes_read":8476,"frame_counts":{"0x0a13":98,"...":0},
 "undecoded_frame_counts":{"0x0a18":49}}
```

- `health` is `ok`, `degraded` (at least one checksum failure in the window) or
  `no_source` (device error, no bytes, or no valid frame).
- `quality` is link integrity only: bytes in valid frames / (those + resync bytes). It is
  `null` whenever `health` is `no_source`. It says nothing about the accuracy of the
  radar's vitals.
- `checksum_errors` counts header and payload checksum failures. A single corrupted frame
  can add more than one, because stray `0x01` bytes met while resyncing are also tested.

**Spatial-evidence adapter (cargo feature `spatial-evidence`, off by default).** The module
`src/spatial.rs` turns target positions into WeftOS-spatial ADR-107 §7 `spatial.evidence.v1`
`radar_track_point` lines. Without the feature none of its code or flags is compiled and the
dependency tree is unchanged. The wire shape is mirrored with local serde structs (as
`ruview-spatial-evidence` and `ld2450-radar` do), with no WeftOS dependency and no new
dependency of any kind.

- **Runtime switch.** Off unless `--spatial-out FILE` is given; lines are appended to a
  plain path (not under `/dev`, `/proc`, `/sys`, no `..`). The cog has no HTTP export, so
  `--spatial-out export` is refused. No socket is opened.
- **Pose.** `--radar-pose x,y,z,yaw_deg,pitch_deg,x_sign` (or the 5-field form without
  pitch, as pitch 0): mount position in `room_enu` metres, boresight yaw in degrees
  counter-clockwise from room +x (0 faces east, 90 faces north; ADR-107's one convention,
  no conversion layer), boresight pitch (positive up), and the sign of the radar's x axis.
  This ADR measured x as "left/right" without establishing which side is positive, so
  `x_sign` is configured: +1 when +x is to the radar's right looking out, found by stepping
  to the right of the boresight and watching the sign of `x_m`. The forward distance on the
  floor is `f = y·cos(pitch)`; `east = px + f·cos(yaw) + s·x·sin(yaw)`,
  `north = py + f·sin(yaw) − s·x·cos(yaw)`, rounded to 1 mm. z is 0.0.
- **Beam.** Every line carries ADR-107's `sensor` block: mount position, yaw, pitch, and
  half-angles of 60° in elevation and 60° in azimuth from Hi-Link's HLK-LD6002 product page
  ("Horizontal beam (-3dB): -60 to +60", "Vertical beam (-3dB): -60 to +60",
  <https://www.hlktech.net/index.php?id=1180>, read 2026-10-03). These are −3 dB beam
  widths, CLAIMED by the vendor and not measured here. The engine uses them to place the
  evidence in the beam's vertical band at the track's range and to drop floor clutter.
- **Which frames.** Each `0x0A04` record is one line; `track` is the record's 1-based index
  in its frame (ephemeral, never a person). A `0x0A17` frame becomes a line (track 1) only
  when no `0x0A04` frame has been parsed in the last 500 ms: the two carried the same
  position in every measured frame, and `0x0A17` is the only position during the measured
  gaps in `0x0A04`. Frames are decoded again for the adapter so the window's own decoding
  is unchanged; the 64-point window cap does not apply to the file.
- **The `0x0A04` caveat carries over.** The record layout is MEASURED on one radar with one
  person (n = 1 in every frame), not taken from a vendor document. Multi-target records,
  their order and the meaning of `f3`/`f4` are unknown (see "Open unknowns" above). The
  adapter therefore uses only `x_m`/`y_m`, emits no `velocity` and no height, and every
  line inherits that uncertainty. The range cross-check against `0x0A16` (7.6 cm median)
  says nothing about the angle, so the default `uncertainty_m` of 0.3 m is an assumption.
- **Refusal.** Without a pose (or a region) the cog keeps running, writes no evidence,
  sets `spatial: "no_pose"` (or `"no_region"`) and adds `spatial_evidence_refused: <why>`
  to `reasons` on every output line. While emitting, `spatial` is `"emitting"`; a failed
  write gives `"write_error"` and `spatial_evidence_error: <why>`. The `spatial` field
  exists only in feature builds.
- **Envelope.** `region` from `--spatial-region`; `source_id` from `--spatial-source-id`
  (default `ld6002-radar`, a device, never a person); `uncertainty_m` from
  `--spatial-uncertainty-m` (default 0.3); `t_ns` is the frame's parse time;
  `provenance.receipt` is `<source_id>:<sequence>`, `producer` `ld6002-radar@<version>`,
  `proof` `MEASURED`. The cog has no simulator, so every line it writes comes from the UART;
  the emitter's `SYNTHETIC` path is exercised by tests only.

**Category: `health`.** The distinctive outputs are the radar's heart and breathing rate.
The presence flag is secondary and is not trustworthy on its own (see below). This matches
`health-monitor`, the closest existing cog.

**Configuration.** `--interval` (1–3600 s), `--device` (must be under `/dev/`, no `..`),
and `--baud` (9600–3000000, default 115200). `--help` is served by
`cog_sensor_sources::handle_help`. Unknown arguments exit 2 without output.

## Non-claims

- The vitals are the module's on-chip estimates. They are not medical measurements and
  this cog does not validate them.
- No `status: "LIVE"`, no bounding box, no pose. Target x/y are the radar's own
  reported positions in its local frame, not room coordinates, and not validated beyond
  the distance cross-check above.
- `presence` is the radar's `0x0F09` flag as reported. A prior bench measurement of this
  module found the flag latching: it stayed true for minutes with no vitals frames. A
  consumer should corroborate it with `phase_frames > 0` rather than trust it alone.
- A missing value is `null`. Nothing is interpolated or carried over from an earlier
  window.

## Verification

- Unit tests build synthetic frames from the format above: checksums, big-endian header,
  zero-length frames, resync after garbage, corrupted header and payload, oversize length,
  every split point of a multi-frame buffer, byte-at-a-time feeding, unknown types, and
  the null-when-unreported rules. Target tests cover 0, 1 and 3 records, every wrong
  length (short, long, count mismatch, overflow-sized count), non-finite x/y, NaN `f3`
  serialized as `null`, every split point across two reads (including the parse-time
  stamp), the 64-point cap and `targets_dropped`, and `nearest` last-valid-wins.
  `tests/cli.rs` checks `--help`, argument refusal, the
  `no_source` line for a missing device, and that the default build refuses the spatial
  flags (exit 2).
- With `--features spatial-evidence`, 12 more unit tests, and 6 CLI tests instead of 5 (the
  default build's spatial refusal test is replaced by two): a golden `radar_track_point` line compared
  with ADR-107's example (same keys, `velocity` absent, `sensor` added); yaw 0 mapping
  radar-forward to +x and yaw 90 to +y; the transform (`x_sign` ±1, yaw 225, a right-hand
  point landing clockwise of the boresight, range preserved); pitch shortening the floor
  distance and the 60°/60° beam; out-of-range positions dropped; `0x0A17` used only in
  `0x0A04` gaps; refusal without a pose or region; `SYNTHETIC` versus `MEASURED`; pose and
  flag bounds; the file sink; encoded frames through the framer into the file; and, at the
  binary level, `spatial: "no_pose"` with no file written and `spatial: "emitting"` with no
  frames and no file.
- The lines from the framer test (13 lines, SYNTHETIC) parse and ingest with
  `weftos-spatial-core` at feat/spatial-workspace `fc4b0ad28`.
- No captured radar bytes are committed. Vital signs are person data.

## Consequences

- The cog opens a device path itself instead of going through `cog-sensor-sources`, as
  `health-monitor` does. Whether that is sanctioned for third-party cogs, and whether the
  cog's OS user on a Seed is in `dialout`, are open questions for Cognitum.
- `serialport` is built with `default-features = false`, so no libudev is needed and the
  cog never enumerates ports.
- A JSON schema under `schemas/weavelogic/` is not written yet. It waits on the shared
  envelope schema.
