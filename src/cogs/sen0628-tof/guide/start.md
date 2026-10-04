# Start here

> Read an 8x8 depth map on a Cognitum Seed, then place it, check it and tune it in the weft-tof-scope app.

This guide covers the DFRobot SEN0628, an 8x8 matrix time-of-flight sensor (a VL53L7CX ranging chip with an RP2040 board around it), connected to a Cognitum Seed over I2C. The `sen0628-tof` cog reads a depth frame several times a second and reports the nearest object and where it is, how much of the view has a valid reading, motion, presence, and left, middle and right distances.

The sensor measures distance in millimetres for each of 64 zones (or 16 in 4x4 mode) across a 60 x 60 degree view. It works from 20 mm to 3500 mm. It needs no analog parts: the Seed talks to it directly on I2C bus 1, so the wiring is four wires.

## The 5-minute path

Wiring and placement take the longest. Everything else is a few commands.

1. **Get the parts.** The sensor, a Gravity 4-pin I2C cable (or four jumpers) and a rigid mount. → see **Parts**
2. **Wire it.** Four wires to the Seed header. Trust the labels on the board, not the wire colours. → see **Wiring**
3. **Check I2C on the Seed.** It is already on for our Seed (cog0), because the ECG cog needed it. → see **Set up the Seed**
4. **Install and start the cog.** Sideload it, then start it from the app or with `curl`. → see **Set up the Seed**
5. **Open the companion app** `weft-tof-scope` and follow its checklist, top to bottom. Each failing step names the fix.
6. **Place it.** Mount it rigidly with a clear view, then learn the background with the scene empty. → see **Mounting**
7. **Tune it.** Check the valid share and noise, then set the presence margin. → see **Calibrate**

You can test the software with no hardware at all: the cog has a `--simulate` mode that makes a synthetic room with a person walking through it.

## How the data flows

```diagram
flow
```

The cog sets the matrix mode at start (this takes about 5 s), then reads one frame at the `rate_hz` setting. It learns a background, compares each frame against it, and builds the report from the last few seconds of frames.

## What you get

| Output | Where | Use |
|---|---|---|
| JSON report, one per interval | stdout; the Seed keeps it in `GET /api/v1/apps/sen0628-tof/logs` | Nearest object, valid share, motion, presence, sectors, noise |
| 8-float vector, id 22 | The Seed's store | Compact history of the report |
| Depth frame export | `http://<seed>:8047` (`/status`, `/frame`, `/frames`, `/raw.csv`) | The last 30 s of raw frames; what the app reads |

→ see **API reference** for every field and endpoint.

## Honest limits

- It is a coarse depth camera, not a safety device. It sees distance, not what the object is. → see **Safety**
- Presence compares each zone against a background learned at start. If people were in view while it learned, presence will be wrong until you restart the cog.
- Glass, direct sunlight and very dark or very shiny surfaces cause missing or wrong readings. → see **Mounting**
- Simulate mode has been verified. **Not yet verified:** a real SEN0628 on the header of our Seed (cog0). Treat the wiring and I2C details in this guide as unproven until that test is done.

## Pages in this guide

Parts, Wiring, Mounting, Set up the Seed, Calibrate, Troubleshoot, API reference, Safety, Glossary.
