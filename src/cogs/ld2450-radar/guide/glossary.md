# Glossary

> Plain-word definitions of the terms used in this guide.

| Term | Meaning |
|---|---|
| **LD2450** | Hi-Link's 24 GHz radar module that tracks up to three moving targets. |
| **FMCW** | Frequency-modulated continuous wave. The radar sweeps its frequency and measures distance from the echo's frequency shift. |
| **Target** | One tracked object the radar reports: x, y, speed and gate size. Usually a moving person. |
| **Slot** | One of the three target places in a frame. A slot number is not a person's identity. |
| **Boresight** | The line straight out from the antenna face. The radar's y axis. |
| **radar_local** | The cog's coordinate frame: y along the boresight, x sideways, in metres. |
| **room_enu** | The spatial engine's room frame: metres east and north from the room's south-west corner. |
| **Yaw** | The direction the radar faces, in degrees counter-clockwise from east: 0 east, 90 north. |
| **Distance gate** | The radar's distance step. `resolution_mm` reports its size. |
| **Sign-magnitude** | How the radar encodes x, y and speed: the top bit is the sign (1 means positive) and the other 15 bits the size. |
| **UART** | The two-wire serial link (TX and RX) between the radar and the Seed. |
| **Baud** | Symbols per second on the UART. The radar's factory setting is 256000. |
| **8N1** | 8 data bits, no parity, 1 stop bit. |
| **Mini UART** | The Pi's simpler UART (`ttyS0`). Its baud follows the core clock and it buffers only 8 bytes. |
| **PL011** | The Pi's full UART (`ttyAMA0`), with deeper buffers and a clock independent of the core. |
| **Serial console** | A Linux login and boot log on the UART. It must be removed before the radar can use the UART. |
| **ACK** | The radar's reply to a command, with a status (0 success, 1 failure). |
| **Config mode** | Entered with the enable-config command. The radar stops reporting until end-config. |
| **Resync** | Skipping bytes until the next valid frame header. |
| **Cog** | A small app that runs on a Cognitum Seed. This one is `ld2450-radar`. |
| **Seed** | The Cognitum appliance. Ours is a Raspberry Pi Zero 2 W. |
