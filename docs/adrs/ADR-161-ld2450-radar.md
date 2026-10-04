# ADR-161: HLK-LD2450 24 GHz multi-target tracking radar reader cog

**Status**: Proposed
**Date**: 2026-10-03
**Cog**: `ld2450-radar` 0.1.0

The number 161 is provisional (the next free number after ADR-160 across our worktrees) and
is reassigned when the cog is exported upstream.

## Context

The part is a Hi-Link HLK-LD2450 from a confirmed order. It is a 24 GHz FMCW radar module with
patch antennas and an onboard MCU (BOOT0 pads on the board). Its 4-pin header is labelled `5V`,
`G`, `UR` (the radar's UART RX) and `UT` (its UART TX). Hi-Link's manual describes the antenna
as one transmitter and two receivers; the order notes say one TX and one RX. Two receive
channels are what an angle measurement needs, so the manual's description is the likelier
one, but this has not been checked against the board.

The radar tracks up to three moving targets and streams their positions and speeds at 10 Hz.
That makes it a cheap 2-D position layer next to WiFi-CSI work and the `ld6002-radar` cog
(ADR-157), which reports one target's vitals.

The target host is a Cognitum Seed (Raspberry Pi Zero 2 W, armv7). Its header UART
(`/dev/serial0` → `ttyS0`, GPIO14/15, pins 8/10) is currently the Linux serial console
(`console=ttyS0,115200` in `cmdline.txt` plus `serial-getty@ttyS0`). A 3.3 V USB-serial
adapter (`/dev/ttyUSB0`) is the alternative that needs no device change.

`cog-sensor-sources` has no UART source. Like `ld6002-radar`, this cog opens a caller-supplied
device path with the `serialport` crate.

## Protocol sources

| Document | Version, date | Used for |
|---|---|---|
| Hi-Link "HLK-LD2450 Motion Target Detection and Tracking Module, Serial Communication Protocol" | V1.03, 2023-10-17 | UART settings, report frame, command/ACK framing, commands, the worked example |
| Hi-Link "HLK-LD2450 Motion target detection and tracking module, Instruction manual" | V1.00, 2023-05-10 | Power, logic level, range, angles, refresh rate, mounting |
| Hi-Link "HLK-LD2450 User Guide" (operation manual, PDF dated 2024-12-20) | undated | Three more captured report frames with Hi-Link's decoding; enable-config stops reporting; distance resolution is fixed |
| Raspberry Pi documentation, "Configure UARTs" (`computers/configuration/interfaces.adoc`) and the firmware overlay README (`disable-bt`, `miniuart-bt`) | fetched 2026-10-03 | Mini UART limits, core clock, overlays, disabling the console |
| Broadcom "BCM2835 ARM Peripherals" §2.2 and §13 | 2012 | Mini UART baud formula and 8-byte FIFO; PL011 16×12 receive FIFO |

The vendor PDFs are not committed.

Protocol facts, each from protocol V1.03 unless stated:

| Fact | Value | Source |
|---|---|---|
| UART | 256000 baud, 8 data bits, no parity, 1 stop bit, TTL | §1.2.1, §2 |
| Logic level | 3.3 V IO | §1.2.1; manual §5.1 |
| Supply | 5 V, supply able to give > 200 mA; 120 mA average | §1.2.1; manual Table 4 |
| Byte order | little-endian | §2.1.1 |
| Report frame | `AA FF 03 00`, 3 × 8-byte target, `55 CC` (30 bytes) | §2.3 Table 9 |
| Target | x `int16`, y `int16`, speed `int16`, distance resolution `uint16` | §2.3 Table 10 |
| Sign convention | high bit 1 = positive, 0 = negative, low 15 bits = magnitude (sign-magnitude, not two's complement) | §2.3 Table 10 and worked example |
| Units | x, y in mm; speed in cm/s; resolution in mm ("size of a single distance gate") | §2.3 Table 10 |
| Empty target | its 8 bytes are `0x00` | §2.3 example |
| Rate | 10 frames per second | §2.3; manual Table 4 |
| Worked example | `AA FF 03 00 0E 03 B1 86 10 00 40 01 00…00 55 CC` → x −782 mm, y 1713 mm, speed −16 cm/s, resolution 320 mm | §2.3 |
| Command frame | `FD FC FB FA`, length `u16`, command word `u16` + value, `04 03 02 01` | §2.1.2 Tables 2-3 |
| ACK frame | same header and tail; data = command word with `0x0100` set, status `u16` (0 success, 1 failure), return value | §2.1.2 Tables 4-5 |
| Enable config | `0x00FF`, value `0x0001`; ACK returns status, protocol version `0x0001`, buffer size `0x0040`. Other commands are invalid without it | §2.2.1 |
| End config | `0x00FE` | §2.2.2 |
| Single / multi target | `0x0080` / `0x0090`; factory default multi | §2.2.3-4, Table 7 |
| Query tracking mode | `0x0091` → `0x0001` single, `0x0002` multi | §2.2.5 |
| Read firmware | `0x00A0` → type `u16`, major `u16`, minor `u32`; example `00 00 02 01 16 24 06 22` = "V1.02.22062416" | §2.2.6 |
| Baud rates | index 1-8 = 9600 … 460800; 7 = 256000 (factory) | §2.2.7 Table 6 |
| Reporting pauses in config mode | "data reporting stops after the enable configuration command is replied" | User Guide §2.3 |
| Coverage | 6 m, azimuth ±60°, pitch ±35°; wall mount at 1.5-2 m | manual §2.1, §7, Table 4 |
| Bluetooth | on by factory default; command `0x00A4` | §2.2.10, Table 7 |

Unknown, deliberately not interpreted:

- Which side of the radar positive x is on, and whether positive speed means approaching or
  receding. The documents give the sign encoding but not the physical direction.
- Whether a stationary person stays tracked. The documents describe motion tracking.
- Whether tracking mode is stored across power cycles. Table 7 lists it among factory
  defaults, which suggests it is, but the command sections do not say.
- Accuracy of x, y and speed. No vendor figure, nothing measured.

## Decision

`ld2450-radar` opens the radar's serial device, reassembles frames continuously and prints
one JSON line per `--interval` window (default 1 s).

**Framer.** One long-lived buffer handles both frame kinds. A report needs the exact header,
30 bytes and the exact tail. An ACK needs the header, a length of 4-64 and the exact tail.
On any mismatch the framer drops one byte and rescans (the `ld6002-radar` rule). A strict
prefix of a header at the end of the buffer waits for more bytes; anything else is skipped,
so noise never grows the buffer. Neither frame kind has a checksum, so header, tail and
length are the only integrity checks.

**Decoding.** x, y and speed use the sign-magnitude rule above (`0x0000` and `0x8000` are both
0). A slot whose x and y both decode to 0 is empty; the protocol sends absent targets as
zeros, and (0, 0) is the radar itself. A non-empty slot with y ≤ 0 (the user guide says y is
always positive) or |x| or y beyond 10 m (the radar's range is 6 m) is counted in
`implausible_targets` and dropped, because a corrupted byte cannot otherwise be detected.

**Output line.** The `weavelogic.cog-output.v0` envelope from ADR-157, a proposal to Cognitum
and not current catalog practice:

```json
{"schema":"weavelogic.cog-output.v0","cog":"ld2450-radar","version":"0.1.0",
 "timestamp":1791071255,"timestamp_ms":1791071255138,"window_start_ms":1791071254136,
 "sensor_class":"radar","source":{"kind":"ld2450-uart","device":"/dev/serial0","verified":true,"simulated":false},
 "health":"ok","quality":1.0,"reasons":[],"hint":null,"frame":"radar_local",
 "target_count":1,"max_target_count":1,"latest_frame_ms":1791071255037,
 "targets":[{"slot":1,"x_m":-0.782,"y_m":1.713,"speed_mps":-0.16,"resolution_mm":320}],
 "firmware":null,"tracking_mode":null,"frames":10,"frame_rate_hz":10.0,
 "parse_errors":0,"bad_tail":0,"bad_ack_len":0,"implausible_targets":0,
 "resync_bytes":0,"bytes_read":300,"acks":0}
```

- `targets` are the latest frame's targets, in slot order. A slot is not a person identity.
  `x_m`, `y_m` and `speed_mps` are exact unit conversions of the radar's integers.
  `resolution_mm` is passed through.
- `target_count` counts the latest frame's targets and is `null` when no frame arrived.
  `targets: []` with `target_count: 0` means the radar reported nobody.
  `max_target_count` is the most in any frame of the window.
- `health` is `ok`, `degraded` (any `bad_tail`, `bad_ack_len` or implausible target) or
  `no_source` (device error, no bytes, or no valid report frame). `quality` is link integrity
  (bytes in valid frames ÷ bytes consumed), `null` when `no_source`. It says nothing about
  position accuracy.
- `hint` names the next physical check when something is wrong (device path and console,
  power and the data wire, baud and config mode, lost bytes on the mini UART).
- `frame_rate_hz` is report frames per second over the window: `null` for a replay or a device
  that never opened.
- `source.verified` is true only for real UART frames. `--simulate` sets `simulated: true`,
  `verified: false` and the reason `simulated`.
- `frame` is always `radar_local`: metres, y along the boresight, x lateral. The cog takes no
  pose. Mapping to the spatial engine's `room_enu` (WeftOS-spatial ADR-107: east/north from the
  room's south-west corner; yaw in degrees counter-clockwise from +x/east, the project's one
  convention) belongs to the optional adapter below. The guide gives the transform
  `east = px + y·cos(yaw) + s·x·sin(yaw)`, `north = py + y·sin(yaw) − s·x·cos(yaw)`,
  with `s` = ±1 from the first-run x-direction check.

**Commands are opt-in.** By default the cog never writes to the radar.
`--query-firmware` sends enable-config, query-tracking-mode, read-firmware and end-config,
which change nothing. `--tracking-mode single|multi` also sends the mode command, which does
change the radar's configuration; the default `keep` sends nothing. Each command waits up to
1 s for its ACK. End-config is always attempted, so the radar resumes reporting. Errors land
in `reasons` as `config_error: …`. The console allows `--once --query-firmware` but never a
mode change.

**Sources.** `--simulate` produces protocol bytes (a person walking a slow figure 1.5-4.5 m out,
and a second standing target for half of each 30 s cycle) at 10 Hz and answers the commands
above, including the pause in config mode. Its bytes go through the same framer and decoder.
`--replay FILE` decodes a captured raw byte file (a regular file, not under `/dev`, `/proc` or
`/sys`, no `..`, at most 16 MiB) as fast as it parses, about `interval × 10` frames per line.
It is exclusive with `--simulate` and with commands.

**Export.** In continuous mode a read-only HTTP/1.0 export (sen0628-tof's pattern, ADR-159)
starts before the device opens: `/status`, `/targets`, `/frames?seconds=N` (N ≤ 30), `/guide`.
It binds `127.0.0.1:8052` by default, because target positions show where people are.
`0.0.0.0:8052` is an explicit choice. `--once` binds no port.

**No store ingest.** Like `ld6002-radar`, the cog does not POST to `/api/v1/store/ingest`.

**Spatial-evidence adapter (cargo feature `spatial-evidence`, off by default).** The module
`src/spatial.rs` turns targets into WeftOS-spatial ADR-107 §7 `spatial.evidence.v1`
`radar_track_point` lines. Without the feature, none of its code, flags or routes is compiled;
the default build has the same dependencies and its code is unchanged. The wire shape is
mirrored with local serde structs (as `ruview-spatial-evidence` does), with no WeftOS
dependency and no new dependency of any kind.

- **Runtime switch.** Off unless `--spatial-out export|FILE` is given. `export` serves
  the last 30 s as JSON lines at `GET /spatial?seconds=N` on the existing loopback export
  (continuous mode only). `FILE` appends JSONL to a plain path (not under `/dev`, `/proc`,
  `/sys`, no `..`). No new socket is opened.
- **Pose.** `--radar-pose x,y,z,yaw_deg,pitch_deg,x_sign` (the older 5-field form without
  pitch is still accepted, as pitch 0): mount position in `room_enu` metres, the boresight
  yaw in degrees counter-clockwise from room +x (0 faces east, 90 faces north; ENU math yaw,
  ADR-107 §7), the boresight pitch (positive up, measure it with an inclinometer app), and the
  sign of the radar's x axis (+1 when +x is to the radar's right, from the guide's step-right
  check). The radar reports targets in its tilted sensing plane, so the forward distance on the
  floor is `f = y·cos(pitch)`; `radar_to_room` computes `east = px + f·cos(yaw) + s·x·sin(yaw)`,
  `north = py + f·sin(yaw) − s·x·cos(yaw)`, rounded to 1 mm.
- **Beam.** Every line carries ADR-107's `sensor` block: the mount position, yaw, pitch and
  the manual's beam half-angles (35° elevation, 60° azimuth). The engine uses it to place the
  evidence in the beam's vertical band at the track's range and to drop tracks past the
  beam's floor intersection (floor clutter) or outside the room. There is no conversion layer
  for other yaw conventions. Target z is 0.0: the radar is
  2-D and ADR-107's radar model treats a track as a floor-to-head column. The pose's z is
  validated and documented as the mount height but does not enter the position.
- **Refusal.** Without a pose (or a region) the cog keeps running, writes no evidence, and
  sets `spatial: "no_pose"` (or `"no_region"`) and adds `spatial_evidence_refused: <why>` to
  `reasons` on every output line. While emitting, `spatial` is `"emitting"`; a failed file
  write gives `"write_error"`. The `spatial` field exists only in feature builds.
- **Envelope.** `region` from `--spatial-region`; `source_id` from `--spatial-source-id`
  (default `ld2450-radar`, a device, never a person); `uncertainty_m` from
  `--spatial-uncertainty-m`, default 0.3 m, an assumption until measured; `t_ns` is the
  frame's parse time; `provenance.receipt` is `<source_id>:<sequence>`, `producer`
  `ld2450-radar@<version>`, `proof` `MEASURED` for real UART frames and `SYNTHETIC` under
  `--simulate`. `track` is the radar's slot (1-3), ephemeral, as ADR-107 §9 requires.
- **No velocity.** The radar's speed is radial and its sign convention is unverified, so the
  optional `velocity` is omitted rather than guessed.
- Not available with `--replay`. A position beyond ±1000 m (a wrong pose) is skipped.

**Guide.** A WeftOS ADR-104 guide ships in `guide/` (start, parts, wiring, setup, placement,
troubleshoot, api, glossary), compiled in and served at `/guide`.

**Configuration.** `--interval` 1-3600 s; `--device` under `/dev/` with no `..`, default
`/dev/serial0`; `--baud` only the eight documented rates, default 256000; `--query-firmware`;
`--tracking-mode keep|single|multi`; `--api-bind host:port`; `--simulate`; `--replay`. Unknown
arguments exit 2 with no output.

**Category: `presence`.** The output is where people are, not vitals.

**Seed UART.** At 256000 baud the mini UART's divider is fine at the 250 MHz core clock that
`enable_uart=1` fixes (250 MHz / (8 × 122) = 256 148 baud, +0.06 %). The risk is lost bytes:
the mini UART buffers 8 received bytes (about 0.3 ms at this rate) and has no framing-error
detection. The recommendation is the PL011 on pins 8/10: `dtoverlay=disable-bt` if the Seed
does not use the Pi's Bluetooth, otherwise `dtoverlay=miniuart-bt` with `core_freq=250`.
Either keeps `/dev/serial0` as the device. Whether the mini UART is good enough at 10 frames
a second is unmeasured; `bad_tail`, `resync_bytes` and `frame_rate_hz` measure it.

## Non-claims

- No identity, no pose, no bounding box, no height. Targets are the radar's own reported
  positions in its local frame. Their accuracy is not measured.
- The direction of +x and the meaning of the speed sign are not established.
- `targets: []` means the radar reported no target, which may include a person standing still.
- No frame is ever fabricated. A missing value is `null`; nothing is carried over between
  windows.
- Simulated output is never `verified`.

## Verification

All of the following is CODE or SYNTHETIC. **Nothing is MEASURED on hardware yet**: no
LD2450 has been read by this cog on a Seed or a laptop.

- Unit tests (33): the protocol's worked example frame; the user guide's three captured frames
  with Hi-Link's decoded values; sign-magnitude for both zeros, the limits and round trips;
  an all-zero frame; three targets in slot order; resync after garbage with partial headers; a
  bad tail; a truncated frame followed by full ones; every split point of a report, ACK,
  report buffer; byte-at-a-time equals bulk; no buffer growth on noise; the documented command
  bytes for enable/end config, single, multi, query mode and read firmware; the documented
  ACKs, including the firmware string "V1.02.22062416"; bad ACK lengths; window rules
  (latest frame wins, units, empty frames, implausible targets, quality, `no_source` nulls,
  simulated never verified); the simulator staying in the field of view, answering commands
  and pausing reports in config mode; configuration against the simulator; the export routes;
  argument bounds; every console command parsing as `--once` with no config write; and the
  guide's page list and version.
- CLI tests (9 in the default build, `tests/cli.rs`): `--help`, refused arguments, `--device`
  outside `/dev`, the one-line `no_source` for a missing device, `--once --simulate` end to
  end, the simulated firmware query, `--replay` of the protocol example behind line noise, a
  missing replay file, and the default build refusing the spatial flags.
- With `--features spatial-evidence`, 12 more unit tests and 2 more CLI tests: a golden
  `radar_track_point` line compared with ADR-107's example (same keys and order, velocity
  absent); yaw 0 mapping radar-forward to +x and yaw 90 to +y; the pose transform
  (`x_sign` ±1, yaw 225, a right-hand point landing clockwise of the boresight, range
  preserved); NaN and out-of-range positions dropped; refusal without a pose or region, `SYNTHETIC` versus `MEASURED`, receipts,
  flag parsing and bounds, the file sink, the `/spatial` route, `--once --simulate` writing
  SYNTHETIC JSONL with `spatial: "emitting"`, and `spatial: "no_pose"` with no file written. The default build
  has a CLI test that the spatial flags are refused (exit 2).
- `guide-check` (WeftOS `weftos-sensor-guide`) validates `guide/`.
- No captured radar bytes are committed. Positions of people are person data.

## Consequences

- `serialport` 4.10 sets arbitrary rates on Linux through `termios2`/`BOTHER`, so 256000 needs
  no extra code. It is built without default features, so no libudev.
- Freeing the Seed's UART removes its serial console. That is a device change that needs the
  owner's approval and is not part of this cog. The guide gives the steps.
- The radar's own Bluetooth is on from the factory and Hi-Link's app can reconfigure it. The
  cog never sends the Bluetooth command; turning it off is an owner decision.
- Whether the cog's OS user on a Seed can open `/dev/serial0` (`dialout`) is still an open
  question for Cognitum, as in ADR-157.
- A JSON schema under `schemas/weavelogic/` waits on the shared envelope schema.
