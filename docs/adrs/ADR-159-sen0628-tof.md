# ADR-159: sen0628-tof, an 8×8 matrix ToF on the Seed's I2C header

**Status:** Proposed
**Date:** 2026-10-01
**Cog:** `sen0628-tof` 0.1.0

## Context

We have a DFRobot SEN0628 Gravity 8×8 matrix time-of-flight sensor:
- a VL53L7CX ToF sensor plus an RP2040 MCU;
- 3.3-5 V, under 80 mA;
- I2C at 0x30-0x33 (factory 0x33), UART at 115200, or USB-C;
- 20-3500 mm range, 60°×60° field of view, 15-60 Hz ranging.

We want it on our Seed (cog0) the same way as the SEN0213 ECG (ADR-158): a cog that exports both raw frames and processed results, a sensor guide (WeftOS ADR-104), and a companion app (`weft-tof-scope`).

I2C is already enabled on cog0, and the sensor's addresses don't clash with the ECG's ADS1115 at 0x48.

## Decision

1. **Read the sensor over I2C bus 1** at 0x33 by default (0x30-0x33 configurable), powered from 3.3 V.
2. **Port DFRobot's C++ protocol** (DFRobot_MatrixLidar.cpp, MIT, 2025-04-03), using `libc` and `/dev/i2c-N` only:
   - request: `0x55, argsH, argsL, cmd, args`;
   - poll single bytes until the status is `0x53` or `0x63` (`0xFF` means not ready);
   - then `cmd, lenL, lenH, data`, read in chunks of at most 32 bytes.

   `CMD_SETMODE (1)` with `[0, 0, 0, 4|8]` sets the mode at start, followed by the vendor's 5 s settle. `CMD_ALLDATA (2)` returns little-endian u16 millimetres, row-major, X left→right and Y top→bottom.

   We don't follow DFRobot's Raspberry Pi Python driver: it writes the packet once per byte and decodes the length as `(hi << 2) | lo`.
3. **Processing:**
   - a zone is valid at 20 mm up to `max_range_mm`;
   - the nearest object and its zone, the valid share and the median;
   - motion: the mean absolute change between consecutive frames;
   - presence: a background learned as the per-zone median of the first `learn_seconds` of frames, a zone occupied when it's more than `presence_mm` closer, and presence when at least 2 zones are occupied;
   - left/middle/right sector minimums;
   - per-zone noise.
4. **Outputs** follow ADR-158:
   - a JSON report per interval, which includes the latest frame, the background and the per-zone noise;
   - an 8-float store vector, id 22;
   - a read-only HTTP export on `0.0.0.0:8047` (`/status`, `/frame`, `/frames`, `/raw.csv`, `/guide`). It starts before the sensor is found.

   The ingest read stops at Content-Length, as in ADR-158.
