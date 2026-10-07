# Sensor node — product portion

Phase A ownership, 2026-10-07. The cog unit, the bus requirement, and the native and wasm halves are the product portion of WeftOS `docs/architecture/sensor-node.md`. The WeftOS node, the mesh, and the ADR-100 container runtime stay on that page. The source page is post-tag and was not on v0.8.3. The full spec remains there until a later content split.

A hardware requirement is the bus the cog uses, not a board name. The native half and the wasm half ship in one package. The host opens the port. `weaver cog install` installs the native half.
