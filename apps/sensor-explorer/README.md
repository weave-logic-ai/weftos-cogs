# WeftOS Sensor Explorer

A browsable, contributable hardware catalog (sensors → modules → chips) with an **MCP agent
surface**, on **Cloudflare Workers + D1**. Seeded from the WeftOS `catalog.json` (139 modules, 96
chips, 3 projects, with datasheets, Mouser buy links, and datasheet-critical notes).

This is **phase 1, MCP-first**: the agent API (search / get / tree / suggest / add) + public read
REST + a minimal landing page. The rich tree-browse UI and the human contribution/approval flow are
next.

## Layout

- `src/index.ts` — Worker: `POST /mcp` (authed), public read REST, landing page.
- `src/mcp.ts` — MCP JSON-RPC tools: `search_sensors`, `get_part`, `list_tree`, `suggest_for_task`, `add_part`.
- `migrations/0001_init.sql` — D1 schema (`parts`, `api_keys`, `contributions`).
- `scripts/gen-seed.mjs` — turns `catalog.seed.json` into `seed.sql`.
- `catalog.seed.json` — a snapshot of the WeftOS catalog (re-copy to refresh).

## Deploy (you run these — I can't auth to your Cloudflare account)

```bash
npm install

# 1. Create the D1 database, then paste its database_id into wrangler.jsonc
wrangler d1 create sensor-explorer

# 2. Schema + seed
npm run db:migrate            # applies migrations/0001_init.sql (remote)
npm run seed:gen              # catalog.seed.json -> seed.sql
npm run db:seed               # loads seed.sql into D1 (remote)

# 3. A bootstrap API key for the MCP/write surface (any strong random string)
wrangler secret put BOOTSTRAP_API_KEY

# 4. Ship it
npm run deploy
```

Local dev: `npm run db:migrate:local && npm run seed:gen && npm run db:seed:local && npm run dev`.

## Agent surface (MCP)

Point a harness at `POST https://<your-worker>/mcp` with header
`Authorization: Bearer <BOOTSTRAP_API_KEY or a D1 api_keys key>`. Standard MCP JSON-RPC
(`initialize`, `tools/list`, `tools/call`). Tools:

| Tool | What it does |
|---|---|
| `search_sensors` | keyword search (type/kind filters) |
| `get_part` | full record by id (specs, datasheet, buy, notes, chips) |
| `list_tree` | taxonomy counts (type / module kind / vendor / category) |
| `suggest_for_task` | rank candidate parts for a described need |
| `add_part` | propose a part/update (contribute scope) → pending review |

Issue scoped keys by inserting into `api_keys` (`key`, `owner`, `scope` = read\|contribute\|admin).

## Read API (public)

`/api/tree` · `/api/search?q=&type=&limit=` · `/api/parts/:id` · `/api/catalog.json` (full export,
for the console/appliance to embed) · `/healthz`.

## Refresh the catalog

Re-copy the latest snapshot and reseed:

```bash
cp ~/weftos/crates/weftos-cog-market/catalog/catalog.json catalog.seed.json
npm run seed:gen && npm run db:seed
```
