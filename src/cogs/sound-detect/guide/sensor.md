# The sensor

> KY-038 / LM393 electret-mic + comparator module, read through an ADS1115 ADC. One module per mic; the audio cog scales to many.

## What it is

A cheap electret microphone wired to an **LM393 comparator** with a sensitivity
trimpot. It has two outputs: **AO** (a rough analog level) and **DO** (a digital
HIGH when the sound crosses the trimpot threshold). The cog reads it through an
**ADS1115** 16-bit I²C ADC so several mics share one chip.

## What it's good for

- **Sound-event detection** — claps, doors, knocks, a voice starting, a machine
  turning on. "Is there sound, how often, how loud-ish."
- **Activity / noise-rate monitoring** over time (events per minute, quiet time).
- **Triggering** other cogs or scenes on a loud event.
- **Many cheap channels** — one ADS1115 = up to 4 analog mics; four ADS1115 on a
  bus = 16; networked mic nodes beyond that.

## What it's *not* for

- **Calibrated SPL / decibels** — this is a threshold/comparator part, not a
  metering mic. The analog level is relative, not a dB reading.
- **Recording or audio quality** — there is no audio path, only a level.
- **Frequency analysis** — no spectrum; it's loudness/threshold only.

## Datasheet spec (vendor, not our measurement)

| | |
|---|---|
| Mic | electret, omnidirectional |
| Comparator | LM393, trimpot threshold, AO + DO outputs |
| ADC | ADS1115, 16-bit, up to 860 SPS, I²C 0x48–0x4B |
| Logic | 3.3 V (never 5 V into the Pi) |

## What to expect from the output

- **Quiet room:** a low, steady `level_v`, `activity_pct` near 0, few or no events.
- **A clap / voice:** a spike in `level_v`/`peak_v`, one counted event, `present`
  goes true briefly.
- `events_per_min` and `activity_pct` rise with how noisy the room is.
- Pick the mic you're on with the **sensor picker** (banner shows the channel).

## Calibration

1. In the actual quiet room, turn the module's trimpot until the **DO LED just
   stops** blinking on ambient noise, then back off a hair.
2. Run `measure-cog.py` to read the resting **mean** and **noise floor**; set
   `--threshold` just above `mean + a few × stddev` so quiet = no events and your
   target sound = events.
3. Re-check after moving the mic — every room and mount is different.

## Spatial evidence: not applicable

This cog has no `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107). One KY-038 is an omnidirectional electret mic behind a threshold comparator. It reports that a sound crossed the threshold, not where it came from or how far away it was, so it has no position, range or bearing to place in the room's evidence map. None of ADR-107's record types fits.

What might fit later: several mics with sample-synchronous timing could estimate a bearing from time differences of arrival, and that would need a new bearing-only record type (an ADR-107 amendment, like `radar_range` for range-only radar). These modules' comparator output and the ADS1115's rate do not support that. A room-level "sound present" flag would also need its own record type. It is not evidence about occupied space.

## Measured

> Pending: wire this mic and run
> `scripts/measure-cog.py http://<seed>:8049 --seconds 30 --label "Mic 1, quiet kitchen"`.
> The real resting level + noise floor land here (they set the right threshold).
