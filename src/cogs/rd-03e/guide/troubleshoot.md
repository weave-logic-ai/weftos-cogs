# Troubleshoot

> The checklist step that fails names the fix. Here are the common ones.

**`no_source` / "open /dev/ttyUSB0" failed.**
- Wrong port: the adapter landed on a different device. List what's there (`ls /dev/ttyUSB* /dev/ttyACM* /dev/serial*`) and pass it with `--port`.
- No adapter present: a USB-serial dongle isn't plugged in, or the Pi UART isn't enabled. For the Pi UART, enable it and use `--port /dev/serial0` — see **Wiring → Enable the Pi UART**.
- Permissions: the user running the cog must be in the `dialout` group to open a serial device.

**Garbage frames / `no_frames` forever.**
- Wrong baud. The RD-03E runs at **256000** — that's the default; don't override it to 115200. If you changed `--baud`, set it back.
- Wrong wire direction: the module's **TX** must reach the Pi's **RXD** (pin 10). Data flows module → Pi; if it's crossed you get silence or noise.
- A 5 V USB-serial adapter can corrupt the stream (and damage the module) — use a **3.3 V** adapter.

**Never shows present.**
- Power or orientation: confirm `VCC` is on 3.3 V (pin 1), not 5 V, and that the module's antenna face is pointed at the space you're standing in.
- The module streams on power-up with no config; if `/status` stays `no_frames`, it isn't powered or the data wire is wrong — recheck **Wiring**.

**It misses someone / sees through the wrong wall.**
- The RD-03E reads **through thin plastic, not metal**. A metal enclosure, a foil-backed panel, or a metal shelf in front of it blocks or reflects the beam. Mount it behind plastic with a clear line to the space.
