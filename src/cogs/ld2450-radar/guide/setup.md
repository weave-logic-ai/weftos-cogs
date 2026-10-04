# Set up the Seed

> Free the header UART from the serial console (with the owner's approval), pick the right UART for 256000 baud, then install and run the cog.

## Before you change anything

These steps change how the Seed boots and remove its serial console, which is a recovery path if the network fails. **Get the device owner's approval first.** Make sure you can still reach the Seed over the network or USB before you remove the console.

Check the current state (read-only):

```
cat /boot/firmware/cmdline.txt        # look for console=serial0,115200 or console=ttyS0,115200
systemctl is-enabled serial-getty@ttyS0.service
ls -l /dev/serial0                    # -> ttyS0 means the mini UART is primary
grep -E 'enable_uart|dtoverlay|core_freq' /boot/firmware/config.txt
systemctl is-enabled hciuart.service  # is the Pi's own Bluetooth in use?
```

On older images the files are in `/boot/` instead of `/boot/firmware/`.

## Which UART: mini UART or PL011

On a Pi Zero 2 W the header pins 8 and 10 carry the **mini UART** (`ttyS0`) by default, and the better **PL011** UART (`ttyAMA0`) drives the Pi's Bluetooth. Raspberry Pi's documentation says the mini UART has smaller FIFOs, no flow control and no framing-error detection, which makes it "more prone to losing characters at higher baud rates", and that its baud rate follows the VPU core clock.

What that means at the radar's 256000 baud:

- **The rate itself is fine.** With the console enabled the core clock is fixed at 250 MHz (`enable_uart=1`). The mini UART's divider then gives 250 MHz / (8 × 122) = 256 148 baud, 0.06 % off, well inside UART tolerance. (Formula from the BCM2835 peripherals datasheet, §2.2.1.)
- **Dropped bytes are the risk.** The mini UART holds only 8 received bytes. At 256000 baud that is about 0.3 ms of data, so a busy CPU can lose bytes. The PL011 holds 16 and its clock does not depend on the core clock.

**Recommendation:** move the PL011 to pins 8 and 10.

- If the Seed does not use the Pi's own Bluetooth: `dtoverlay=disable-bt` (and `sudo systemctl disable hciuart`).
- If it does: `dtoverlay=miniuart-bt` with `core_freq=250`, which moves Bluetooth to the mini UART instead.

Either way `/dev/serial0` then points at `ttyAMA0` and the cog's default device still works. **Unverified on our Seed:** the mini UART may well be good enough at 10 frames a second. If you stay on it, watch `bad_tail` and `resync_bytes` (→ see **Troubleshoot**). Lowering the radar to 115200 baud with its baud-rate command is another option; the cog does not send that command.

## Free the UART (needs approval)

1. Remove the serial console from the kernel command line. Edit `/boot/firmware/cmdline.txt` (one line) and delete only the `console=serial0,115200` entry (or `console=ttyS0,115200`). Keep `console=tty1` and everything else.
2. Stop the login prompt on the UART:

   ```
   sudo systemctl disable --now serial-getty@ttyS0.service
   ```

3. Keep the UART hardware on: `/boot/firmware/config.txt` must still have `enable_uart=1`.
4. Optional, recommended: add `dtoverlay=disable-bt` (or `miniuart-bt` plus `core_freq=250`) to `config.txt`, as above. After that the getty to disable is `serial-getty@ttyAMA0.service` if one was enabled.
5. Reboot.

The same change through Raspberry Pi's tool is `sudo raspi-config` → Interface Options → Serial Port → login shell **No**, serial hardware **Yes**.

After the reboot `cat /proc/cmdline` shows no `console=` for the UART, and `ls -l /dev/serial0` shows which UART is on the pins. The cog's OS user needs read and write access to it (usually the `dialout` group).

## Install and run

```
scripts/cross-build.sh ld2450-radar
cog-ld2450-radar --once --simulate                        # no hardware
cog-ld2450-radar --once                                   # /dev/serial0, 256000
cog-ld2450-radar --once --query-firmware                  # also reads the version (needs pin 8 wired)
cog-ld2450-radar --interval 1                             # continuous, export on 127.0.0.1:8052
cog-ld2450-radar --once --device /dev/ttyUSB0             # USB-serial adapter
```

## First-run checklist

1. `--once --simulate` prints one line with `"simulated":true` and a target. The software works.
2. `--once` prints `health` `ok` and `frames` between 8 and 12. The radar is streaming at its 10 Hz.
3. With nobody in view, `target_count` is 0 and `targets` is `[]`.
4. Walk slowly 2 m in front of the radar: `target_count` becomes 1 and `y_m` is near 2.
5. Step sideways to your right and note whether `x_m` goes up or down. → see **Placement**
6. `parse_errors` stays 0 and `quality` stays at or near 1.0 over a minute of continuous running.

## Radar Bluetooth

The LD2450's own Bluetooth is on from the factory, and Hi-Link's phone app can change the radar's settings over it. On a shared site, consider turning it off with the radar's Bluetooth command (0x00A4, then a restart). The cog never sends it; it is a configuration change for the owner to approve.

## How it flows

```diagram
flow
```
