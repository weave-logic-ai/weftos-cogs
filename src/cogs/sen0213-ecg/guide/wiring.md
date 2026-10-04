# Wiring

> Eight connections between the Seed header, the ADS1115 and the SEN0213. Trust the pin labels, not the wire colours.

Power everything from **3.3 V on pin 1**. Never use the 5 V pins (2 and 4): the ADS1115 pulls SDA and SCL up to its own supply, and the Pi's GPIO is not 5 V tolerant.

## Find pin 1

```diagram
header
```

Hold the Seed with the header along the top edge and the SD card on the left. Pin 1 is the top-left pin of the inner row. Pins 1, 3 and 5 run along that row. Pin 6 is in the outer row, opposite pin 5.

## The wiring diagram

```diagram
wiring
```

## Pin by pin

| From | To | Typical wire |
|---|---|---|
| Seed pin 1 (3V3) | ADS1115 `VDD` | red |
| Seed pin 6 (GND) | ADS1115 `GND` | black |
| Seed pin 3 (GPIO2, SDA) | ADS1115 `SDA` | yellow |
| Seed pin 5 (GPIO3, SCL) | ADS1115 `SCL` | green |
| ADS1115 `ADDR` | ADS1115 `GND` | jumper; sets address 0x48 |
| SEN0213 `A` (signal) | ADS1115 `A0` | blue |
| SEN0213 `+` | ADS1115 `VDD` (3.3 V) | red |
| SEN0213 `-` | ADS1115 `GND` | black |

Colours are a convention only. Gravity cable colours vary between batches.

## Using the DFRobot Gravity ADS1115 (DFR0553)

- The Gravity I2C lead has four labelled pins: `+ - C D`. Connect `+` to pin 1, `-` to pin 6, `C` (clock) to pin 5 and `D` (data) to pin 3.
- Plug the SEN0213's Gravity cable straight into analog port `A0`. That port also powers the sensor at 3.3 V.
- Leave the board's address switch on 0x48.

## Address options

The ADS1115 `ADDR` pin picks the I2C address. The cog's `i2c_addr` setting is the decimal value.

| ADDR tied to | Address | `i2c_addr` |
|---|---|---|
| GND | 0x48 | 72 (default) |
| VDD | 0x49 | 73 |
| SDA | 0x4A | 74 |
| SCL | 0x4B | 75 |

## Wire it, step by step

1. **Power off** the Seed and unplug the sensor from the electrodes.
2. Connect pin 1 to `VDD` and pin 6 to `GND`. Check them against the diagram, not the colours.
3. Connect pin 3 to `SDA` and pin 5 to `SCL`. Don't swap them.
4. Tie `ADDR` to `GND`.
5. Connect the SEN0213: `A` to `A0`, `+` to `VDD`, `-` to `GND`.
6. Re-read every wire against the table, then power on and give the strain relief a tug.

**Final check.** Start the cog and open the app. The step **ADS1115 found on I2C** goes green when the chip answers. Then **Sensor powered** turns green once the baseline sits near 1.65 V.

## Gotchas

- I2C must be enabled on the Seed first, or nothing will answer. → see **Set up the Seed**
- A wrong address looks the same as a missing chip. If `ADDR` is on `VDD`, the cog must be set to 73.
- Don't enable the Seed agent's own ADS1115 driver on this address while the cog runs. → see **Troubleshoot**
- **Not yet verified on hardware:** this pin map has been checked against the board labels and the cog's code, but we have not yet run a real ADS1115 and SEN0213 on our Seed.
