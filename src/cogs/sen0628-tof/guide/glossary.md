# Glossary

> Plain-word definitions of the terms used in this guide.

| Term | Meaning |
|---|---|
| **ToF** | Time of flight. The sensor sends out infrared light and times how long it takes to come back, which gives a distance. |
| **VL53L7CX** | The ST ranging chip on the SEN0628. It measures an 8x8 grid of distances, 20 to 3500 mm. |
| **RP2040** | The microcontroller on the board. It runs the sensor and serves frames over I2C, UART or USB-C. |
| **Zone** | One cell of the grid. It reports one distance in mm. Zones grow with distance. |
| **FOV** | Field of view. Here 60 degrees horizontal, 60 vertical and 90 diagonal. |
| **8x8 and 4x4** | The matrix modes: 64 zones, or 16 wider zones. The cog sets the mode at start, which takes about 5 s. |
| **Crosstalk** | Light that bounces off a cover glass or nearby surface straight back to the sensor. It causes false near readings. |
| **Ambient light** | Light from the room or the sun. Infrared in it adds noise and reduces range. |
| **Background** | The per-zone median distance, learned from the first `learn_seconds` of frames. Presence is measured against it. |
| **Presence margin** | `presence_mm`. How much closer than the background a zone must be to count as occupied. |
| **Sector** | A third of the columns: left, middle or right. The report gives the nearest valid distance in each. |
| **Motion** | `motion_mm`. The mean absolute change between consecutive frames, over zones valid in both. |
| **Valid zone** | A zone reading from 20 mm to `max_range_mm`. A reading of 0 means no target. |
| **I2C** | A two-wire bus. The Seed talks to the sensor on bus 1. |
| **SDA / SCL** | The I2C data and clock wires. SDA is pin 3, SCL is pin 5. |
| **Gravity connector** | DFRobot's 4-pin PH2.0 connector. Pins are `+ - C D`. |
| **Cog** | A small app that runs on a Cognitum Seed. This one is `sen0628-tof`. |
| **Seed** | The Cognitum appliance. Ours is a Raspberry Pi Zero 2 W. |
| **Sideload** | Installing a cog by copying it to the Seed yourself, instead of from the store. |
