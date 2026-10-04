# Set up the Seed

> Check I2C, install the cog, start it, and open the companion app.

The Seed is a Raspberry Pi Zero 2 W. The I2C step is already done on our Seed (cog0), because the ECG cog needed it.

## 1. I2C (once per Seed)

I2C must be on. It already is on cog0. For a different Seed, back up `config.txt` first and enable it:

```bash
ssh genesis@<seed> 'sudo cp /boot/firmware/config.txt /boot/firmware/config.txt.pre-i2c
  echo "dtparam=i2c_arm=on" | sudo tee -a /boot/firmware/config.txt
  echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf
  sudo systemctl reboot'
ssh genesis@<seed> 'ls /dev/i2c-1'
```

`/dev/i2c-1` must exist after the reboot. A firmware update may undo this: check `/dev/i2c-1` after every update.

`i2cdetect` is not installed on the Seed. The cog's own error is the probe. → see **Troubleshoot**

## 2. Install the cog (sideload)

The cog is not in Cognitum's store. From the cogs repo:

```bash
scripts/cross-build.sh sen0628-tof
scripts/seed-sideload.sh sen0628-tof genesis@<seed>
```

This needs SSH key auth to the Seed. The agent lists the cog at once, with no restart.

## 3. Test it with no hardware

The simulate mode makes a synthetic room with a person walking through it. It checks the cog and the Seed, not your wiring.

```bash
curl -X POST http://<seed>/api/v1/apps/sen0628-tof/console \
  -H 'Content-Type: application/json' -d '{"command":"--once --simulate"}'
```

Expect `status: ok` and a `source` of `simulator/8x8`. In a short `--once` run, `background_learned` may stay `false`, because the background needs `learn_seconds` of frames and the window is only a few seconds. That is normal. `<seed>` is `169.254.42.1` over USB, or its LAN or tailnet address.

## 4. Start the cog

Continuous mode keeps the frame export up. Start it from the app or with `curl`:

```bash
curl -X POST http://<seed>/api/v1/apps/sen0628-tof/start
curl http://<seed>:8047/status
```

The export starts before the sensor is found, so `/status` reads `no_source` while you wire. That is normal. With a real sensor, the first frames take about 5 s after start, while the cog sets the matrix mode. A bearer token may be required on some Seeds; add `-H 'Authorization: Bearer <pairing token>'` if the agent refuses writes.

Start it with the scene empty, so it learns a clean background. → see **Mounting**

## 5. Run weft-tof-scope

The app lives in the WeftOS repo (`crates/weftos-tof-scope`).

**Native:**

```bash
scripts/build.sh scope tof
SEED_HOST=169.254.42.1 target/release/weft-tof-scope
```

**Browser (WASM):**

```bash
scripts/build.sh scope-web tof
python3 -m http.server 8092 -d crates/weftos-tof-scope/www
```

Then open `http://127.0.0.1:8092/?seed=169.254.42.1`.

`SEED_HOST` and `?seed=` take the Seed's address. The app derives the agent API (`http://<seed>`, port 80) and the cog export (`http://<seed>:8047`). There is no SSH and no TLS. Port 8443 is a self-signed TLS endpoint that browsers reject, so use port 80.

`COGNITUM_SEED_TOKEN` sets the pairing token if the agent needs one.

## The checklist

The app checks ten steps in order. Later steps wait for earlier ones.

| # | Step | Passes when |
|---|---|---|
| 1 | Seed agent reachable | the agent API answers |
| 2 | Cog installed | `sen0628-tof` is in `/api/v1/apps` |
| 3 | Cog running | the cog runs in continuous mode |
| 4 | Signal export reachable | `:8047/status` answers |
| 5 | SEN0628 found on I2C | the report is not `no_source` |
| 6 | Frames arriving | at least 3 frames per second |
| 7 | Zones see a target | at least 70 % of zones valid (amber above 0 %) |
| 8 | Background learned | `background_learned` is true |
| 9 | Mounting level | Check level is within 5 % both ways (optional) |
| 10 | Zone noise | mean zone noise is 15 mm or less (amber above) |

Steps 1 to 4 are about software; this page covers them. Step 5 → see **Wiring**. Step 6 → see **Troubleshoot**. Steps 7 and 9 → see **Mounting**. Steps 8 and 10 → see **Calibrate**.
