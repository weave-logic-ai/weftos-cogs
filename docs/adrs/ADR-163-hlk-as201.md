# ADR-163: hlk-as201, Hi-Link HLK-AS201 attitude/IMU reader cog

**Status**: Proposed
**Date**: 2026-10-03
**Cog**: `hlk-as201` 0.1.0

The number 163 is provisional, as for the neighbouring cog ADRs, and is reassigned when the cog
is exported upstream. This ADR records decisions already present in the cog; it does not add
behaviour.

## Context

The part is a Hi-Link HLK-AS201, a 9/10-axis attitude sensor: 3-axis accelerometer, 3-axis
gyroscope and 3-axis magnetometer, plus a barometer on 10-axis variants, with a 32-bit DSP
(`cog.toml` description; `guide/sensor.md`). The module's X/Y/Z silkscreen identifies it as an
IMU, not a radar (`cog.toml`, `guide/sensor.md`). The cog was first added together with
`rd-03e` (commit `49aa3bb`). Its first protocol guess was a WIT-style fallback, which was wrong
and was replaced by the real Hi-Link protocol in `6aa7d08`.

The target host is a Cognitum Seed (Raspberry Pi Zero 2 W class, `hardware_requirement =
["pi-zero-2w", "v0-appliance"]` in `cog.toml`). The sensor is a 3.3 V TTL UART part. It reaches
the Seed either on the header UART (pins 8/10, `/dev/serial0`) or through a 3.3 V USB-serial
adapter (`/dev/ttyUSB0`) (`guide/wiring.md`, `guide/setup.md`).

`cog-sensor-sources` is used for `--help` handling only (`src/main.rs`, `handle_help`). The cog
opens its own serial device through a local `Serial` type (`src/serial.rs`).

## Protocol sources

| Document | Version, date | Used for |
|---|---|---|
| Hi-Link HLK-AS201 datasheet, <https://hlktech.net/index.php?id=1380> | V1.1, 2025-08-01 | Frame layout, report command, payload layout, scaling, default baud, calibration commands |

The datasheet is cited by `src/main.rs`, `guide/sensor.md` and `guide/troubleshoot.md`. It is not
committed to this repository, so the table below is as the cog records it, not re-verified here.

| Fact | Value | Where recorded |
|---|---|---|
| UART | 115200 baud default, 8N1, 3.3 V TTL; adjustable 4800-921600 | `guide/sensor.md`, `guide/wiring.md` |
| Frame | `FA FB`, `len`, `cmd`, data, `SUM`, `FC FD` | `src/main.rs` header comment |
| Length | `len` = byte count from `cmd` through the check byte | `src/main.rs` |
| Check | `SUM` = (cmd + data bytes) & 0xFF | `src/main.rs` |
| Byte order | little-endian | `src/main.rs` |
| Report | `cmd 0x00`: 1 sensor-type byte, then a 42-byte ten-axis payload | `src/main.rs`, `decode_report` |
| Payload order | accel (3 x i16), gyro (3 x i16), euler (3 x i16), mag (3 x i16), quaternion (4 x i16), temp (i16), pressure (i32), height (i32) | `src/main.rs`, `decode_report` |
| Scaling | accel x16/32768 g; gyro x0.0625 deg/s; euler x180/32768 deg; mag x0.006103515625 uT; quaternion x1/32768; temp x0.01 C; pressure x0.0002384185791 Pa; height x0.0010728836 m | `src/main.rs` constants |
| Default report rate | 20 Hz | `guide/sensor.md` |
| Get-version example | `FA FB 03 10 01 11 FC FD` (cmd 0x10, data 0x01, SUM 0x11) | unit test `build_frame_matches_datasheet_example` |
| Calibration commands | `0x1C` accel zero-bias (flat, still); `0x1D` begin and `0x1E` save magnetometer calibration; `0x1A` data-reporting switch | `guide/sensor.md`, `guide/troubleshoot.md` |

Not established from the repository (see Open questions): the meaning of the sensor-type byte
beyond its use for `mag_accuracy`, the meaning of other commands, and any accuracy figure.

## Decision

1. **Reader.** `hlk-as201` opens a UART device, reassembles frames continuously and prints one
   JSON line per `--interval` window (default 1 s; `--once` uses `--window`, default 2 s). The
   serial module (`src/serial.rs`) configures raw 8N1 with `VMIN=0` reads. On Linux it uses
   `termios2` with `BOTHER`, so arbitrary rates need no `Bxxxx` constant. On macOS it sets
   raw `termios` and then the exact rate with `IOSSIOSPEED`. The device is opened read/write
   but the cog never writes to it.

