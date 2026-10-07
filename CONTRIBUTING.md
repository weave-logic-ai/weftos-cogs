# Contributing

Work on a branch. Do not commit to `main` or `master`.

## Rust

The toolchain is Rust 1.95, from `rust-toolchain.toml`, including clippy and rustfmt.

This repository builds with cargo. It does not use the WeftOS `scripts/build.sh`.

```sh
cargo test --locked --workspace
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Commit `Cargo.lock` with the change. Do not bump dependency versions as part of an unrelated edit.

Packages under `src/cogs/` are not workspace members. `scripts/gate.sh` checks those cogs. `scripts/repo-gate.sh` is the full repository gate: workspace, Sensor Explorer, wasm libraries, catalog generation, a private-registry round trip, and `scripts/gate.sh --fast`. Pass `--cross` to include the ARM cross-build.

## WASM

Wasm builds run in the `rust:1.95-bookworm` container. The shell inside that image is `bash -c`. `bash -lc` drops `rustup` off `PATH`.

The target is `wasm32-unknown-unknown` and the profile is `release-wasm`. The gate builds the libraries for `cog-manager`, `guide-view`, `cog-ecg-scope`, `cog-sound-scope`, and `cog-tof-scope`.

ARM cog binaries use `scripts/cross-build.sh` and the image `weavelogic-cogs-cross:1.97.1`. That image is not the workspace compiler. The workspace stays on Rust 1.95.

## Sensor Explorer

`apps/sensor-explorer` uses Wrangler 3.114.17. `package.json` allows `^3.90.0`. Do not upgrade Wrangler as a side effect of another change.

The `db:*` scripts that pass `--remote` change the live D1 database. Local checks use `--local`.

## Secrets

Do not commit `.dev.vars`, signing keys, licence seeds, or tailnet addresses. See [SECURITY.md](SECURITY.md).
