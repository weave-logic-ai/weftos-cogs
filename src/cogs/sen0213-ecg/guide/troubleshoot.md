# Troubleshoot

> Symptom, likely cause, fix. Start with the first failing step in the app's checklist.

## Reading the status

| `status` | Meaning |
|---|---|
| `no_source` | The cog can't talk to the ADC. The report has an `error`. |
| `flat` | Signal std is under 2 mV: sensor unpowered or loose. |
| `leads_off` | More than 5% of samples are near 0 V or 3.3 V. |
| `no_beats` | Leads fine, but fewer than 2 RR intervals so far. The first 2 s are a learning period. |
| `ok` | Beats found; heart rate is reported. |

## Symptoms

| Symptom | Likely cause | Fix |
|---|---|---|
| `no_source`, error "i2c write reg 0x01 at 0x48: Input/output error" | Nothing answered at 0x48 | Check pin 1 to `VDD`, pin 6 to `GND`, pin 3 to `SDA`, pin 5 to `SCL`, and `ADDR` to `GND`. Wiggle each wire. |
| `no_source`, error contains "No such file" | I2C is off, so `/dev/i2c-1` is missing | Enable I2C and reboot. → see **Set up the Seed** |
| `no_source`, error "no ADS1115 at 0x48 ... config read back" | Something answers at that address, but it isn't an ADS1115 (or the address is wrong) | Check the address and that the device is an ADS1115. |
| `no_source` after changing `ADDR` | The cog is still looking at 0x48 | Set `i2c_addr` to match: 73 for `VDD`, 74 for `SDA`, 75 for `SCL`. |
| `flat` | Sensor not powered, or `A` not on `A0` | `+` to 3.3 V, `-` to `GND`, `A` to `A0`. Baseline should be about 1.65 V. |
| `leads_off` | Pad lifted or dry, or a lead unplugged | Press the pads, use fresh gel, check the 3.5 mm jack. |
| R-waves point down | RA and LA swapped | Swap the two pads. → see **Electrodes** |
| Strong 50 or 60 Hz hum | Mains pickup; wrong notch | Apply the recommended notch, twist the leads, check `RL` contact, move away from chargers and mains cables. |
| Wandering baseline | Breathing, movement, poor contact | Sit still, resting arms, fresh pads. The display filter removes most slow drift (0.5 Hz high-pass). |
| Fuzzy, spiky noise (EMG) | Muscle tension | Relax, rest your arms, don't grip or talk. Try the chest placement. |
| Heart rate about double | T-waves counted as beats | Try lead II for a taller R-wave. Check polarity. Compare with a tapped pulse. |
| `late_samples` keeps climbing | The Seed is too busy, or the rate is too high | Lower `sample_rate` toward 250. Stop other heavy apps. Check `max_late_ms`. |
| `read_errors` climbing | Loose I2C wire | Reseat SDA and SCL; add strain relief. |
| Export unreachable on `:8046` | Cog not running, or `api_bind` is loopback | Start the cog. Check `api_bind` is `0.0.0.0:8046`, not `127.0.0.1:8046`. |
| Config changes seem "lost" | `PUT` replaced the whole config | Always send every key. → see **API reference** |
| `no_source` after a firmware update | The OTA reset `config.txt` | Check `/dev/i2c-1`; re-apply the I2C steps. |
| Odd or unstable readings from the ADC | The agent's own ADS1115 driver is also enabled | Disable the agent's `i2c_sensors` entry for 0x48. |
| Browser can't reach the Seed on port 8443 | Self-signed TLS | Use plain HTTP on port 80. |

## Check I2C without i2cdetect

`i2cdetect` is not on the Seed. Instead:

1. Confirm `ls /dev/i2c-1` works over SSH.
2. Start the cog and read `curl http://<seed>:8046/status`.
3. "Input/output error" means no device answered. "No such file" means I2C is off. Anything else means the chip responded.

## Fighting the agent's driver

The Seed agent has its own ADS1115 driver (`sensor-config.json` `i2c_sensors`, a 10 Hz pipeline). It is too slow for ECG. Both drivers write the chip's config register, so **never enable it on 0x48 while this cog runs**. Move one of them to another address if you need both.

## Quick isolation order

1. Run `--once --simulate`. If it fails, the problem is the cog or the Seed, not your wiring.
2. Run `--once` with no pads on. Expect `leads_off` or `flat`, **not** `no_source`.
3. Put the pads on and look for `ok` and a believable heart rate.

## Last resorts

- Power off, and re-seat every wire from the diagram.
- Measure 3.3 V between pins 1 and 6, and again at the ADS1115 `VDD`.
- Try another set of pads and another placement.
