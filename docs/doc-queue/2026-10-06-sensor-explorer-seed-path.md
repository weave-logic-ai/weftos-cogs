# Sensor Explorer refresh command points outside this repository

- **State:** observed
- **Observed:** 2026-10-06, Doc, during the public COG successor write
- **Rank:** 1 — a builder refreshing the catalog will copy from another tree, or from a home directory that is not this checkout
- **Documents:** `apps/sensor-explorer/README.md` (not under `docs/`). Not edited. This brief allowed path repairs only inside `docs/`.

## What is wrong

The refresh snippet is `cp ~/weftos/crates/weftos-cog-market/catalog/catalog.json catalog.seed.json`.

The catalog file is in this repository at `crates/weftos-cog-market/catalog/catalog.json` (present at `056029567eb8`). The `~/weftos/...` source is a cross-repository path.

## Evidence

- README line 67, read 2026-10-06.
- `test -e crates/weftos-cog-market/catalog/catalog.json` at `056029567eb8`: exists.

## Not done

No edit outside `docs/`. No new image or seed file was invented. The module and chip counts in that README were not recomputed and were not copied into COG-107.
