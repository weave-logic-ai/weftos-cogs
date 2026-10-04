# Parts

> What you need: the sensor, a four-wire cable, a rigid mount and a Seed with I2C on.

## Shopping list

| Part | Notes | Needed |
|---|---|---|
| DFRobot SEN0628 | 8x8 matrix ToF, Gravity PH2.0-4P connector | Yes |
| Gravity 4-pin I2C cable, or 4 jumpers | Female jumpers to the Seed header pins | Yes |
| Rigid mount | Screws, a bracket or a printed holder; the sensor must not move | Yes |
| Cognitum Seed | Raspberry Pi Zero 2 W, I2C bus 1 on the header | Yes |

## About the sensor

- **Chip:** VL53L7CX time-of-flight sensor, with an RP2040 on the board that serves frames over I2C, UART or USB-C.
- **Power:** 3.3 to 5 V, under 80 mA. We power it from the Seed's 3.3 V.
- **Connector:** Gravity PH2.0-4P. The cog uses I2C. UART is fixed at 115200 and USB output supports 8x8 only. The cog uses neither.
- **Addresses:** 0x30, 0x31, 0x32 or 0x33, chosen on the board. The factory default is 0x33.
- **Range and view:** 20 to 3500 mm; 60 degrees horizontal, 60 vertical, 90 diagonal.
- **Rate:** the chip ranges at 15 to 60 Hz. The cog reads up to 15 frames a second.
- **Accuracy:** about 11 mm at 20 to 200 mm on a white target, and 5 % at 200 to 3500 mm on white. Grey targets are slightly worse.

## About the Seed

- I2C must be on. It already is on our Seed (cog0), because the ECG cog needed it. A firmware update may undo it. → see **Set up the Seed**
- Bus 1 is shared with the ECG cog's ADS1115 (address 0x48). Different addresses coexist fine.
- Up to four SEN0628 boards can share the bus at different addresses. Each one needs its own cog instance and its own config.

## Nice to have

- A tape measure and a flat wall, for the levelling check. → see **Mounting**
- A second person to walk through the view while you tune presence.
- Strain relief on the cable, so a tug does not move the sensor.

→ see **Wiring**
