# Start here

> An attitude sensor that reports orientation, acceleration, angular rate and magnetic heading over a serial wire.

The **Hi-Link HLK-AS201** is a 9/10-axis attitude sensor — a 3-axis accelerometer, a 3-axis gyroscope and a 3-axis magnetometer (some variants add a barometer). It reports **orientation** (roll / pitch / yaw), **acceleration** (g), **angular rate** (deg/s) and **magnetic heading** over UART. The `hlk-as201` cog decodes that stream and tells you the device's attitude and whether it is moving or still.

## It is an IMU, not a radar

The **X / Y / Z silkscreen** printed on the board is the giveaway: this is an **inertial measurement unit (IMU)**, not a radar. It measures its own motion and orientation — it does not detect objects, range or presence. If you want presence or distance, you want a different cog.

## Protocol (confirmed)

> This cog decodes the **Hi-Link AS201 serial protocol, confirmed against the official datasheet
> (HLK-AS201 V1.1, 2025-08-01).** Frames are `FA FB · len · cmd · data · SUM · FC FD`; the sensor
> report (`cmd 0x00`) carries a 42-byte ten-axis payload — accel, gyro, Euler angles, magnetic field,
> quaternion, temperature, pressure and height — with the datasheet's scale factors. Not a guess.

## Try it without hardware

Run it in simulation to see the shape of the data before you wire anything:

```
cog-hlk-as201 --once --simulate
```

## The short path

1. **Wire it.** Four wires: power, ground, and the sensor's TX into the Pi's RX. → see **Wiring**
2. **Enable the Pi UART and install the cog.** → see **Set up the Seed**
3. **Confirm `/status`** shows a live attitude instead of `no_source`.
4. **Calibrate** — a flat-and-still accel zero-bias, and a figure-8 for magnetic heading. → see **Troubleshoot**
