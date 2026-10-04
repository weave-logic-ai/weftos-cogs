# Parts

> What to buy or gather, why each item is needed, and what trips people up.

You need one sensor, one ADC, some wire, and a way to power the Seed that is safe on a person.

## Parts table

| Item | Why | Gotcha |
|---|---|---|
| DFRobot SEN0213 with its 3-lead cable and gel pads | The AD8232 ECG front end. Output is 0-3.3 V, idling near mid-supply. | Silk labels on the 3-pin Gravity (PH2.0-3P) port are `A`, `+`, `-`. Cable colours vary, so follow the labels. The 3-lead cable plugs into the 3.5 mm jack. |
| ADS1115 breakout, 3.3 V compatible | The Seed has no analog input. The ADS1115 digitises at 16 bit over I2C. | Use the board at 3.3 V only. Breakouts pull SDA and SCL up to their own supply, and the Pi's GPIO is not 5 V tolerant. A generic board works; so does the DFRobot Gravity I2C ADS1115 (DFR0553). |
| Jumper wires (Dupont), or Gravity cables | Connect the ADS1115 to the Seed header and the sensor to the ADS1115. | Dupont wires wiggle loose, and a loose SDA or SCL wire looks like a missing chip. Add strain relief (hot glue or a zip tie), or use a screw-terminal GPIO HAT. |
| 4-wire I2C lead (3V3, GND, SDA, SCL) | Optional. Neater than four separate jumpers. | Match pins by label, not colour. |
| Fresh electrode pads (gel, snap type) | Skin contact decides signal quality. | Dry or reused pads are the most common cause of noise. Keep spares. |
| Battery or USB power bank | Powers the Seed while pads are on a person. | The AD8232 is not isolated. Don't use a mains-connected laptop or charger with pads on. → see **Safety** |
| A Seed (Raspberry Pi Zero 2 W) | Runs the cog. | The stock image has I2C off. → see **Set up the Seed** |
| A Mac, PC or phone browser | Runs `weft-ecg-scope` (native or in a browser tab). | It reaches the Seed over USB, LAN or tailnet. |

## Optional

- A multimeter. Measuring 3.3 V between Seed pins 1 and 6, and 3.3 V on the ADS1115 `VDD`, rules out a power fault in one minute.
- A screw-terminal GPIO HAT, if you want the wiring to survive being moved.
- Hot glue or zip ties for strain relief.

## Not needed

- No level shifter. Everything runs at 3.3 V.
- No `i2cdetect`. It is not installed on the Seed, and the cog's own error message works as the probe. → see **Troubleshoot**
- No prices are quoted here. The cog's design note estimates about $15 for the ADS1115.

## Before you start

1. Check the ADS1115 board is meant for 3.3 V use, or that it has a regulator you are not bypassing.
2. Check the electrode pads are in date and still wet with gel.
3. Decide how you will power the Seed during recording (battery or power bank).
4. Read **Safety** once.
