# Start here

> Detect sound/noise on a Cognitum Seed, then watch it live and tune it in the weft-sound-scope app.

This guide covers a KY-038 / LM393 sound-detection module — an electret microphone with a comparator and a sensitivity trimpot — connected to a Cognitum Seed through an ADS1115 ADC. The `sound-detect` cog reads the module's OUT line and reports whether sound is present, the per-minute event rate, the quiet time since the last event, and the activity level.

The module's OUT is a voltage. The Seed (a Pi) has no analog input, so an ADS1115 turns it into numbers on the I2C bus — the **same ADC the ECG cog uses**, on a different channel (A1), so both can share it.

## The 5-minute path

1. **Wire it.** Three wires to the module, and the ADS1115 to the Seed header. Trust the labels, not the wire colours. → see **Wiring**
2. **Enable I2C on the Seed** and install the cog. → see **Set up the Seed**
3. **Open `weft-sound-scope`** and follow the hook-up checklist, top to bottom. Each failing step names the fix.
4. **Tune it.** Turn the trimpot (or set `--threshold`) until a clap counts but the room's hum does not. → see **Tuning**
5. **Place the mic** where it hears what you care about. → see **Placement**

You can test the software with no hardware at all: the cog has a `--simulate` mode that makes synthetic sound bursts.

## How the signal flows

```diagram
flow
```

## What you get

| Output | Where | Use |
|---|---|---|
| JSON report, one per window | stdout; `GET /api/v1/apps/sound-detect/logs` | present, events/min, quiet_s, activity, level, peak |
| 8-float vector, id 23 | the Seed's store | compact history |
| Live export | `http://<seed>:8049` (`/status`, `/raw`) | the last ~15 s of the OUT level; what the app plots |

## Honest limits

- A threshold detector reports *that* a sound happened, not *what* it was.
- The common 3-pin module gives a **digital** OUT (high on sound). The cog also handles an analog-OUT variant, degraded.
- **Not yet verified on hardware:** the pin map is checked against the board labels and the cog's code, but we have not yet run a real module + ADS1115 on our Seed.
