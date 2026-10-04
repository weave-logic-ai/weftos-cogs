# Troubleshooting

### `status` is `no_source`

The cog never sampled the OUT line.

- Check the GPIO exists and you can open it: `ls -l /dev/gpiochip0`, and that the cog's user is in
  the `gpio` group.
- Confirm `--gpio N` matches the header pin you wired OUT to (default BCM 17 = pin 11).
- On a non-Linux host there is no GPIO — use `--simulate`.

### Always `present`, never clears

- The module holds OUT HIGH for ~5 s after the last motion, and anything moving nearby (a fan, a
  curtain, a pet) keeps re-triggering it. Aim the beam away from constant movement.
- A Doppler radar is sensitive; reduce the sensing area by angling the ceiling mount.

### Never `present`, even when you move

- You may still be in the **7–9 s warm-up** (`"warming": true`). Wait, then move again.
- Check OUT is actually wired to the pin named by `--gpio`, and that the module has **5 V** on VCC.
- Remember the radius is only ~3–4 m and it needs **motion** — walk, do not stand still.

### `motion_events` is 0 but `present` is true

That is expected when motion began **before** this window started: the onset (rising edge) was
counted in an earlier window and OUT is still within its 5 s hold.

### UART fields stay null

The OUT line is the ground truth; UART is best-effort. The baud rate is undocumented — try
`--baud 115200` (and 57600/38400/19200). Confirm TX/RX are not swapped.
