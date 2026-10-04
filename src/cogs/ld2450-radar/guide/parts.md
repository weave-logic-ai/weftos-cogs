# Parts

> What you need: the radar, four jumpers, a rigid mount, and a Seed whose UART you are allowed to free.

## Shopping list

| Part | Notes | Needed |
|---|---|---|
| Hi-Link HLK-LD2450 | 24 GHz tracking radar, 4-pin header `5V G UR UT` | Yes |
| 4 female-female jumpers | Radar header to Seed header | Yes |
| Rigid wall mount | Screws or a printed bracket; the radar must not move | Yes |
| Cognitum Seed | Raspberry Pi Zero 2 W, primary UART on pins 8 and 10 | Yes |
| 3.3 V USB-serial adapter | Optional: use it instead of the Seed's UART (`/dev/ttyUSB0`) | No |

## About the radar

From Hi-Link's instruction manual V1.00 and serial protocol V1.03:

- **Radio:** 24 to 24.25 GHz, FMCW, 250 MHz sweep. The manual describes one transmit and two receive antennas; two receivers are what let it measure angle.
- **Power:** 5 V, about 120 mA on average. The supply must be able to give more than 200 mA.
- **Logic:** the UART runs at 3.3 V, so it connects straight to the Pi's pins. No level shifter.
- **UART:** 256000 baud, 8 data bits, no parity, 1 stop bit (factory default).
- **Output:** up to three targets, 10 frames a second.
- **Coverage:** up to 6 m, ±60° side to side, ±35° up and down.
- **Size:** 15 × 44 mm.
- **Bluetooth:** on by default. Hi-Link's phone app can connect to it and change settings. → see **Set up the Seed**
- **Pins:** the 4-pin header is labelled `5V`, `G`, `UR` (the radar's receive line) and `UT` (its transmit line). The board also has a socket with the same signals; use one or the other.
- **Leave alone:** the BOOT0 pads next to the microcontroller. They are for reflashing the radar's firmware.

## About the Seed

- The Seed's header UART (`/dev/serial0`, pins 8 and 10) is the Linux serial console out of the box. It has to be freed first, which needs a reboot and the device owner's approval. → see **Set up the Seed**
- On a Pi Zero 2 W that UART is the "mini UART" unless Bluetooth is moved off the main UART. The setup page explains which to use at 256000 baud.
- The 5 V pins (2 and 4) come straight from the Seed's USB power input, so a weak power supply will show up as radar resets.

## The USB-serial alternative

A 3.3 V USB-serial adapter (CP2102, CH340 or FTDI) avoids touching the Seed's console. Wire the radar to the adapter (radar `UT` to adapter RX, radar `UR` to adapter TX, ground to ground), power the radar from 5 V, and run the cog with `--device /dev/ttyUSB0`. **Use a 3.3 V adapter or set its jumper to 3.3 V.**

→ see **Wiring**
