# Start here

> Track up to three moving people with an LD2450 radar on a Cognitum Seed, then place the radar and map its targets into the room.

This guide covers the Hi-Link HLK-LD2450, a 24 GHz radar that tracks up to three moving targets and reports where each one is, connected to a Cognitum Seed over its UART. The `ld2450-radar` cog reads the radar's frames, about ten a second, and prints one JSON line per window with the latest targets: x and y in metres, speed in metres per second, plus how many frames arrived and how many bytes failed to parse.

The radar reports positions only. Nothing in its output identifies a person, and the slot number (1 to 3) is not a stable identity: the radar can reassign it.

## The path

Freeing the Seed's UART needs a reboot and the device owner's approval. Everything else is a few commands.

1. **Get the parts.** The radar, four female jumper wires, and a rigid wall mount. → see **Parts**
2. **Free the Seed's UART.** Out of the box the header UART runs the Linux serial console. → see **Set up the Seed**
3. **Wire it.** Four wires: 5 V, ground, and the two UART lines crossed over. → see **Wiring**
4. **Install and start the cog.** Run `--once` first and check the line. → see **Set up the Seed**
5. **Place it.** Wall-mounted at 1.5 to 2 m, facing the area. Check which way x points. → see **Placement**
6. **Map it into the room.** Record the radar's position and heading, then use the formula on the placement page. → see **Placement**

You can test the software with no hardware at all: `--simulate` makes frames of a person walking in front of the radar and runs them through the same decoder as the real thing.

## How the data flows

```diagram
flow
```

## What you get

| Output | Where | Use |
|---|---|---|
| JSON line per window | stdout; the Seed keeps it in `GET /api/v1/apps/ld2450-radar/logs` | Latest targets, counts, link health |
| Target export | `http://127.0.0.1:8052` on the Seed, continuous mode only | The last 30 s of decoded targets |

→ see **API reference** for every field and endpoint.

## Honest limits

- It tracks moving people. The manual is about motion tracking, so a person sitting still may drop out. This has not been measured on our radar.
- Positions are in the radar's own frame. The cog does not know where the radar is mounted.
- The protocol has no checksum. The cog checks the fixed header, tail and length, and drops targets that cannot be real (behind the radar, or beyond 10 m), but a corrupted byte inside a frame can still slip through.
- **Not yet verified on hardware.** The decoder is tested against the frames printed in Hi-Link's documents and against the simulator. No LD2450 has been read on our Seed yet. Treat the wiring and UART steps as unproven until that test is done.

## Pages in this guide

Parts, Wiring, Set up the Seed, Placement, Troubleshoot, API reference, Glossary.
