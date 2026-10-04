# Tuning

> Set the sensitivity so the sounds you care about count as events and the room's background does not.

There are two knobs: the module's **trimpot** (hardware) and the cog's **`--threshold`** (software, in volts).

## The trimpot first

The blue trimpot sets the comparator point on the board. With the room quiet:

1. Turn it until the module's second LED (the trigger LED) is just **off**.
2. Make the sound you want to catch (a clap, a voice). The LED should blink on.
3. If it blinks on its own, back off; if it never blinks, turn it up.

## Then the threshold

`--threshold` (default **1.5 V**) is where the cog counts an event on the OUT line.

- For the **digital** OUT (most 3-pin modules): OUT swings 0 ↔ 3.3 V, so 1.5 V sits in the middle — leave it.
- For an **analog**-OUT module: lower it (e.g. 0.3–0.8) so the quieter envelope still crosses it.

Watch `weft-sound-scope`: the green trace is the OUT level, the amber line is the threshold. A clap should push the trace above the amber line and bump the event count.

## Reading it

| Field | Means |
|---|---|
| `present` | sound in this window |
| `events_per_min` | threshold crossings over the last 60 s |
| `quiet_s` | seconds since the last event |
| `activity_pct` | fraction of the window above the threshold |
| `peak_v` | loudest OUT this window |
