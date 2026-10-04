# Start here

> A 24 GHz radar that senses people through a wall of plastic — presence and distance, no camera, no config.

The **Ai-Thinker RD-03E** is a 24 GHz FMCW presence + ranging radar (an S3KM111L SoC). It sees a **moving human to about 6 m**, catches **micro-motion** — breathing, a small shift in a chair — to about **3.5 m**, and reports distance to roughly **±5 cm from 30 to 350 cm**. Because it is radar, not a camera, it works in the dark and reads **through thin plastic** (a case, a wall panel) — but not through metal.

The best part: it needs **no configuration**. Power it up and it immediately streams short report frames over a 3.3 V UART. The `rd-03e` cog reads that stream and reports whether a target is present, its distance in cm, the per-minute detection rate, and the quiet time since the last detection.

## The 5-minute path

1. **Wire it.** Four wires: power, ground, and the radar's data line into the Pi's UART. Trust the labels, not the wire colours. → see **Wiring**
2. **Enable the Pi UART** and install the cog. → see **Set up the Seed**
3. **Start the cog** and confirm `/status` shows a live report.
4. **Walk toward it.** Watch `present` flip and `distance_cm` count down.

## Try it with no hardware

The cog has a `--simulate` mode that feeds synthetic radar frames — a person drifting toward and away — so you can see the whole pipeline before you wire anything:

```
cog-rd-03e --once --simulate
```

That prints one report frame and exits. Drop `--once` for a continuous stream.

## Honest limits

- It reports *that* a target is present and *how far*, not *who* or *what*.
- The frame layout is confirmed by community captures; the **gesture codes (state 2-8) are vendor-specific** and reported raw.
- It reads through thin plastic, **not through metal** — mind the enclosure.