2. **Framer and parser** (`parse_frames`, `src/main.rs`). One long-lived accumulator, capped at
   4096 bytes (the oldest bytes are dropped past that). The parser:
   - scans to the next `0xFA`, discarding anything before it;
   - requires `0xFB` next, otherwise drops one byte and rescans;
   - accepts `len` only in 2..=250, otherwise drops one byte and rescans;
   - waits for `len + 5` bytes in total, then requires the tail `FC FD` and a matching `SUM`;
   - on a bad tail or checksum drops one byte and rescans, so a single corrupted frame
     costs only itself;
   - returns only `cmd 0x00` reports, skipping other valid frames (such as replies to
     configuration commands) by draining them without decoding.
   A report whose data is shorter than 43 bytes is dropped. The frame carries a checksum, so
   unlike some neighbouring cogs the integrity check is not limited to header and tail.

3. **Decoding.** Each field is the little-endian integer times the datasheet constant above.
   Pressure is converted from Pa to hPa in the cog (`/ 100`). `mag_accuracy` is `(type_byte >> 2)
   & 0x3`. Nothing is smoothed, filtered or fused.

4. **Motion state.** `moving` is true when the angular-rate magnitude (`gyro_mag_dps`) is at or
   above `--motion` (default 25 deg/s, bounded 1 to 2000). A still-to-moving transition is a
   motion event. The state keeps 15 s of the magnitude trace (`RING_SECONDS`) and 60 s of events
   (`EVENT_SECONDS`) in memory (`src/export.rs`). The 25 deg/s default is a placeholder: no
   resting gyro bias has been measured (`guide/sensor.md`, "Measured").

5. **Output line** (stdout, one JSON object per window). The field list is in `guide/api.md`.
   In `build_report`: `status`, `source`, `moving`, `accel_g`, `gyro_dps`, `euler_deg`,
   `mag_ut`, `quaternion`, `temp_c`, `pressure_hpa`, `height_m`, `mag_accuracy`,
   `gyro_mag_dps`, `motion_events_per_min`, `total_motion_events`, `quiet_s`, `packets`,
   `frame_protocol`, `window_s`, `samples`, `export` and `timestamp`. The values are the
   latest decoded report, rounded for display, not a window average. `status` is `moving`,
   `still`, `no_frames` (the port opened, no valid report yet), or `no_source` (the device
   could not be opened). The cog does not use the `weavelogic.cog-output.v0` envelope of ADR-157
   and ADR-161; adopting it is an open question below.

6. **Sources.**
   - `--port` (default: the first existing of `/dev/ttyUSB0`, `/dev/ttyACM0`,
     `/dev/tty.usbserial-0001`, `/dev/tty.usbserial`, `/dev/tty.SLAB_USBtoUART`, else
     `/dev/ttyUSB0`) and `--baud` (default 115200, bounded 4800 to 921600). Out-of-range
     numeric values fall back to the default with a warning on stderr.
   - `--simulate` generates real protocol frames (`build_frame`, `ImuSim`) at about 10 Hz:
     gravity on Z, a slow tumble, an intermittent shake, a quaternion near identity, 25.0 C and
     roughly sea-level pressure. The bytes go through the same `parse_frames` and decoder as
     hardware bytes. The simulated report reuses the live status values, so the output does not
     itself mark simulated data (see Open questions).
   - **No replay.** There is no `--replay` option; unlike `ld2450-radar` this cog cannot decode
     a captured byte file.

7. **No source is reported honestly.** If the device cannot be opened, the cog prints
   `{"status":"no_source","error":...,"timestamp":...}` and stores it as the latest report.
   With `--once` it then exits. In continuous mode it retries every `interval` clamped to 1-5 s.
   If the device opens but no valid frame arrives, `status` is `no_frames`. Nothing is
   fabricated and no values are carried between runs. Read errors are retried after 20 ms and
   logged every 200th error.

