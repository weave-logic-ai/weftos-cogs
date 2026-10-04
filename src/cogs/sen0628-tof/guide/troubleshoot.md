# Troubleshoot

> Symptom, likely cause, fix. Start with the first failing step in the app's checklist.

## Reading the status

| `status` | Meaning |
|---|---|
| `no_source` | The cog can't talk to the sensor. The report has an `error`. |
| `no_frames` | The sensor was found but no frames arrived in the report window. |
| `no_targets` | Frames arrive but no zone has a valid reading. |
| `ok` | Frames with at least one valid zone. |

## Symptoms

| Symptom | Likely cause | Fix |
|---|---|---|
| `no_source`, "nothing answered at 0x33" | No device answered | Check wiring, the I2C/UART setting and the address. |
| `no_source`, "set 8x8 mode: no response" | The board's firmware is busy | Power-cycle the board. |
| `no_source`, error contains "No such file" | I2C is off, so `/dev/i2c-1` is missing | Enable I2C and reboot. → see **Set up the Seed** |
| `no_frames` | Frames are failing to read | Read the cog's log for "frame error". Check wiring, then power-cycle. |
| `no_targets` | Nothing in range, or the lens is covered | Uncover the lens, point it at a surface within 3.5 m. |
| Many invalid zones | Glass, sunlight, or out of range | → see **Mounting** |
| Noisy zones | Dark or shiny surface, or sunlight | → see **Calibrate** |
| Presence always on | Background learned with people in view, or margin too small | Restart the cog with the scene empty. Raise `presence_mm`. |
| Presence never on | Background not learned, or margin too large | Check `background_learned`. Lower `presence_mm`. |
| Nothing answers after changing the address | The board was not power-cycled | Power-cycle the board. Set `i2c_addr` to match. |
| Export unreachable on `:8047` | Cog not running, or `api_bind` is loopback | Start the cog. Check `api_bind` is `0.0.0.0:8047`. |
| Config changes seem "lost" | `PUT` replaced the whole config | Always send every key. → see **API reference** |
| `no_source` after a firmware update | The update reset `config.txt` | Check `/dev/i2c-1`; re-apply the I2C steps. |
| `frame_rate_hz` below the setting | The Seed is busy | Lower `rate_hz`. Stop other heavy apps. |
| Browser can't reach the Seed on port 8443 | Self-signed TLS | Use plain HTTP on port 80. |

## no_source in detail

### Nothing answered at 0x33

The cog opened `/dev/i2c-1` but the sensor did not respond to a read at its address. Check, in order:

1. Pin 1 to `+`, pin 6 to `-`, pin 3 to `D`, pin 5 to `C`. Wiggle each wire.
2. The I2C/UART switch is on I2C. Check the silkscreen next to the switches. **Unverified:** we do not have the positions.
3. The cog's `i2c_addr` matches the board: 51 for 0x33, 50 for 0x32, 49 for 0x31, 48 for 0x30.
4. You power-cycled the board after any change to the address or the I2C/UART setting.

### Set 8x8 mode: no response

The board answered but its firmware did not reply to the mode command within 8 s. It is busy or stuck, often after a previous run or a change of mode. Power-cycle the board and wait. Mode changes take about 5 s.

### Other errors

"ioctl I2C_SLAVE" or "address ... is not a SEN0628 address" means the address is outside 0x30 to 0x33. "open /dev/i2c-1" means I2C is off or the bus number is wrong.

## Check I2C without i2cdetect

`i2cdetect` is not on the Seed. Instead:

1. Confirm `ls /dev/i2c-1` works over SSH.
2. Start the cog and read `curl http://<seed>:8047/status`.
3. "nothing answered" means no device answered. "No such file" means I2C is off. A status of `ok` means the board responded.

## I2C missing after an OTA

A Seed firmware update can reset `config.txt` and remove `dtparam=i2c_arm=on`. After every update, check `ls /dev/i2c-1`. If it is gone, re-apply the I2C steps and reboot. The ECG cog breaks the same way.

## Frame rate below the setting

The cog polls the board over I2C. The Pi Zero is slow and does other work. If `frame_rate_hz` is below `rate_hz`, lower `rate_hz` to what it achieves, and stop other heavy apps. A lower rate is fine for presence.

## Quick isolation order

1. Run `--once --simulate`. If it fails, the problem is the cog or the Seed, not your wiring.
2. Run `--once` with the sensor wired. Expect `ok` or `no_targets`, **not** `no_source`.
3. Point it at a wall about 1 m away and check `valid_pct`.

## Last resorts

- Power off, and re-seat every wire from the diagram.
- Measure 3.3 V between pins 1 and 6, and again at the sensor's `+` and `-`.
- Power-cycle the sensor board, not just the cog.
