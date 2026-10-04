# Getting started

The **HLK-LD1040C** is a 10.525 GHz X-band Doppler radar that detects **movement**. It is a
ceiling-mount motion/occupancy sensor with roughly a **3–4 m** sensing radius and a wide **110°**
beam. It does **not** see through walls, and it only reacts to things that **move** — a person who
sits perfectly still eventually reads as "clear".

What to expect:

1. **Warm-up.** For the first **7–9 seconds** after power-on the module is settling. The cog flags
   these readings with `"warming": true`; treat them as invalid.
2. **Motion → OUT HIGH.** When something moves in the beam, the module drives its **OUT** pin HIGH.
3. **5 s hold.** OUT stays HIGH for about **5 seconds** after the last motion (the output delay), so
   brief gaps in movement do not flicker the line.
4. **2 s block.** After OUT falls, the module will not re-trigger for about **2 seconds**.

The cog reads the OUT pin on a Pi GPIO and, once per window, reports whether motion is currently
held, how many motion onsets occurred, the fraction of the window OUT was high, and how long since
the last motion.

Try it with no hardware:

```
cog-ld1040c-motion --once --simulate
```

Then wire the module (see **wiring**) and run `--once` against the real OUT line.
