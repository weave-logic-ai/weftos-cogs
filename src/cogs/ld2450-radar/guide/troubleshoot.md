# Troubleshoot

> Match what the output line says to the likely cause and fix.

Run `cog-ld2450-radar --once` and read `health` and `reasons` first.

## `no_source` with `device_error`

The device could not be opened.

- `No such file or directory`: wrong `--device`, or the UART is not enabled (`enable_uart=1`). Check `ls -l /dev/serial0`.
- `Permission denied`: the cog's user is not in the group that owns the device (usually `dialout`).
- `Device or resource busy`: something else has the port open, often `serial-getty`. → see **Set up the Seed**

## `no_source` with `no_bytes`

The port opened but nothing arrived.

- The radar has no power: check 5 V on pin 2 and ground on pin 6.
- The data wire is on the wrong pin: radar `UT` must go to pin 10 (RXD).
- On the Pi's UART: check the console was removed and the Seed was rebooted.

## `no_source` with `no_valid_frames`

Bytes arrive but none form a frame.

- **Wrong baud rate.** The factory rate is 256000. If the radar was reconfigured, try the other documented rates with `--baud`.
- **The login console is still on the UART.** The radar's bytes get echoed and mixed with prompts. → see **Set up the Seed**
- **Config mode.** After an enable-config command the radar stops reporting until end-config. Restart the radar (power-cycle) if a tool left it there.

## `degraded`, `parse_errors` above 0

Frames start but their tails do not match: bytes are being lost or corrupted.

- On the mini UART (`/dev/serial0` → `ttyS0`), lost bytes at 256000 baud are the likely cause. Move the PL011 onto the pins. → see **Set up the Seed**
- Long or loose jumpers. Keep the wires short and seated.
- A weak 5 V supply resets the radar mid-frame.

## `implausible_targets` above 0

The cog dropped targets behind the radar or beyond 10 m. The protocol has no checksum, so these are usually corrupted bytes. Treat it like parse errors.

## `frame_rate_hz` well below 10

The radar sends 10 frames a second. Fewer, with no parse errors, means whole frames are lost (an overloaded CPU on the mini UART), or the radar was busy with a command.

## Targets with nobody there

Moving clutter (fans, curtains), movement behind the wall, or another 24 GHz radar. → see **Placement**

## `config_error`

Only when `query_firmware` or `tracking_mode` is set.

- `no ACK for command`: the Pi TX wire (pin 8 to radar `UR`) is missing or swapped.
- `command refused`: the radar answered with a failure status.

The cog always tries end-config afterwards, so the radar resumes reporting.
