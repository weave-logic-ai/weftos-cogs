# ADR-167: HLK-LD1040C 10 GHz Doppler motion radar reader cog

**Status**: Proposed
**Date**: 2026-10-04
**Renumbered**: 2026-10-06
**Cog**: `ld1040c-motion` 0.1.0
**Predecessor number**: provisional ADR-166, which collided with sound-detect. The redirect is `docs/adrs/ADR-166-ld1040c-motion.md`. ADR-166 is the sound-detect decision.

The original file called 166 provisional, the next free number after ADR-165, and said it would be reassigned on export. sound-detect already held ADR-166 and is dated 2026-10-03. This decision keeps its text and takes 167, the next unused number in this repository.

## Context

The part is a Hi-Link HLK-LD1040C, photographed in hand (silk `HLK-LD1040C V1.1`). It is a
10.525 GHz X-band **Doppler** motion-sensing module (1T1R, fixed frequency) with an integrated
MCU that does the IF demodulation, amplification and digital processing. Unlike the FMCW parts
(`ld2450-radar` ADR-161, `ld6002-radar` ADR-157), a Doppler sensor detects **moving** targets
only — it reports motion/occupancy, not range or position. Hi-Link positions it for smart
lighting (ceiling mount, 110° ±10° beam, ~3–4 m sensing radius; 8–10 m wall-radial). It does
not penetrate walls.

Its 5-pin header is labelled `VCC · GND · TX/SDA · RX/SCL · OUT` plus a `D1` status LED. The
module exposes three interfaces:

- **`OUT`** — a TTL digital line, HIGH on motion, with a **5 s hold** (delay) after each trigger
  and a **2 s block** before it can re-trigger. There is a **7–9 s power-on warm-up** during
  which readings are invalid. This is the reliable ground-truth signal and the cog's primary
  input.
- **UART** (`TX`/`RX`) — a frame protocol (header `0x3C 0x3A`, footer `0x3A 0x3E`, all multi-byte
  fields big-endian) carrying the mid-frequency AD value (motion amplitude), SUM0 noise, SUM2
  signal strength, and the sensitivity/threshold/delay config. **Baud is not documented** in the
  datasheet, so UART is a best-effort, off-by-default richer-data path, not ground truth.
- **I²C** (`SDA`/`SCL`) — not used by this cog.

The target host is a Cognitum Seed (Raspberry Pi Zero 2 W, armv7). The cog reads `OUT` as a
Linux GPIO input on `/dev/gpiochip0` (BCM 17, header pin 11 — a free general-purpose pin). On a
Pi 5 / v0-appliance the header is `gpiochip4` with different offsets; a `--gpiochip` override is
a follow-on if the appliance variant is needed. `cog-sensor-sources` has no GPIO source, so the
cog opens the chip line directly with `gpio-cdev` (Linux-gated; the host build stubs it).

## Protocol / datasheet sources

| Document | Version, date | Used for |
|---|---|---|
| Hi-Link "Radar Sensor Module HLK-LD1040C Datasheet" (Shenzhen Hi-Link Electronic Co.) | undated, fetched 2026-10-04 (rajguruelectronics.com mirror) | Frequency (10.525 GHz), power (5–12 V / ~50 µA / Vout 3.2–3.4 V), 7–9 s warm-up, 110° beam, 3–4 m radius, `OUT` 5 s delay / 2 s block, startup modes, the UART "Mobile and Hand Scan Inductive Communication Protocol" field table |
| Hi-Link / hlktech.net product pages (10G radar HLK-LD1040 / LD1040C) | fetched 2026-10-04 | Doppler principle, UART vs IFC (I²C) variants, application positioning |

## Decision

Ship `ld1040c-motion`: read `OUT` via GPIO (BCM 17, 100 Hz sampling) and report per window the
current held presence (`present`), rising-edge count (`motion_events`), the fraction of the
window motion was asserted (`active_fraction`), `seconds_since_motion`, and a `warming` flag for
the first ~9 s. When `--uart <dev>` is set, best-effort parse the frame and add
`motion_amplitude` / `signal` / `noise`, emitting `null` when a field is not parsed — never
fabricated. `--simulate` synthesizes a walking-person pattern through the same sampler honoring
the 5 s hold, so the cog runs with no hardware. One JSON line per window to stdout; an 8-float
store vector POSTed to the Seed store (id 27):

`[present, motion_events_per_min/30, active_fraction, seconds_since_motion/60,
motion_amplitude/255 or 0, signal/1024 or 0, warming, reserved]`

Export on `127.0.0.1:8053` (loopback) with `/status /raw /guide /healthz`. ADR-104 guide ships
in `guide/`.

## Consequences

- Cheap motion/occupancy layer alongside the FMCW position/vitals radars; no range or position.
- **Open, to verify on the bench**: the UART baud (defaulted 9600) and the frame byte offsets
  (inferred from the datasheet field order) need a real capture before UART telemetry is trusted;
  the store-vector normalization constants (÷30, ÷60, ÷255, ÷1024) are initial estimates to tune
  to observed ranges; the GPIO line is correct for the Pi Zero 2 W Seed, not a Pi 5 appliance.
- No sensor is wired to the Seed's GPIO 17 at deploy time, so the cog correctly reports
  `present: false` until the module is connected (`VCC→5V`, `GND→GND`, `OUT→pin 11`).
