# sen0628-tof

A Cognitum cog for the **DFRobot SEN0628** 8×8 matrix time-of-flight sensor (VL53L7CX + RP2040) on the Seed's I2C header, at 0x33 by default.

It reads depth frames (8×8 or 4×4, 20-3500 mm, 60° field of view) and reports:
- the nearest object and its zone;
- the share of zones that see a target;
- frame-to-frame motion;
- presence against a learned background;
- left, middle and right sector distances;
- per-zone noise.

It exports raw frames too. ADR-159.

The full documentation is the sensor guide in `guide/` (WeftOS ADR-104): start, parts, wiring, mounting, setup, calibrate, troubleshoot, API reference, safety and glossary. The cog serves it at `GET :8047/guide`, and the companion app `weft-tof-scope` (`crates/cog-tof-scope`) shows it in its Guide tab, next to a live zone heatmap and a hook-up checklist.

## Wiring (3.3 V only)

| SEN0628 | Seed header |
|---|---|
| `+` | pin 1 (3V3) |
| `-` | pin 6 (GND) |
| `C` (SCL) | pin 5 (GPIO3) |
| `D` (SDA) | pin 3 (GPIO2) |

- Set the board to I2C and the address you want (factory 0x33), then **power-cycle it**.
- Don't use pins 2 or 4 (5 V): the sensor's I2C pull-ups would then go to 5 V.
- It shares bus 1 with the ECG cog's ADS1115 (0x48).

## Install and run

```bash
scripts/cross-build.sh sen0628-tof
scripts/seed-sideload.sh sen0628-tof genesis@<seed>
curl -X POST http://<seed>/api/v1/apps/sen0628-tof/start          # continuous, export on :8047
curl -X POST http://<seed>/api/v1/apps/sen0628-tof/console -d '{"command":"--once --simulate"}'
```

## Outputs

- **A JSON report per interval** (stdout; the Seed keeps it in `/apps/sen0628-tof/logs`):
  - `status` (`ok`, `no_targets`, `no_frames`, `no_source` + `error`), `mode`, `frames`, `frame_rate_hz`, `valid_pct`;
  - `nearest {mm, x, y}`, `median_mm`, `motion_mm`;
  - `background_learned`, `presence`, `occupied_zones`;
  - `sectors {left, middle, right}`, `mean_zone_noise_mm`, `zone_noise_mm[]`, `background_mm[]`;
  - `frame {t_ms, side, mm[]}`.
- **A store vector** (id 22, each value clamped to 0-1): `[nearest/max, valid, presence, occupied share, motion/500, left/max, middle/max, right/max]`.
- **An HTTP export** on `0.0.0.0:8047`: `/status`, `/frame`, `/frames?seconds=N` (N up to 30), `/raw.csv?seconds=N` (`t_ms,side,z0..zN`), `/guide`.

## Tested

- 17 host unit tests: the protocol packets, frame decoding, scene analysis (nearest, validity, motion, sectors, background and presence, noise), export routes and limits, options, ingest framing, and a simulated end-to-end report.
- **On cog0 (2026-10-01):**
  - `--once --simulate` through the agent console: exit 0 in 3.1 s, 31 frames at 10 Hz.
  - Continuous simulate: presence detected, nearest object at 1193 mm.
  - With no sensor attached: `no_source` with the hint "nothing answered at 0x33". The export is reachable from the Mac.
- **Not yet tested:** the physical SEN0628.
