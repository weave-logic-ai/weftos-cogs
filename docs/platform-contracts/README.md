# Platform contracts

WeftOS still hosts a cog. This repository owns the cog product: guides, catalogs, sources, the host licence check, and Sensor Explorer. The WeftOS decisions below stay the OS record. Links go to tag `v0.8.3` (`fafd6168f8f97280ee24e5c2d2813b6e34918e15`). This directory does not copy those decisions.

The product successors are [COG-104](../decisions/COG-104-sensor-guides.md) through [COG-107](../decisions/COG-107-catalog-link.md). The import record is [PROVENANCE.md](../decisions/PROVENANCE.md).

| Contract | Tagged record | What this repository must do |
|---|---|---|
| Placement | [ADR-099](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-099-governed-workload-placement.md) | Treat a cog as a workload the OS places. Do not re-decide placement here. |
| Cog workload kind | [ADR-100](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-100-cog-workload-kind.md) | Keep the host contract in WeftOS. Product crates call the local daemon. They do not link the kernel. |
| Topology | [ADR-103](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-103-weave-topology-roles-and-instances.md) | Honour machine, user, project, and leaf roles when a cog is installed into a project. |
| Sources, OS portion | [ADR-105](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-105-cog-sources.md) | Leave project configuration, trust anchors, and the daemon command boundary in WeftOS. [COG-105](../decisions/COG-105-cog-sources.md) owns registry format, source resolution, and publish tooling. |
| Licence, OS portion | [ADR-106](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-106-seed-licence-proxy.md) | The daemon answers the licence protocol. [COG-106](../decisions/COG-106-host-licence.md) is only what Cog Host does with that answer. `cog-protocol` is the client and the golden vectors. The handler is not in this repository. |
| Catalog link | [ADR-107](https://github.com/weave-logic-ai/weftos/blob/v0.8.3/docs/adr/adr-107-catalog-hardware-software-link.md) | [COG-107](../decisions/COG-107-catalog-link.md) owns the module-to-cog link, Sensor Explorer, and the manager panels. |

Wire method names on the licence socket stay `cog.licence.import`, `cog.licence.status`, `cog.licence.claims`, and `cog.licence.revoked`. Renaming a crate does not rename those methods.
