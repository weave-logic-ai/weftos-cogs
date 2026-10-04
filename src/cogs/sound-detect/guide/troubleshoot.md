# Troubleshoot

> The checklist step that fails names the fix. Here are the common ones.

**`no_source` / ADS1115 not found.**
- I2C not enabled → see **Set up the Seed**.
- Wrong address: if `ADDR` is on VDD, set the cog's `i2c_addr` to 73.
- A wrong wire looks the same as a missing chip — re-check SDA/SCL against the diagram, not the colours.
- Don't enable the Seed agent's own ADS1115 driver on the same address while the cog runs.

**Reads, but no events (`quiet` forever).**
- Threshold too high or trimpot too low → see **Tuning**.
- Module on the wrong channel: default is A1; set `channel` to match your wiring.
- Analog-OUT module: lower `--threshold` to ~0.3–0.8 V.

**Events all the time (`present` never clears).**
- Threshold too low / trimpot too high, or the mic is near a fan or a buzzing supply → **Tuning**, **Placement**.

**Nothing in the app.**
- The cog must be running (export on `:8049`), and the app's Seed host must point at the Seed. Over USB that's `169.254.42.1`; on the LAN/tailnet use the Seed's IP.
- In a browser served from `localhost`, Chrome may block calls to a private IP (Private Network Access) — use the native app, or serve the page from the Seed.