8. **Store ingest.** Unlike `ld6002-radar` and `ld2450-radar`, this cog does POST to the
   Seed's `/api/v1/store/ingest` (`store_to_seed`, `127.0.0.1:80`, HTTP/1.0). The payload is
   `{"vectors": [[25, v]], "dedup": true}` where id 25 is this cog's slot ("21=ecg, 22=tof,
   23=sound, 24=radar, 25=hlk-as201 IMU", `src/main.rs`) and `v` is eight values clamped to
   0..1: roll, pitch and yaw normalised from +/-180 deg, ax, ay, az normalised from +/-16 g,
   gyro magnitude over 2000 deg/s, and the `moving` flag. Ingest is skipped while `status` is
   `no_frames`. A failure is logged to stderr and does not stop reporting. The 5 s
   read/write timeouts match the other cogs. The ADR-158 finding about the Seed's ingest
   keep-alive stall may apply here; it is unmeasured for this cog.

9. **HTTP export** (`src/export.rs`). A read-only HTTP/1.1 listener starts before the device
   opens, so a companion app can watch `no_source` turn live. Routes: `/status` (the latest
   report, or `{"status":"starting"}`), `/raw` (a decimated motion trace: `source`, `fs`, and
   `samples` of `{t_ms, v}`), `/guide` (the embedded guide bundle) and `/healthz`. Any other
   path returns `{"error":"not found"}` with status 200. Every response carries
   `Access-Control-Allow-Origin: *`. The default bind is `0.0.0.0:8051` (`--api-bind`,
   `cog.toml` `[api] bind_port = 8051`, `bind_loopback_only = false`). `--once` against a
   missing device binds no port. The `fs` field reports the configured baud rate, not a
   sample rate (see Open questions).

10. **Guide.** A WeftOS ADR-104 guide ships in `guide/`: pages `start`, `sensor`, `wiring`,
    `setup`, `troubleshoot` and `api` (`guide/guide.toml`), compiled in and served at `/guide`.
    `guide/sensor.md` has a "Measured" section that is explicitly pending.

11. **Configuration** (`cog.toml`, `parse_opts`). `--interval` 1-60 s; `--window` 1-30 s;
    `--port`; `--baud` 4800-921600 (advanced); `--motion` deg/s; `--api-bind host:port`;
    `--simulate` (advanced); `--once`; `--help`. The console may run only `--once`,
    `--once --simulate` and `--help`, for at most 15 s and 65536 bytes of output. The cog sends
    no configuration or calibration command to the sensor; calibration is described in the
    guide as something the owner does with the datasheet commands.

12. **Wiring and bus enablement.** Four wires (`guide/wiring.md`): sensor VCC to pin 1 (3V3),
    never the 5 V pins; GND to pin 6; sensor TX to pin 10 (GPIO15 RXD), which is the data
    direction; sensor RX to pin 8 (GPIO14 TXD), used only to reconfigure the sensor. A USB-serial
    adapter crosses the same way and needs no device change. Using `/dev/serial0` means freeing
    the primary UART from the serial login console (`raspi-config nonint do_serial_hw 0`,
    `do_serial_cons 1`, then reboot, per the guide). That is a change to the Seed's base OS. It
    needs the owner's approval and is not performed by the cog.

13. **Category: `presence`.** `cog.toml` currently says `category = "motion"`; the catalog is
    being mapped from `motion` to `presence`. The cog's own meaning is attitude and motion of
    the object the sensor is attached to, not where people are. The mapping is a catalog
    taxonomy decision, accepted here with that caveat recorded (see Open questions).

## Spatial evidence: not applicable

