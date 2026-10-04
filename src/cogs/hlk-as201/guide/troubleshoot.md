# Troubleshoot

> Most problems are the wrong port or baud, the Pi UART not freed, or a sensor that needs calibration. The protocol itself is confirmed against the datasheet.

**`no_source` — no serial device.**
- Wrong port: the adapter appears as `/dev/ttyUSB0`, the Pi header UART as `/dev/serial0`. Set `--port` to match.
- USB-serial adapter not plugged in, or the Pi UART still owned by the serial login console → see **Wiring** (enable the UART, disable the console).
- Check direction: **data = sensor TX → Pi RXD**. If TX and RX are swapped you get silence.

**`no_frames` — the port opens but no valid frames arrive.**
- Wrong baud: the AS201 default is 115200, but it can be set from 4800 to 921600 — try the others.
- No `FA FB … FC FD` frame ever validates → check TX/RX direction and that the sensor is powered at 3.3 V.
- Data reporting may be switched off on the module (datasheet `0x1A`); the default is on.

**The numbers look wrong (impossible angles, modulus ≠ 1 g at rest, drifting heading).**
- **Calibrate.** Run the accel zero-bias (`0x1C`, flat and still) — afterwards the acceleration
  modulus should sit at ≈ 1.0 g in any static orientation. The heading needs the magnetometer
  figure-8 (`0x1D` rotate about X/Y/Z, then `0x1E`); until then `mag_accuracy` stays low.
- Keep magnets, motors and speakers ≳ 20 cm away — magnetic interference corrupts the heading.
- The protocol is confirmed (datasheet V1.1), so wrong-looking numbers are calibration or wiring,
  not a decode mismatch.

**Gyro is noisy / the zero drifts.**
- Vibration: mount the sensor flat, still and rigidly decoupled from motors or fans. The gyro is sensitive to mechanical noise.
- Let it settle for a few seconds after power-up before trusting the zero.

**Nothing in the app.**
- The cog must be running (export on `:8051`), and the app's Seed host must point at the Seed. Over USB that is `169.254.42.1`; on the LAN/tailnet use the Seed's IP.
- In a browser served from `localhost`, Chrome may block calls to a private IP (Private Network Access) — use the native app, or serve the page from the Seed.
