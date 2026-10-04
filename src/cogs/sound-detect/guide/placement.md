# Placement

> Put the mic where it hears what you want and not what you don't.

- **Point the mic opening** at the space you care about (a doorway, a room, a machine). The electret is roughly omnidirectional but a little directional out the front.
- **Keep it off hard vibrating surfaces.** The mic also picks up structure-borne thumps; a bit of foam under the board rejects footsteps and bumps.
- **Away from fans and vents.** Constant airflow raises the background and forces a higher threshold, which then misses quiet sounds.
- **A metre or two is plenty** for room-level detection (claps, voices, doors). This is presence-of-sound, not a measurement mic — it won't give distance or level in dB.
- **Mains hum** (50/60 Hz) and buzzing supplies can sit just under the threshold. If `activity_pct` is high in a "quiet" room, move away from the source or raise the threshold.

For several rooms, run one module + cog per room (each on its own Seed or Fleet node) rather than one mic trying to cover the house.
