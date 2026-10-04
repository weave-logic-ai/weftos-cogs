# ADR-166: sound-detect, sound-event detection from a KY-038 / LM393 module over an ADS1115 on the Seed's I2C header

**Status:** Proposed
**Date:** 2026-10-03
**Cog:** `sound-detect` 0.1.0

## Context

We want a cheap "is there sound, how often, how long since the last one" signal on the Seed (cog0). The sensor is a KY-038 / LM393 module: an electret microphone, an LM393 comparator and a sensitivity trimpot. It has an analog output (AO, a rough level) and a digital output (DO, high when sound crosses the trimpot threshold). `guide/sensor.md` describes it. The cog's `OUT` wire is whichever output the module exposes, and the code treats it as a voltage.

Constraints shape the design, and they are the same as for `sen0213-ecg` (ADR-158):
- The Seed has no ADC, so an analog or comparator line needs an external converter. The cog uses an ADS1115 on I2C (`src/adc.rs`).
- The Seed image ships with I2C off. `guide/setup.md` gives the enablement (`raspi-config nonint do_i2c 0` or `dtparam=i2c_arm=on`, then a reboot). This repo does not show the base image being changed; see Open questions.
- `cog-sensor-sources` has no I2C source, so the cog talks to `/dev/i2c-N` directly.
- The module and the ADS1115 run at 3.3 V (`guide/wiring.md`, `guide/sensor.md`). Nothing from 5 V may reach the Pi.
- The ECG cog already uses ADS1115 input A0, so this cog defaults to A1 and the two can share one chip (`cog.toml`, `guide/wiring.md`).

## Decision

1. **Digitize with an ADS1115 on I2C**, default bus 1 (`/dev/i2c-1`), default address 0x48, input A1 (`cog.toml` `i2c_bus`, `i2c_addr`, `channel`).
   - `src/adc.rs` writes a config word for continuous conversion, single-ended against GND, PGA plus/minus 4.096 V, 860 SPS, comparator disabled. A unit test pins `config_word(0) == 0x42E3` and `config_word(1) == 0x52E3`.
   - `open` reads the config register back and refuses to start if it does not match, so a missing or wrong chip is reported rather than read as noise.
   - Allowed addresses are 0x48-0x4B (`cog.toml` gives 72-75 decimal). The channel is 0-3.
   - Access is `ioctl I2C_SLAVE` plus 2-byte reads, `libc` only.
2. **Sampling.** The loop reads on absolute deadlines at `--sample-rate`, default 1000 Hz, range 100-2000 Hz (`cog.toml`, `src/main.rs`). A display ring is decimated to about 100 Hz (`RING_HZ`) regardless of the sample rate and keeps 15 s (`RING_SECONDS` in `src/export.rs`).
3. **Detection** (`src/main.rs`, `sample_loop`, `build_report`):
   - A sample at or above `--threshold` volts counts as "above". Default 1.5 V, range 0.05-3.3 V. A rising edge is an event.
   - Events are debounced to one per burst: a second rising edge within 120 ms (`DEBOUNCE_MS`) of the last is ignored.
   - The report is built over the interval (or `--window` with `--once`). `present` is true if the window has an event or any sample above threshold. `status` is `sound`, `quiet`, or `no_samples`.
   - `activity_pct` is the share of window samples above threshold. `events_per_min` is the event count over the last 60 s. `quiet_s` is the time since the last event, or `null` before the first.
   - `level_v` is the window mean and `peak_v` the window maximum.
   - The default of 1.5 V suits the digital OUT, which swings between 0 and 3.3 V. For analog-OUT modules `guide/tuning.md` says to lower it to about 0.3-0.8 V. That is guide advice, not measured here.
4. **Outputs** (four):
   - a JSON report per period on stdout. Fields are listed in `guide/api.md`. The report carries no waveform.
   - an 8-float vector to `POST 127.0.0.1:80/api/v1/store/ingest` as `[[23, vector]]` with `dedup: true`. The elements are `[present, activity, events_per_min/60, peak/3.3, quiet_s/60, level/3.3, threshold/3.3, 0]`, each clamped to 0-1. Id 23 follows the convention in the code comment (21 ecg, 22 tof, 23 sound). It is skipped when the status is `no_samples`.
   - a read-only HTTP export on `0.0.0.0:8049` (`[api] bind_port = 8049`, `bind_loopback_only = false`; `--api-bind` overrides it). Routes in `src/export.rs`: `/status` (latest report), `/raw` (last 15 s of decimated level), `/guide` (the compiled-in guide bundle), `/healthz`. CORS is `*`. It starts before the ADC is found, so a companion app can watch `no_source` turn into a live level (`guide/setup.md`).
   - a sensor guide (WeftOS ADR-104): `guide/guide.toml` plus Markdown pages (`start`, `sensor`, `wiring`, `setup`, `tuning`, `placement`, `api`, `troubleshoot`), compiled in by `src/guide.rs` and served at `GET :8049/guide`. A pinout photo ships beside it.
