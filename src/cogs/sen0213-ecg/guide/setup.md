# Set up the Seed

> Enable I2C, install the cog, start it, and open the companion app.

The Seed is a Raspberry Pi Zero 2 W with a 32-bit userland. These steps were checked on our Seed (cog0) on 2026-10-01.

## 1. Enable I2C (once per Seed)

I2C is off on the stock image. Back up `config.txt` first.

```bash
ssh genesis@<seed> 'sudo cp /boot/firmware/config.txt /boot/firmware/config.txt.pre-i2c
  echo "dtparam=i2c_arm=on" | sudo tee -a /boot/firmware/config.txt
  echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf
  sudo systemctl reboot'
ssh genesis@<seed> 'ls /dev/i2c-1'
```

`/dev/i2c-1` must exist after the reboot. A firmware OTA may undo this: check `/dev/i2c-1` after every update.

`i2cdetect` is not installed on the Seed. The cog's own error is the probe. → see **Troubleshoot**

## 2. Install the cog (sideload)

The cog is not in Cognitum's store. From the cogs repo:

```bash
scripts/cross-build.sh sen0213-ecg
scripts/seed-sideload.sh sen0213-ecg genesis@<seed>
```

This needs SSH key auth to the Seed. The script copies the binary and a `manifest.json` into `/var/lib/cognitum/apps/sen0213-ecg/`. The agent lists the cog at once, with no restart.

## 3. Test it with no hardware

The simulate mode makes a synthetic 72 bpm ECG. It checks the cog and the Seed, not your wiring.

```bash
curl -X POST http://<seed>/api/v1/apps/sen0213-ecg/console \
  -H 'Content-Type: application/json' -d '{"command":"--once --simulate"}'
```

Expect `status: ok` and about 72 bpm after roughly 10 s. `<seed>` is `169.254.42.1` over USB, or its LAN or tailnet address.

## 4. Start the cog

Continuous mode keeps the signal export up. Start it from the app (**Start cog**) or with `curl`:

```bash
curl -X POST http://<seed>/api/v1/apps/sen0213-ecg/start
curl http://<seed>:8046/status
```

The export starts before the ADC is found, so `/status` reads `no_source` while you wire. That is normal. A bearer token may be required on some Seeds; add `-H 'Authorization: Bearer <pairing token>'` if the agent refuses writes.

## 5. Run weft-ecg-scope

**Native:**

```bash
cargo build --locked --release -p cog-ecg-scope --bin weft-ecg-scope
SEED_HOST=169.254.42.1 target/release/weft-ecg-scope
```

**Browser:**

```bash
python3 -m http.server 8091 -d crates/cog-ecg-scope/www
```

`scripts/repo-gate.sh` builds the wasm library (`wasm32-unknown-unknown`, profile `release-wasm`) in a container. The page imports `www/pkg` once that package exists. The binary name is `weft-ecg-scope`. The crate is `crates/cog-ecg-scope`.

Open `http://127.0.0.1:8091/?seed=169.254.42.1`.

`SEED_HOST` and `?seed=` take the Seed's address. The app derives the agent API (`http://<seed>`, port 80) and the cog export (`http://<seed>:8046`). There is no SSH and no TLS. Port 8443 is a self-signed TLS endpoint that browsers reject, so use port 80.

`COGNITUM_SEED_TOKEN` sets the pairing token if the agent needs one.

## The checklist

The app checks ten steps in order. Later steps wait for earlier ones.

| # | Step | Passes when |
|---|---|---|
| 1 | Seed agent reachable | the agent API answers |
| 2 | Cog installed | `sen0213-ecg` is in `/api/v1/apps` |
| 3 | Cog running | the cog is started |
| 4 | Signal export reachable | `:8046` answers |
| 5 | ADS1115 found on I2C | the chip answers |
| 6 | Sensor powered | baseline near 1.65 V, not flat |
| 7 | Electrodes attached | the signal stays off the rails |
| 8 | Heartbeats detected | quality at least 0.8 |
| 9 | Lead polarity | R-waves point up |
| 10 | Mains notch | the notch matches the hum |

Steps 1-4 are about software; this page covers them. Steps 5-6 → see **Wiring**. Steps 7 and 9 → see **Electrodes**. Steps 8 and 10 → see **Calibrate**.
