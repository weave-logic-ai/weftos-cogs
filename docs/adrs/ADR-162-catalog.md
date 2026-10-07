# ADR-162: catalog, serving the WeftOS hardware catalog from the Seed

**Status:** Proposed
**Date:** 2026-10-03
**Cog:** `catalog` 0.1.0

## Context

The WeftOS hardware catalog (Projects, Modules, Chips) describes every sensor, board and chip we have explored. It was a JSON file plus a generator script (`scripts/gen-catalog-viewer.py`, per the header of `src/cogs/catalog/src/main.rs`). We want the appliance to offer it on the mesh, so anyone who can reach the Seed can browse it without a desktop tool.

This cog is not a sensor. It has none of the usual sensor-cog parts: no bus, no parser, no `--simulate`, no store vector. It is an "app" cog in the ADR-001 cog-as-plugin sense (`src/cogs/catalog/src/main.rs` header), host-managed like the sensor cogs. The first commit is `9384f13` (2026-10-02).

## Decision

1. **`catalog` is a read-only HTTP server.** It serves three routes, all with `Access-Control-Allow-Origin: *` (`route()` and `handle()` in `src/cogs/catalog/src/main.rs`):
   - `GET /` and `GET /index.html`: the viewer page (`text/html`).
   - `GET /catalog.json`: the catalog (`application/json`).
   - `GET /healthz`: `{"ok":true}`.
   - Anything else is `404` with `{"error":"not found"}`. A query string is stripped before routing. The cog does not check the request method; any request line is routed by its path.
2. **Port and bind.** The default is `0.0.0.0:8060` (`[api] bind_port = 8060`, `bind_loopback_only = false` in `src/cogs/catalog/cog.toml`). `--api-bind <host:port>` sets the full address; `--port <n>` sets `0.0.0.0:<n>`; `--api-bind` wins if both are given. The listener is plain `std::net::TcpListener`, plain HTTP, no TLS.
3. **The catalog is an embedded snapshot.** `src/cogs/catalog/catalog.json` is pulled in with `include_str!` at build time, so the cog reads no file and makes no network call at run time. The viewer is a self-contained HTML page held in `main.rs`; its `__DATA__` placeholder is replaced with the same JSON each time `/` is served, so the page works with no second request.
4. **The snapshot is refreshed by hand, as a commit.** There is no refresh at run time. The history shows the pattern: the snapshot was copied from `weftos-cog-market/catalog` in `9384f13`, then refreshed in `a276acf`, `df10887`, `01f33c5` and `0895221` (255 to 556 items). That directory is now `crates/cog-market/catalog`. Each refresh needs a rebuild and a new cog binary. The file's own `generated` field (`2026-10-02`) and `schema` field (`1`) tell a reader which snapshot a running cog holds.
5. **Catalog shape.** The JSON is an object with `schema`, `generated`, `note`, `projects` (3), `modules` (245) and `chips` (308), 556 items in all. A unit test (`embedded_catalog_is_valid_json_with_sections`) checks that it parses and that the three sections are arrays. The `note` field says specs tagged `source` are vendor or datasheet values, not our bench data, and that `buy` entries come from a distributor pull.
6. **No sensor, no store writes.** The cog opens no device, calls no Seed API and writes nothing to the store. Its only dependencies are `cog-sensor-sources` (used for `handle_help` only) and `serde_json` (used in the test) (`src/cogs/catalog/Cargo.toml`).
7. **Flags.** `--help` is handled by `cog_sensor_sources::handle_help`, using the embedded `cog.toml`. `--interval` and `--once` are accepted by the host harness convention but ignored: the cog serves until killed. On a bind failure it prints `[cog-catalog] fatal: ...` to stderr and exits 1. After a successful bind the main thread parks forever and a single thread accepts connections.
8. **Manifest.** `id = "catalog"`, `name = "Hardware Catalog"`, binary `cog-catalog-arm`, `hardware_requirement = ["v0-appliance"]` (`cog.toml`). The release profile is size-optimized (`opt-level = "s"`, LTO, `panic = "abort"`, stripped).
9. **Console limits (decided, being added to `cog.toml` by a separate change).** `allowed_commands` stays `["--help"]`. `max_runtime_secs = 15` and `output_limit_bytes = 65536` are added, the same values the `bridge`, `hlk-as201` and `ld2450-radar` manifests carry. `--help` is the only command the console can run, so 15 s and 64 KiB are ample.
10. **Category.** `cog.toml` currently says `category = "app"`. It is being mapped to `developer`. The code does not depend on the value.

## Consequences

- The appliance can show the full hardware catalog on the mesh with no desktop tool, on any browser that reaches port 8060.
- The data goes stale until someone refreshes the snapshot and ships a new binary. Specs, prices and links in it are as old as the `generated` date.
- The binary is large for what it does: the snapshot is about 600 KB and is held in memory, and `/` builds a fresh copy of the page (viewer plus JSON) per request.
- The server is minimal. `handle()` reads one buffer of 2048 bytes in a single `read()`, sets no read timeout and handles connections one at a time on one thread, so a client that connects and sends nothing blocks every other request. Only request paths are used, so a request line cut off in that first read can be misrouted. Nothing here is a vulnerability for a read-only public catalog, but it is not hardened the way `bridge` is (ADR-160 Amendments 2 and 3).
- The export is reachable on the LAN and tailnet, by default, from any origin (CORS `*`). The content is a public parts list; it carries no user data. Set `api_bind` to `127.0.0.1:8060` to keep it on the Seed.
- Installed by sideload like the other cogs unless it is added to a registry. The catalog cog is not a sensor, so it has no ADR-104 sensor guide in this tree.
- `/healthz` reports only that the server is up, not which snapshot it holds. A reader has to fetch `/catalog.json` and look at `generated`.

## Spatial evidence: not applicable

This cog has no `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107), although the sensor-cog skill ships one by default for sensor cogs. It is not a sensor. It observes nothing, so it has no position, range or bearing to report and no ADR-107 record type fits.

## Alternatives

- **Serve the catalog from a desktop or cloud page only.** Needs a connection to a host outside the appliance. Rejected by the cog's purpose, which is to offer the catalog on the mesh.
- **Read `catalog.json` from disk at run time.** Would let the data be updated without a rebuild, but the cog would then depend on where the host puts a data file. The embedded snapshot needs only the binary. Whether a refreshable file is wanted is an open question below.
- **Make it part of the guide server of another cog.** Each cog serves its own guide; a catalog of all cogs' sensors does not belong to any one of them.

## Open questions

- Is `0.0.0.0` the right default bind? The decision above records what the code does, not that anyone chose it over loopback for a reason.
- Who owns the refresh, and on what trigger? The commits show it done by hand after the upstream `weftos-cog-market/catalog` changed. That tree is now `crates/cog-market/catalog`. There is no script in this tree that does it.
- Should the cog carry an ADR-104 guide, or is that only for sensor cogs? None is shipped.
- Should `/healthz` or a `/status` route report the snapshot's `generated` date and item counts?
- Should the server get a read timeout and a method check (`GET` and `HEAD` only) before it is exposed beyond a trusted network?
- Is the `category` mapping to `developer` final? It is a decision made outside this ADR; this ADR only records it.