5. **Spatial-evidence adapter (cargo feature `spatial-evidence`, off by default).** `src/spatial.rs` turns depth frames into WeftOS-spatial ADR-107 §7.1 `spatial.evidence.v1` `tof_depth` lines. Without the feature none of its code, flags or routes is compiled and the dependencies are unchanged; the default build ignores the flags, as it ignores any unknown flag. The wire shape is mirrored with local serde structs (no WeftOS dependency, no new dependency).
   - **Runtime switch.** Off unless `--spatial-out export|FILE` is given. `export` serves the last 30 s at `GET /spatial?seconds=N` on the existing export (continuous mode only; it binds `0.0.0.0` unless `api_bind` says otherwise, like the frames). `FILE` appends to a plain path (not under `/dev`, `/proc`, `/sys`, no `..`). No socket is opened; the store ingest stays the only outbound call. At most `--spatial-max-hz` lines per second (default 1, up to 15): walls and furniture are static evidence.
   - **Pose.** `--tof-pose x,y,z,yaw_deg,pitch_deg[,roll_deg[,x_sign]]`: lens position in `room_enu` metres; boresight yaw in degrees counter-clockwise from room +x (0 faces east, 90 north; ADR-107's one convention, no conversion layer); pitch, positive up; roll about the boresight, positive clockwise looking out (default 0, omitted from the line when 0); `x_sign` (default 1).
   - **Zone order.** ADR-107 wants row 0 at the top and column 0 on the left, both looking out along the boresight. The cog's frames are row-major with X left→right and Y top→bottom looking out of the lens (decision 2, guide "Mounting"), the same order, so zones are copied across unchanged. Which board edge is "top" is not verified on hardware (guide "Mounting"). A board mounted upside down is `roll_deg` 180; an image that turns out mirrored is `x_sign` -1, which reverses each row. A unit test pins the order with the engine's ray rule: zone (0, 0) points up and left of the boresight, zone (7, 7) down and right, and roll 180 swaps them.
   - **Record.** `fov_deg` `[60, 60]` (Context); `grid` `[side, side]` (8 or 4); `range_mm` the raw zone values; `valid` the cog's rule, 20 mm up to `max_range_mm`, since the board firmware hides the VL53L7CX's per-zone status. `uncertainty_m` defaults to 0.05 m, an assumption. `source_id` defaults to `sen0628-tof` (a device, never a person); `provenance.receipt` is `<source_id>:<sequence>`, `producer` `sen0628-tof@<version>`, `proof` `MEASURED` for I2C frames and `SYNTHETIC` under `--simulate`.
   - **Refusal.** Without a pose or a region the cog keeps running, writes nothing, and reports `spatial: "no_pose"` (or `"no_region"`) with `reasons: ["spatial_evidence_refused: <why>"]`. While emitting, `spatial` is `"emitting"`; a failed write gives `"write_error"`. `no_source` lines carry no `spatial` field. A bad spatial flag exits 2.
   - **Tests** (13 more with the feature): the ADR-107 §7.1 example line reproduced exactly from a 4×4 frame (same keys and order); a full 8×8 frame under 16 KiB; yaw 0 → +x and yaw 90 → +y; the zone-order pin above; `x_sign` -1 reversing rows; validity; the rate limit and malformed frames; refusal without a pose or region; `SYNTHETIC` versus `MEASURED`; pose and flag bounds; the file sink; and the `/spatial` route. Twelve lines from `--once --simulate` parse with `weftos-spatial-core` (feat/spatial-workspace `fc4b0ad28`) and ingest with its `TofDepthModel`.
6. **A sensor guide** ships in `guide/` (`guide.toml` with header, wiring, grid and flow data, plus ten pages), compiled in and served at `/guide`.

## Consequences

- No ADC is needed. The sensor shares bus 1 with the ECG cog, and up to four SEN0628s can be added at different addresses, each needing its own cog instance and config.
- **Changing the I2C address or the I2C/UART setting needs a power-cycle of the board**, per DFRobot. The switch positions weren't in the documentation we could read; the guide says so and points to the silkscreen.
- The background is only as good as the scene at start: restart the cog with the view empty to re-learn it. A moving person during learning usually still works, because the per-zone median rejects them.
- The board firmware doesn't expose the VL53L7CX's per-zone target status, so 0 and out-of-range readings are the only validity signal.
- The export carries depth maps, which show where people are. Set `api_bind` to `127.0.0.1:8047` on untrusted networks.
- Not tested yet with the physical sensor. Simulate mode was verified on cog0: through the console in 3.1 s, and continuously at 10 Hz with presence detection.

## Alternatives

- **UART on GPIO14/15:** it would collide with the serial console and the LD-series radars. I2C shares a bus with the ECG ADC instead.
- **USB-C to the Seed:** the Seed's data port is in gadget mode, and DFRobot's USB output is 8×8 only.
- **DFRobot's obstacle-avoidance commands (6-8):** not used, because the cog computes sectors itself from full frames.
