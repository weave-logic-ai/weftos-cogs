# Wiring

> Four wires: 5 V, ground, and the two UART lines crossed over. Trust the labels on the boards, not the wire colours.

The radar needs **5 V power** but talks **3.3 V logic**, so its UART pins connect directly to the Pi's 3.3 V UART pins. Hi-Link's manual states that the module's IO level is 3.3 V. Do not power it from the 3.3 V pins.

**Free the UART first.** Until the serial console is moved off it, the Seed will print a login prompt at the radar and read radar bytes as typed input. → see **Set up the Seed**

## Find pin 1

```diagram
header
```

Hold the Seed with the header along the top edge and the SD card on the left. Pin 1 is the top-left pin of the inner row and pin 2 the top-left pin of the outer row. Pins 2, 4, 6, 8 and 10 run along the outer row from the left.

## The wiring diagram

```diagram
wiring
```

## Pin by pin

| LD2450 pin | Seed pin | Typical wire |
|---|---|---|
| `5V` | pin 2 (5V) | red |
| `G` | pin 6 (GND) | black |
| `UT` (radar TX) | pin 10 (GPIO15, RXD) | green |
| `UR` (radar RX) | pin 8 (GPIO14, TXD) | yellow |

**Cross the data lines.** The radar transmits on `UT`, so that wire goes to the Pi's receive pin, pin 10. The Pi transmits on pin 8 into the radar's `UR`.

- **Data (needed):** radar `UT` → pin 10. Without it there is nothing to read.
- **Commands (optional):** pin 8 → radar `UR`. The cog only writes when you ask it to (`query_firmware` or `tracking_mode`). Leave this wire off and the cog still reads targets, and nothing can change the radar's settings from the Seed.

Colours are a convention only.

## Wire it, step by step

1. **Power off** the Seed.
2. Connect pin 2 to `5V` and pin 6 to `G`. Check them against the table, not the colours.
3. Connect pin 10 to `UT`.
4. If you want firmware or mode commands, connect pin 8 to `UR`.
5. Re-read every wire, then power on.

**Final check.** Run `cog-ld2450-radar --once`. A `health` of `ok` with `frames` near 10 means the radar is streaming. → see **Troubleshoot**

## With a USB-serial adapter instead

| LD2450 pin | Adapter pin |
|---|---|
| `5V` | 5V (or a separate 5 V supply) |
| `G` | GND |
| `UT` | RXD |
| `UR` | TXD |

The adapter must use 3.3 V logic. Run the cog with `--device /dev/ttyUSB0`. Nothing on the Seed needs to change.

## Gotchas

- `UT` and `UR` name the radar's own pins. "T" is the radar's output.
- A wrong baud rate looks like garbage: `resync_bytes` climbs and `frames` stays 0. The factory rate is 256000.
- **Not yet verified on hardware:** this pin map follows the radar's labels, Hi-Link's manual and the Pi's documented header, but we have not yet run an LD2450 on our Seed.
