# Start here

> Read a single-lead ECG on a Cognitum Seed, then check and tune it in the weft-ecg-scope app.

This guide covers the DFRobot SEN0213 (an AD8232 ECG front end) connected to a Cognitum Seed through an ADS1115 ADC. The `sen0213-ecg` cog reads the signal, finds the heartbeats, and reports heart rate, beat intervals and a quality score.

The SEN0213 gives an analog voltage. The Seed (a Raspberry Pi Zero 2 W) has no analog input, so an ADS1115 turns the voltage into numbers on the I2C bus.

**This is not a medical device.** Don't use it for diagnosis. → see **Safety**

## The 5-minute path

The wiring takes the longest. Everything else is a few commands.

1. **Get the parts.** An ADS1115 breakout, jumper wires, fresh electrode pads, and a battery or power bank. → see **Parts**
2. **Wire it.** Four wires to the Seed header, three to the sensor. Trust the labels on the boards, not the wire colours. → see **Wiring**
3. **Enable I2C on the Seed.** It is off on the stock image. One config line and a reboot. → see **Set up the Seed**
4. **Install and start the cog.** Sideload it, then start it from the app or with `curl`. → see **Set up the Seed**
5. **Open the companion app** `weft-ecg-scope` and follow its hook-up checklist, top to bottom. Each failing step names the fix.
6. **Put the pads on** and keep still. → see **Electrodes**
7. **Tune it.** Pick the right mains notch, check the polarity, and check the heart rate against your pulse. → see **Calibrate**

You can test the software with no hardware at all: the cog has a `--simulate` mode that makes a synthetic 72 bpm ECG.

## How the signal flows

```diagram
flow
```

The cog reads the ADC at 250 Hz, filters the waveform for display, finds R-peaks with the Pan-Tompkins method, and works out the rest from the beat times.

## What you get

| Output | Where | Use |
|---|---|---|
| JSON report, one per window | stdout; the Seed keeps it in `GET /api/v1/apps/sen0213-ecg/logs` | Heart rate, RR intervals, SDNN, RMSSD, quality, sampling health |
| 8-float vector, id 21 | The Seed's store | Compact history of the report |
| Live signal export | `http://<seed>:8046` (`/status`, `/raw`, `/raw.csv`) | The last 60 s of raw and filtered waveform; what the app reads |

→ see **API reference** for every field and endpoint.

## Honest limits

- The cog reports `null` instead of guessing: no heart rate unless the leads are on and at least two beats were found.
- `quality` measures how regular the beat intervals are. It does not measure signal cleanliness, and an irregular rhythm lowers it too.
- Verified on our Seed (cog0, 2026-10-01): the software, the simulate mode, the `no_source` report and the export from a browser. **Not yet verified:** a real ADS1115 and SEN0213 on the header, and a real person.

## Pages in this guide

Parts, Wiring, Electrodes, Set up the Seed, Calibrate, Troubleshoot, API reference, Safety, Glossary.