5. **The ingest read stops at Content-Length** (`response_complete` in `src/main.rs`). This is the same finding as ADR-158 decision 5: the Seed agent answers HTTP/1.1 and keeps the socket open, so reading to EOF would stall until the 5 s read timeout. A unit test covers it.
6. **Simulate, replay and no_source.**
   - `--simulate` uses `SoundSim`: about 3.3 V bursts of about 80 ms every 0.7 s on a near-0 V floor with a little noise, so the whole pipeline produces events without hardware. A unit test checks the detection and the bounded vector.
   - There is no `--replay`. The cog has no recorded-input path; see Open questions.
   - If the ADC cannot be opened, the cog prints `{"status":"no_source","error":...}` on stdout. With `--once` it then exits (the exit code is 0, since `main` returns normally). In continuous mode it retries every `min(interval,5)` s until the chip appears. With `--once` and no simulation, the export is not started when the source is missing.
7. **Console limits** (`cog.toml` `[console]`): only `--once`, `--once --simulate` and `--help` are allowed, with a 15 s runtime cap and a 64 KiB output cap. Options are bounded: an out-of-range value falls back to its default with a warning, and `--i2c-addr` outside 72-75 falls back to 72.
8. **Hardware requirement** in the manifest: `pi-zero-2w` and `v0-appliance`.

## Consequences

- Extra hardware is needed beyond the module: an ADS1115 (shareable with the ECG cog, one input each) and the I2C base-OS setting. `guide/setup.md` says to check `/dev/i2c-1` exists after a reboot; whether a firmware update can undo the setting is not established here.
- The cog reports presence and rate of threshold crossings. It does not give calibrated sound level, audio, or a spectrum (`guide/sensor.md`). `level_v` and `peak_v` are voltages on the OUT line. They are only a loudness proxy on analog-OUT modules, and on a digital OUT they are close to 0 or 3.3 V.
- Setting `--sample-rate` above 860 Hz oversamples the ADS1115. The chip converts at 860 SPS in this configuration, so extra reads return repeated conversions. The default of 1000 Hz is above that rate. The code does not state this as intended; see Open questions.
- The export is reachable on the LAN and tailnet and carries only a level trace and counts. Use `--api-bind 127.0.0.1:8049` on untrusted networks.
- The Seed agent has its own ADS1115 driver (see ADR-158). `guide/troubleshoot.md` says not to enable it on the same address while this cog runs.
- A tuned threshold is room-specific. `guide/tuning.md` and `guide/placement.md` give the procedure (trimpot first, then `--threshold`). The "Measured" section of `guide/sensor.md` is still pending, so no resting level or noise floor is recorded.
- The store id 23 is chosen in code. No registry in this repo was checked for conflicts.

## Spatial evidence: not applicable

This cog has no `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107), although the sensor-cog skill ships one by default. One KY-038 is an omnidirectional electret mic behind a threshold comparator. It reports that a sound crossed the threshold, not where it came from or how far away it was, so it has no position, range or bearing to place in the room's evidence map. None of ADR-107's record types fits.

What might fit later: several mics with sample-synchronous timing could estimate a bearing from time differences of arrival, and that would need a new bearing-only record type (an ADR-107 amendment, like `radar_range` for range-only radar). These modules' comparator output and the ADS1115's rate do not support that. A room-level "sound present" flag would also need its own record type. It is not evidence about occupied space.

## Alternatives

- **A digital GPIO read of DO:** no ADC needed. It loses the analog path and the level and peak fields, and `cog-sensor-sources` has no GPIO source either. The ADS1115 also lets analog-OUT variants work in a degraded way (`cog.toml` description).
- **A microphone ADC or I2S MEMS mic:** would give real audio and a spectrum. It is a different part and a different cog, and this one is scoped to threshold events.
- **Routing through `cog-sensor-sources`:** it has no I2C path. Adding one belongs upstream, out of scope under the no-PR rule.

## Open questions

- Is the I2C enablement applied in the base image, or per Seed by hand? `guide/setup.md` describes the manual steps only, and no repo evidence shows the image changed.
- Whether the sensor on hand is the digital-OUT or an analog-OUT variant, and the real resting level and noise floor. `guide/sensor.md` marks "Measured" as pending, and the guide refers to a `measure-cog.py` run that was not found in this cog's directory.
- Whether sample rates above 860 Hz are intended. See Consequences.
- A `--replay` option, as the sensor-cog workflow expects, does not exist. Whether it is wanted for this cog is not decided.
- **Guide image.** `guide/sensor.md` referenced a `module.jpg` that was never committed, which `guide-check` rejects (gate step 11). The reference was removed on 2026-10-03; a module photo can be added to `guide/` and bundled in `src/guide.rs` later.
- Whether the ADS1115 channel A1 sharing with the ECG cog works with both cogs running at once on one chip. The two cogs each write the config register for their own channel, which would conflict in continuous mode. This is not tested in this repo.
- Whether id 23 is registered or free in the store vector convention.
- On-device proof: no hardware run of this cog is recorded in the repo.
