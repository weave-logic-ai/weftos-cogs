# Start here

> Make a pair of Mentra Live glasses show up as a live node on the mesh — presence, battery, and link health — before building anything on top.

The **Mentra Live** glasses run Android (the MentraOS ASG client). They are too small to host a WeftOS cog themselves, so this cog runs on a **companion** — your Mac, or the Seed appliance (cog0) — that can reach the glasses over **ADB**. Every few seconds it checks whether the glasses are reachable, reads their battery, measures the link round-trip, and posts a **fleet heartbeat** so they appear as an online node. The moment the link drops, the heartbeats stop and the node goes offline on its own — no faked liveness.

This is the floor to build on. Once the glasses are a node, the next cogs add the HUD display, the on-board sensors, and the audio path.

```diagram
flow
```

## The 5-minute path

1. **Reach the glasses over ADB** — on USB, or over Wi-Fi with `adb connect`. → see **Connect**
2. **Run the cog** pointed at the glasses and at your weft-cog-host.
3. **Confirm it joined** — the host's fleet roster lists `mentra-01` as online with a battery reading.
4. Watch `/status` track battery and link as you move around.

## Try it with no hardware

The cog has a `--simulate` mode that feeds synthetic glasses telemetry — a battery that drains and recharges, always "online" — so you can see the whole report + heartbeat path before touching a device:

```
cog-mentra-live --once --simulate
```

That prints one telemetry snapshot and exits. Drop `--once` for a continuous stream.
