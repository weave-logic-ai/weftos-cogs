# Wiring

> Four connections between the Seed header and the SEN0628. Trust the labels on the board, not the wire colours.

Power the sensor from **3.3 V on pin 1**. Avoid the 5 V pins (2 and 4). The sensor accepts 5 V, but its I2C pull-ups then follow that supply, and the Pi's GPIO is not 5 V tolerant.

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
| Seed pin 1 (3V3) | SEN0628 `+` (VCC) | red |
| Seed pin 6 (GND) | SEN0628 `-` (GND) | black |
| Seed pin 3 (GPIO2, SDA) | SEN0628 `D` (SDA) | green |
| Seed pin 5 (GPIO3, SCL) | SEN0628 `C` (SCL) | blue |

Colours are a convention only. Gravity cable colours vary between batches.

**Trust the silkscreen.** The Gravity I2C pins are `+ - C D`: `C` is the clock (SCL) and `D` is the data (SDA). Some Gravity boards that switch between I2C and UART label the same pins `D/T` and `C/R`. Follow the pin's function on the board, not the first letter.

## Choose I2C, not UART

The board has a switch that selects I2C or UART output, and a second one that selects the I2C address. The cog needs I2C.

- Look at the silkscreen next to the switches for the positions. **Unverified:** the exact switch positions were not in the documentation we could read, so check the board's own labels.
- UART is fixed at 115200 and the cog does not use it.

## Address options

The cog's `i2c_addr` setting is the decimal value.

| Address | `i2c_addr` | Note |
|---|---|---|
| 0x30 | 48 | |
| 0x31 | 49 | |
| 0x32 | 50 | |
| 0x33 | 51 | factory default, cog default |

**After changing the I2C address or the I2C/UART setting, power-cycle the board.** The new setting is not read until power is removed and restored. A change without a power-cycle is the most common cause of "nothing answered".

## Sharing the bus

The ECG cog's ADS1115 sits on the same bus at 0x48, which does not clash with 0x30 to 0x33. You can wire both on pins 1, 3, 5 and 6. Two SEN0628 boards need different addresses and one cog instance each.

## Wire it, step by step

1. **Power off** the Seed.
2. Connect pin 1 to `+` and pin 6 to `-`. Check them against the diagram, not the colours.
3. Connect pin 3 to `D` and pin 5 to `C`. Don't swap them.
4. Check the I2C/UART and address switches, and power-cycle the board if you changed either.
5. Re-read every wire against the table, then power on and give the cable a tug.

**Final check.** Start the cog and open the app. The step **Sensor found** goes green when the board answers. The cog's own error, `nothing answered at 0x33`, tells you when it does not. → see **Troubleshoot**

## Gotchas

- I2C must be enabled on the Seed first, or nothing will answer. → see **Set up the Seed**
- A wrong address looks the same as a missing board. If you set the board to 0x31, the cog must be set to 49.
- Mode changes take about 5 s. The cog sets the mode at every start.
- **Not yet verified on hardware:** this pin map matches the board's published pins and the cog's code, but we have not yet run a real SEN0628 on our Seed.
