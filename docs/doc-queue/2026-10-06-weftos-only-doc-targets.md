# Cog docs still link WeftOS paths this checkout does not contain

- **State:** observed
- **Observed:** 2026-10-06, Doc, during the public COG successor write
- **Rank:** 2 — the links sit in docs a builder opens to run a lane or follow a decision, and the targets are absent. Not rank 1: the public COG successors now exist for guides, sources, licence, and catalog, so those four are no longer only a broken `docs/adr/` link.
- **Not done this run:** no bulk rewrite. The brief limited repairs to withheld `docs/cogs/img` PNGs, absolute `/Users` paths, the private cogs repo, and kernel source paths.

## What is wrong

Checked at `056029567eb8` before the doc edits: `scripts/build.sh`, `scripts/cogs`, `scripts/pi`, `docs/adr`, and `docs/research` were absent. `docs/adrs` exists.

Pages that still point at those absent paths (kernel source paths and the private-repo parenthetical were repaired separately; these were not):

- `docs/cogs/test-pi.md` — `scripts/build.sh test-pi`, `scripts/pi/`, `scripts/cogs/`, links to `docs/adr/adr-099` and `adr-100`
- `docs/cogs/conformance-harness.md` — `scripts/build.sh`, `scripts/cogs/`, the same ADR links
- `docs/cogs/operator-guide.md` — the same ADR links, and `docs/research/mesh-placement/swarm-throughput.md`
- `docs/cogs/weft-licence.md` — `scripts/build.sh licence-cross` and `licence-uid-check` (the cross image itself is built by this repo's `scripts/cross-build.sh`; that one parenthetical was corrected)
- `docs/cogs/cog-sources.md` — still cites `scripts/cogs/expectations.json` and the unrecomputed "93 clean, 5 need `--interval`, 9 need seed peers" line. The design-record link was pointed at COG-105.
- `docs/guides/fleet-manager.md` — `docs/research/fleet-manager/network-tab-brief.md`
- `docs/cogs/ingest-bridge.md` — still cites `scripts/cogs/harness.py`

## Evidence

Method: `test -e` on the five paths above at `056029567eb8` (all absent), then search of `docs/**/*.md` for `../adr/`, `../research/`, `scripts/build.sh`, `scripts/cogs/`, and `scripts/pi/`.

## Do not restore

Do not point these pages back at a private repository path in order to make the links resolve.
