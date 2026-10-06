# COG-107: The catalog links a module to the cog that drives it

- **Status:** Accepted
- **Decision date:** 2026-10-03 (Owner / platform, WeftOS ADR-107)
- **Recorded here:** 2026-10-06, as the public successor for the catalog, Sensor Explorer, and the manager panels. This file does not renumber or amend the WeftOS record.
- **Predecessor:** WeftOS ADR-107, *The hardware catalog links modules to cogs*
- **Audience:** someone browsing hardware, or looking at a module in the manager to see whether a cog exists and what to do next
- **Question:** how does a catalog entry for a sensor connect to the cog that drives it, without storing "installed" or "running" in the catalog itself?
- **Rests on:** WeftOS ADR-107; `Module` in `crates/weftos-cog-market/src/hw.rs`; `cog_view` in `crates/weftos-cog-manager/src/sensor_link.rs` and the panel comment in `crates/weftos-cog-manager/src/sensor_detail.rs`; Sensor Explorer in `apps/sensor-explorer/README.md` (read at `056029567eb8`)
- **Also:** [COG-104](COG-104-sensor-guides.md) (the guide that ships with the cog), [COG-105](COG-105-cog-sources.md) (where an install comes from)

## Context

The catalog described a module. The cog list described software. The sensor view described a running cog. Nothing on the module said which cog drives it, whether that cog was available, installed, or running, or where the hook-up guide was. A reader could not tell "no cog exists" from "a cog exists and this host does not have it."

## Decision

The hardware catalog is the one place that links a module to cog ids. Availability and run state are not stored there. Sensor Explorer publishes and browses the catalog. The manager panels join the link to what sources offer and what the host is doing.

1. **The link lives on the module.** Each catalog `Module` carries `cogs: [<cog id>]`, the ids of the cogs that drive it (the cog's own id). A supporting board lists every cog that needs it. The reverse lookup, which modules a cog needs, is computed, never stored. An empty list means no cog yet.
2. **Optional facts sit beside the link, and empty means unknown.**
   - `firmware`: `version`, `read_with`, `read_config_key`, `update`, `url`, `notes`. `read_with` says how a cog reads the version off the module. `update` only points at the vendor route. Nothing in this catalog updates module firmware.
   - `docs`: `{label, url}`, an `http(s)` link or a repo-relative reference shown as text.
   - `seen_in`: the source documents the entry was drawn from.
3. **Availability and state are not catalog fields.** They come from the cog sources the manager already loads ([COG-105](COG-105-cog-sources.md)) and from the host (installed, running, stopped, refused by the licence check). The join is `cog_view`.
4. **Sensor Explorer browses that catalog.** It is the public read and contribution surface: search, part, tree, suggest, and a proposed add. It serves the catalog, including `/api/catalog.json`, for a console to embed. It does not record whether a cog is installed or running, and it does not install one. Install state stays in the manager's join.
5. **The manager module card is the detail panel.** Expanding it shows Software, Setup, Firmware, Docs and, for a running cog, Stats.
   - Software: the cog, the sources offering it, install state, and Install / Start / Stop / Configure / Open guide.
   - Setup: catalog pins, bus enablement, power, and the cog's guide wiring. A guide bundled with the catalog is shown with no host and no running cog ([COG-104](COG-104-sensor-guides.md)). A running cog's own `/guide` is only a fallback.
   - Firmware: the facts above, and whether the cog can read the version.
   - Docs: datasheet, the guide, curated links, buy link, catalog source.
   - Stats: host supervision facts plus the cog's own status line.
   Configure opens the cog's guide at its config-keys page. The panel does not write cog config. With no host, the panel shows availability only and Install is disabled, with a reason.
6. **Cross-links both ways.** The sensor view and the cog list link to the hardware a cog needs and open that module's card. The catalog list badges a module `cog available`, `installed`, or `running`. Those badges are the join, not fields in the catalog file. Cogs that are not hardware-bound have no module and no panel.
7. **The panel can show the mesh, by asking the connected host.** It does not poll peers itself. Where a host answers, the panel shows per node the version, state, restarts, uptime, and last output. With no answer, the view is the connected host alone and says so. Nodes are picked by address, not by name, and the current host's token is not sent to another node.

## Consequences

- Adding a cog for a module is a catalog edit plus the cog. The manager picks the link up. Sensor Explorer picks up the module when its catalog snapshot is reseeded. Neither stores install state.
- The catalog can reject a malformed entry (a firmware `read_config_key` with no `read_with` is one such reject, in `HwCatalog::validate`). It cannot know that a cog id is published, because cogs live in registries. An id with no listing shows as not published.
- "No cog yet" is a normal catalog state, not an error in the manager.

## What stays in WeftOS

WeftOS ADR-107 remains the predecessor, including host routes the console calls and the decision not to consult the daemon placement layer from the cog host. Those routes are not restated here. The daemon's placement layer stays in WeftOS.
