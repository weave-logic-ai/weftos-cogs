# Decision records

Two series. They are not the same numbering, and neither series is renumbered into the other.

**COG-NNN** (this directory) are the public successors for cog product decisions. **WeftOS ADR numbers** are the historical predecessors. A WeftOS ADR stays the record of the decision that was taken there, including any OS or runtime portion this repository does not carry. A COG file states the product portion and links back. It does not amend the predecessor.

| ID | Title | Status | Predecessor |
|---|---|---|---|
| [COG-104](COG-104-sensor-guides.md) | A sensor guide ships with the cog | Accepted | WeftOS ADR-104 |
| [COG-105](COG-105-cog-sources.md) | Cog sources: multi-repository catalog, project sources, install and catalog | Accepted (product). OS and runtime remain WeftOS ADR-105 | WeftOS ADR-105 |
| [COG-106](COG-106-host-licence.md) | Cog Host asks the local daemon before a licensed cog starts | Accepted (product host contract, 2026-10-06). Daemon, mesh service, and kernel gate remain WeftOS ADR-106 | WeftOS ADR-106 |
| [COG-107](COG-107-catalog-link.md) | Catalog, Sensor Explorer, and manager panels link a module to its cog | Accepted | WeftOS ADR-107 |

## Already in `docs/adrs` — not renumbered

Hardware and app cog decisions already live in [docs/adrs](../adrs) as ADR-157 through ADR-167. They stay there. They are not COG numbers and they are not reassigned.

ADR-166 is [sound-detect](../adrs/ADR-166-sound-detect.md). ADR-167 is [LD1040C](../adrs/ADR-167-ld1040c-motion.md). [ADR-166-ld1040c-motion.md](../adrs/ADR-166-ld1040c-motion.md) is a redirect to ADR-167, not a second ADR-166. Do not renumber either decision to make room for a COG file. COG-104 through COG-107 do not collide with them: they are a different series, and their numbers follow the WeftOS predecessors they succeed.
