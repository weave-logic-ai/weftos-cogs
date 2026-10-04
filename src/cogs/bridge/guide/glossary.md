# Glossary

> The terms this guide uses.

| Term | Meaning |
|---|---|
| **Bridge** | this cog: it receives readings from other boards and writes them into the Seed store |
| **Source** | one `(node, cog)` pair sending readings, e.g. `pi5/sen0628-tof` |
| **Reading** | the JSON a sender posts: source, cog, a vector and optional metrics |
| **Vector** | the 8-number summary a cog produces; what goes into the Seed store |
| **Metrics** | named values (heart rate, nearest distance) kept in the bridge report, not the store |
| **Store id** | the per-source id a vector is written under, so sources don't collide |
| **Store** | the Seed's vector memory; 8 dimensions per vector, witnessed on the chain |
| **weft-bridge-send** | the WeftOS-side sender that polls a cog export and forwards to the bridge |
| **External node** | another board (Pi 5, Orange Pi) running WeftOS and our cogs |
| **Seed** | the Cognitum appliance; the brain that holds the memory |
| **Sideload** | installing a cog onto the Seed by copying its binary and manifest, since it isn't in the store |
| **COG-005 / COG-007** | the decisions that the Seed's fleets are read-only (no fake Seed) and that a bridge links external nodes in |