This cog has no `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107), although the sensor-cog
skill ships one by default. The reasoning is already recorded in `guide/sensor.md` and in commit
`6001ee4`.

The only IMU record in ADR-107, `imu_event` (`moved`), means "this sensing node's mount moved":
the engine then marks evidence from that node's `source_id` stale. The AS201 reports the
attitude of whatever it is attached to, usually a worn or handheld object. Its motion is not a
node moving, and if it is worn it is a person's movement, which ADR-107 keeps out of the map.
Its readings also carry no position. Emitting them as `imu_event` would mark the wrong evidence
stale.

What might fit later: an AS201 fixed to a sensing node's bracket (a radar or ToF mount),
configured with that node's id, could send `imu_event` `moved` when its attitude changes past a
threshold. Its tilt and magnetometer heading could also seed that node's `pose` record. The
heading is a compass bearing (clockwise from north), so it needs converting to ADR-107's yaw
(counter-clockwise from east) and correcting for declination. Neither exists yet.

## Non-claims

- No position, no velocity, no distance. The cog does not integrate acceleration.
- Heading is not trustworthy before the magnetometer calibration, and acceleration is not
  trustworthy before the accel zero-bias calibration (`guide/sensor.md`). The cog neither
  checks nor performs either.
- `moving` means the gyro magnitude reached a threshold, not that a person or object is
  moving in any other sense.
- No accuracy figure for any channel is claimed. Values are the datasheet scaling of raw
  integers.

## Verification

All of the following is CODE or SYNTHETIC. **Nothing is MEASURED on hardware** (`guide/sensor.md`
"Measured" is pending): no AS201 has been read by this cog on a Seed.

- Unit tests (5, `src/main.rs`): option bounds and defaults; the datasheet's get-version frame
  built byte-for-byte; a report frame with 1 g on Z and 25 C decoded; resync past garbage and
  a bad-checksum frame; the simulator feeding the real parser, gravity on Z, about
  1013 hPa, and a store vector of length 8 within 0..1.
- No CLI integration tests (`tests/`) exist for this cog.
- `guide-check` (WeftOS `weftos-sensor-guide`) validates `guide/`.
- Not tested: byte-at-a-time and split-point feeding, a bad length, a report shorter than 43
  bytes, the export routes, and the ingest client.

## Consequences

- The cog is dependency-light and has no libudev dependency; it uses `libc` ioctls directly
  (`src/serial.rs`).
- The export binds all interfaces by default and serves attitude data read-only with open CORS.
  Attitude is low sensitivity compared with positions of people, but `--api-bind
  127.0.0.1:8051` is available on untrusted networks.
- Store vector id 25 is claimed in a hard-coded comment list; a collision with another cog's
  id is only prevented by convention.
- Freeing the Seed's UART removes its serial console, a device change outside this cog.
- Whether the cog's OS user on a Seed can open `/dev/serial0` (`dialout`) is an open question
  for Cognitum, as in ADR-157.

## Alternatives

- **Route through `cog-sensor-sources`:** it has no UART source, so the cog opens the device
  itself, as `ld6002-radar` and `ld2450-radar` do.
- **Use the `serialport` crate:** the cog instead carries a small `termios2`/`IOSSIOSPEED`
  module. It works but duplicates what neighbouring cogs get from a crate (see Open questions).
- **A WIT-style frame decoder:** the first implementation guess. It was wrong and was
  replaced by the real Hi-Link framing (commit `6aa7d08`).
- **Emit `imu_event` for motion:** rejected above, because it would mark the wrong evidence
  stale.
- **Send calibration commands from the cog:** not done. The cog stays read-only toward the
  sensor, and calibration remains an owner action described in the guide.

## Open questions

- **Sensor-type byte.** The cog derives `mag_accuracy` from bits 2-3 of the byte before the
  42-byte payload. The meaning of the other bits and of other sensor types (for example a
  non-barometer variant) is not recorded in the repo. Whether a 9-axis part sends a shorter
  payload, which the cog would drop as shorter than 43 bytes, is not established.
- **Hardware verification.** Real report rate (the datasheet says 20 Hz), gyro bias, the
  `--motion` threshold, pressure and height sanity, and whether the Seed header UART at 115200
  keeps up, are all unmeasured.
- **Simulated marking.** Output does not carry a `simulated` or `verified` flag; `source` holds
  only a description string. Aligning with ADR-161's `source.verified` rule is open.
- **Output envelope.** Whether to move to the `weavelogic.cog-output.v0` envelope, with
  `health`, `quality` and `reasons`, is open.
- **`fs` in `/raw`.** It is set from the baud rate in `State::new`, so it is not a sample rate.
  It should be corrected or removed.
- **Guide image.** `guide/sensor.md` referenced a `module.jpg` that was never committed, which `guide-check` rejects (gate step 11). The reference was removed on 2026-10-03; a module photo can be added to `guide/` and bundled in `src/guide.rs` later.
- **Category.** `presence` fits the catalog mapping but not the sensor's meaning; a better
  taxonomy label for attitude/IMU data may exist.
- **Calibration persistence.** Whether the sensor keeps the `0x1C` and `0x1D`/`0x1E`
  calibrations across power cycles is not stated in the repo.
- **Console and group.** Whether the cog can open `/dev/serial0` as its service user.
- **Ingest.** Whether the 5 s keep-alive stall seen for other cogs applies to this cog's POST.
