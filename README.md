# WeftOS Hardware Cogs

Open, signed, per-arch sensor/radar cogs for [Cognitum](https://cognitum.one) Seeds and WeftOS hosts.

A **cog** is a small Rust program that reads one piece of hardware (a radar, a ToF
imager, an ECG front-end, a mic, an IMU) and emits a JSON line per window plus an
8-float vector into the Seed's vector store. Each cog ships `cog.toml` (typed config),
`src/`, a rendered `guide/` (ADR-104), and tests.

## Cogs

| Cog | Sensor | Reads |
|---|---|---|
| `ld1040c-motion` | HLK-LD1040C | 10 GHz Doppler motion/occupancy |
| `ld2450-radar` | HLK-LD2450 | 24 GHz up-to-3-target position tracking |
| `ld6002-radar` | HLK-LD6002 | 60 GHz FMCW vitals (heart/breath) |
| `rd-03e` | Ai-Thinker RD-03E | 24 GHz presence + ranging |
| `hlk-as201` | HLK-AS201 | attitude / IMU |
| `sen0213-ecg` | DFRobot SEN0213 (AD8232) | single-lead ECG via ADS1115 |
| `sen0628-tof` | DFRobot SEN0628 (VL53L7CX) | 8×8 matrix time-of-flight |
| `sound-detect` | KY-038 mic | sound events via ADS1115 |
| `mentra-live` | Mentra Live glasses | companion presence/battery bridge |
| `bridge` | — | receives remote nodes' readings into the Seed store |
| `catalog` | — | the on-Seed hardware catalog app |

## Build

```sh
scripts/repo-gate.sh                # workspace, explorer, wasm, catalog, cog gate
scripts/cross-build.sh <cog-id>     # armv7 (Seed/Pi Zero 2 W) + aarch64 (Pi 5)
scripts/cross-build.sh              # all cogs
```
Artifacts land in `.cargo-target/dist/<id>/`: stripped `cog-<id>-arm`, `cog-<id>-arm64`, and `manifest.json` from `scripts/cog_manifest.py`.

## Install

Released cogs are signed (Ed25519, pinned WeaveLogic key) and published at the
Sensor Explorer, which serves a COG-008 `registry.json` and per-arch downloads.
Install on a WeftOS host via `cog-sources`, or on a Cognitum Seed via the
`cogrepo` cog — both verify the signature against the pinned key before trusting
the bytes. See COG-008 / COG-012 for the trust model.

## License

MIT — see [LICENSE](LICENSE).
