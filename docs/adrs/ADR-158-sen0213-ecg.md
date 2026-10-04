# ADR-158: sen0213-ecg, single-lead ECG over an ADS1115 on the Seed's I2C header

**Status:** Proposed
**Date:** 2026-10-01
**Cog:** `sen0213-ecg` 0.1.2

## Context

We bought a DFRobot SEN0213, a Gravity heart-rate sensor with an AD8232 single-lead ECG front end. We want it on our Seed (cog0) as a cog that exports both the raw signal and the processed one.

Three constraints shape the design:
- The SEN0213's output is analog (0-3.3 V, PH2.0-3P). The Seed is a Raspberry Pi Zero 2 W, confirmed on-device on 2026-09-30, and it has no ADC.
- The Seed image ships with I2C and SPI off. Its data micro-USB port is in gadget mode, so a USB ADC or a USB microcontroller can't be attached.
- `cog-sensor-sources` has no I2C or GPIO source. That was already an open question for Cognitum.

## Decision

1. **Digitize with an ADS1115 on I2C bus 1** (pins 1, 3, 5 and 6), at address 0x48, powered from 3.3 V.
   - The ADC runs continuously at 860 SPS on ±4.096 V.
   - The cog reads it at 250 Hz (100-500 Hz configurable) against absolute deadlines and reports the jitter.
   - The cog talks to `/dev/i2c-N` directly (`ioctl I2C_SLAVE` plus 2-byte reads, `libc` only), not through `cog-sensor-sources`.
2. **Enable I2C in the base OS** (`dtparam=i2c_arm=on`, `i2c-dev`). This follows the COG-002 precedent of changing the base OS rather than spending a cog slot, and it needs one reboot.
3. **Processing in the cog:**
   - a 0.5-40 Hz display filter with a 60/50 Hz notch;
   - Pan-Tompkins R-peak detection;
   - heart rate from the median RR over 10 s;
   - SDNN and RMSSD over 60 s;
   - a lead state (`ok`, `leads_off`, `flat`) and a 0-1 quality score.

   No value is reported that the signal doesn't support: heart rate is `null` unless the leads are OK and there are at least 2 RR intervals.
4. **Three outputs:**
   - a JSON report per window on stdout, with the waveforms optional via `--emit-samples`;
   - an 8-float vector to `store/ingest` (id 21);
   - a read-only HTTP export of the last 60 s (`/raw`, `/raw.csv`, `/status`) on `0.0.0.0:8046` (`[api] bind_port`, not loopback-only). It starts before the ADC is found, so the WeftOS hook-up app (`weft-ecg-scope`) can watch wiring progress without SSH. (0.1.0 bound loopback; changed in 0.1.1.)
5. **The ingest read stops at Content-Length.** The Seed agent replies HTTP/1.1 and keeps the socket open after an HTTP/1.0 request. Reading to EOF, as upstream cogs do, stalls for the full 5 s read timeout every cycle (measured on cog0: 8.0 s against 3.0 s for a 3 s window).
6. **It ships a sensor guide** (WeftOS ADR-104): `guide/guide.toml` plus ten Markdown pages, compiled in and served at `GET :8046/guide` (added in 0.1.2).
7. **`--once` exits 0 with `status: no_source`** when the ADC is absent. Continuous mode retries until the ADC appears.

## Consequences

- Hardware beyond the sensor is needed (an ADS1115, about $15), plus one base-OS change per Seed. A firmware OTA may rewrite `config.txt`, so check `/dev/i2c-1` after any update.
- Not in Cognitum's store. It is installed by sideload (`scripts/seed-sideload.sh`), the same `apps/<id>/{binary, manifest.json}` layout the agent uses for its own apps.
- The export is reachable on the LAN and tailnet. It is read-only, but the data is ECG; set `api_bind` to `127.0.0.1:8046` on untrusted networks.
- **The Seed agent has its own ADS1115 driver** (`sensor-config.json` `i2c_sensors`, `PUT /api/v1/sensor/embedding/config`), feeding a 10 Hz, 100 ms-window reflex pipeline. That rate is too slow for an ECG waveform, so this cog reads the chip directly. **Never enable the agent's ADS1115 driver on the same address while this cog runs:** both would write the config register.
- The agent's `PUT /api/v1/apps/<id>/config` replaces the whole config (measured on 0.24.2), so clients must send every key.
- The AD8232 isn't isolated. The README requires battery power while pads are on a person, and every output carries `medical: false`.
- The ingest finding probably applies to every upstream cog. It would explain the "about 6 s per cycle" measured for `fall-detect` on the Seed against about 1 s on a Pi 5, where nothing listens on port 80.

## Spatial evidence: not applicable

This cog has no `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107), although the sensor-cog skill ships one by default. A single-lead ECG measures one person's heart through electrodes on their body. It has no position, range or bearing in the room, so no ADR-107 record type fits. The signal is also medical-adjacent person data, which ADR-107 §9 keeps out of the evidence map.

What might fit later: nothing in the map itself. Heart rate belongs with the vitals streams (this cog's report and store vector), for example as a reference for radar and CSI vitals. If spatial fusion ever needs "a person at this location has a heartbeat", that would come from a positioned sensor such as a vitals radar, not from a body-worn ECG.

## Alternatives

- **A microcontroller (ESP32 or Arduino) as the ADC over UART:** it adds firmware to maintain, and the user's adapter is an I2C ADC.
- **An MCP3008 on SPI:** 10 bits, and it would need SPI enabled as well. The ADS1115's 16 bits at 860 SPS suit ECG better.
- **Routing through `cog-sensor-sources`:** it has no I2C path. Adding one belongs upstream, which is out of scope under the no-PR rule.
