# Wiring

> Three wires from the module to the ADS1115, four from the ADS1115 to the Seed. Trust the labels, not the colours.

Power everything from **3.3 V on pin 1**. Never use the 5 V pins (2 and 4): the ADS1115 and the module run at 3.3 V and the Pi's GPIO is not 5 V tolerant.

## Find pin 1

```diagram
header
```

Hold the Seed with the **40-pin header along the top edge** and the ports — micro-SD, HDMI, USB, power — **along the bottom**, micro-SD at the **left**. The header has two rows:

- The row **nearer the chip** (the inner row) carries the **odd** pins **1, 3, 5 … 39**, left to right. **Pin 1 (3.3 V) is at the left of this inner row.** Pins 1, 3, 5 = 3.3 V, SDA, SCL.
- The outer row (nearer the top edge) carries the **even** pins **2, 4 … 40**. Pin 2 (5 V) is its left pin.

Pins 5 and 6 share a column: **pin 6 (GND) is the outer-row pin directly across from pin 5 (SCL)**.

> **See your exact board.** Open the interactive, zoomable Pi Zero 2 W pinout: <https://pinout.xyz/>. The photo `pizero2w-pinout.jpg` next to this guide shows the labelled header on a real Pi Zero 2 W.

![Pi Zero 2 W pinout](pizero2w-pinout.jpg)

## The wiring diagram

```diagram
wiring
```

## Pin by pin

| From | To | Typical wire |
|---|---|---|
| Seed pin 1 (3V3) | ADS1115 `VDD` + module `VCC` | red |
| Seed pin 6 (GND) | ADS1115 `GND` + module `GND` | black |
| Seed pin 3 (SDA) | ADS1115 `SDA` | yellow |
| Seed pin 5 (SCL) | ADS1115 `SCL` | green |
| ADS1115 `ADDR` | ADS1115 `GND` | jumper; address 0x48 |
| Module `OUT` | ADS1115 `A1` | blue |

Colours are a convention only.

## Why A1

The `sen0213-ecg` cog uses A0. Putting the sound module on **A1** lets both sensors share one ADS1115. If you only have the sound sensor, A0 is fine too — set the cog's `channel` to 0.

## Address options

The ADS1115 `ADDR` pin picks the I2C address; the cog's `i2c_addr` setting is the decimal value.

| ADDR tied to | Address | `i2c_addr` |
|---|---|---|
| GND | 0x48 | 72 (default) |
| VDD | 0x49 | 73 |

**Final check.** Start the cog and open the app. The step **ADS1115 found on I2C** goes green when the chip answers.
