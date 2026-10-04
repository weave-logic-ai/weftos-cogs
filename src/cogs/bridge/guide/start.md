# Start here

> Bring sensing from other boards into the Seed's memory, without moving the agent.

The bridge cog lets a sensor on another board — a Pi 5, an Orange Pi, any node running WeftOS — contribute to this Seed's memory. The other board runs one of our cogs and reads its own sensor; a small sender (`weft-bridge-send`) forwards each reading here; the bridge writes it into the Seed store. COG-007.

The Seed stays the brain. The external board does the sensing. Nothing proprietary moves, and the external board is never treated as a Seed.

## How it flows

```diagram
flow
```

## The short path

1. **On the Seed:** install and start the bridge cog. → see **Set up the Seed**
2. **On the other board:** run one of our cogs (for example `sen0628-tof`) so it exports readings on its own port.
3. **On the other board:** run `weft-bridge-send`, pointing it at that cog's export and at this Seed's bridge.
4. Watch the bridge's `/status`: each source shows up with its own store id and a rising count.

## What you get

- Each `(node, cog)` source gets its own store-vector id, so two boards' streams stay separate in the Seed memory.
- The reading is an 8-number summary vector plus named metrics (heart rate, nearest distance, and so on). The full waveform or depth frame stays on the originating board's own export — the Seed store holds 8-dimensional points, so only the summary is written here.
- The data becomes part of the Seed's witnessed, searchable memory, the same as a cog running on the Seed itself.

Not a sensor itself, and not a medical device. → see **Security**
