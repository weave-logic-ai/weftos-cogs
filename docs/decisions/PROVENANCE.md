# Provenance

Public product decisions in this repository were imported from [weave-logic-ai/weftos](https://github.com/weave-logic-ai/weftos) tag [v0.8.3](https://github.com/weave-logic-ai/weftos/tree/v0.8.3).

| Fact | Value |
|---|---|
| Source repository | `weave-logic-ai/weftos` |
| Tag | `v0.8.3` |
| Tagged commit | `fafd6168f8f97280ee24e5c2d2813b6e34918e15` |
| Annotated tag object | `b173b3abd39ac349c2becdc70862526e2a7d3ebe` |
| Import commit in this repository | `056029567eb8c0eea6e37f7b6958a370b3ff2385` |
| Import date | 2026-10-06 |
| Protocol client commit | `f6fd69d14cde919b279071c49b4014af2a685a29` |

Accepted WeftOS ADRs stay in WeftOS. This repository does not keep a second editable copy. The obligations this tree implements are indexed in [platform contracts](../platform-contracts/README.md).

## Ownership

| Decision | Canonical owner | Predecessor | Source path at import |
|---|---|---|---|
| [COG-104](COG-104-sensor-guides.md) | this repository | [ADR-104](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-104-sensor-guides-ship-with-sensor-cogs.md) | `crates/weftos-sensor-guide`, `crates/weftos-cog-companion` |
| [COG-105](COG-105-cog-sources.md) | this repository for the product portion. Project configuration, trust enforcement, and the daemon command boundary stay WeftOS ADR-105 | [ADR-105](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-105-cog-sources.md) | `crates/weftos-cog-sources`, `crates/weftos-cog-repo` |
| [COG-106](COG-106-host-licence.md) | this repository for the Cog Host contract. The daemon, mesh service, and kernel gate stay WeftOS ADR-106 | [ADR-106](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-106-seed-licence-proxy.md) | `crates/weftos-cog-host`. `crates/cog-protocol` arrived in `f6fd69d14cde919b279071c49b4014af2a685a29`, not in the import commit |
| [COG-107](COG-107-catalog-link.md) | this repository | [ADR-107](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-107-catalog-hardware-software-link.md) | `crates/weftos-cog-market`, `crates/weftos-cog-manager` |

Those crates now live under the names in the COG files (`crates/sensor-guide`, `crates/cog-*`). The import paths above are the names at `056029567eb8`.

## Stays in WeftOS

| Decision | Why it stays |
|---|---|
| [ADR-099](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-099-governed-workload-placement.md) | Governed placement is an OS plane. A cog is one workload kind. |
| [ADR-100](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-100-cog-workload-kind.md) | How WeftOS hosts a cog. |
| [ADR-103](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-103-weave-topology-roles-and-instances.md) | Machine, user, project, and leaf roles. |
| ADR-105, OS portion | Project and user configuration, trust enforcement, daemon command boundary. |
| ADR-106, OS portion | Mesh identity, licence facts, and the run-gate protocol the daemon answers. |

Per-cog hardware decisions ADR-157 through ADR-167 already lived in `docs/adrs` of this repository. They were not imported as COG numbers. ADR-166 is sound-detect. ADR-167 is the HLK-LD1040C motion cog. `docs/adrs/ADR-166-ld1040c-motion.md` is a redirect, not a second ADR-166.

## Not yet closed

Two tagged releases, the production migration, a rollback rehearsal, the 30-day soak, and the Phase A proof report are later gates. Phase B, removing the WeftOS copies, is unopened.
