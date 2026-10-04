# About the sensor

The HLK-LD1040C is a **Doppler motion radar** operating in the 10.525 GHz X-band. Doppler radar
measures the frequency shift of its own reflected signal, so it responds to **radial motion**
(movement toward or away from the sensor) — not to static objects.

| Property | Value |
|---|---|
| Frequency | 10.525 GHz (X-band) |
| Detects | Moving / micro-moving targets only |
| Sensing radius | ~3–4 m |
| Beam | ~110°, best mounted on a ceiling facing down |
| Warm-up | 7–9 s after power-on |
| Output hold | ~5 s after last trigger |
| Re-trigger block | ~2 s after OUT falls |
| Through walls | No |

## OUT pin — the reliable signal

```diagram
flow
```

The **OUT** pin is a 3.3 V TTL digital line: HIGH while motion is asserted, LOW otherwise, with the
hold and block timing above. This is the ground truth the cog is built around.

## What it is NOT

- **Not presence detection.** A still person is invisible to Doppler radar. Use OUT for *motion*,
  not for *is someone in the room*.
- **Not ranging.** Unlike an FMCW radar (e.g. RD-03E), the LD1040C reports motion, not distance.
- **Not a camera.** No identity, no image, no pose.

## Optional UART telemetry

Some LD1040C units expose a serial frame carrying internal amplitudes (motion amplitude, signal and
noise). The cog can parse this best-effort when `--uart` is set, but it is **off by default** and
its baud rate is undocumented — see **telemetry**.
