# The sensor

> Ai-Thinker RD-03E — a 24 GHz FMCW radar (S3KM111L SoC) that senses people by motion and micro-motion and reports a distance.

## What it is

A single-chip **24 GHz FMCW radar**. It emits a tiny radio chirp and measures the
reflection, so it detects a person by movement — including the micro-motion of
someone sitting still — and reports how far away they are. No camera, no lens.

## What it's good for

- **Presence / occupancy** — "is someone in this zone," even if they're still.
- **Rough ranging** — distance to the nearest person in centimetres.
- **Through thin non-metal** — it sees through plastic enclosures and thin
  drywall, so it can hide inside a housing.
- **Privacy-friendly, dark/smoke-proof** — no image, works in total darkness.

## What it's *not* for

- **Identifying who/what** — it sees motion and range, not identity.
- **Counting people precisely** or multi-target X/Y tracking — that's the RD-03**D**.
- **Seeing through metal**, or exact positioning / fast gesture UIs without tuning.

## Datasheet spec (vendor, not our measurement)

| | |
|---|---|
| Band | 24.0–24.25 GHz FMCW |
| Range | moving human to ~6 m; micro-motion to ~3.5 m |
| Accuracy | ±5 cm (30–350 cm), ±5 % (350–600 cm) |
| Ranging beam | azimuth ±20°, elevation ±45° (wall-mounted, Rd-03E Specification V1.0.0 §7.2) |
| UART | 256000 8N1, 3.3 V TTL; streams on power-up |

Source: Ai-Thinker / openelab product pages — treat as datasheet, not our bench data.

## What to expect from the output

- **Empty zone:** `status` `clear`, `distance_cm` 0, `state_name` `no_target`.
- **Someone walks in:** `status` `present`, a `distance_cm` that tracks them, and
  a counted detection. It holds briefly after they stop, then clears.
- Frames stream at a few tens of Hz; `state` 2–8 are vendor gesture codes (raw).

## Calibration

1. Mount with the **antenna face toward the zone**; keep metal off the face.
2. Presence needs no zeroing. For distance, check the reading against a tape
   measure at ~1 m and ~3 m and note any offset.
3. Adjust height/angle so the zone you care about is covered; re-test.

## Measured

> Pending: wire this radar and run
> `scripts/measure-cog.py http://<seed>:8050 --seconds 30 --label "RD-03E @ 1 m walk-in"`.
> Real update rate, the distance it reports at known marks, and how long presence
> holds after you stop land here.
