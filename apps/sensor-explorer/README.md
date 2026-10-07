# WeftOS Sensor Explorer

A browsable, contributable hardware catalog (sensors → modules → chips) with an **MCP agent
surface**, on **Cloudflare Workers + D1**. Seeded from
`crates/cog-market/catalog/catalog.json`. That file is the canonical catalog. This app does
not keep a second committed copy. `scripts/publish-catalog-artifact.mjs` writes a generated
byte copy to `public/catalog.json` and the version plus SHA-256 to `public/catalog-release.json`.
Those two files are gitignored. `/api/catalog.json` is still the D1 export, and its bytes are
not the canonical file.

This is **phase 1, MCP-first**: the agent API (search / get / tree / suggest / add) + public read
REST + a minimal landing page. The rich tree-browse UI and the human contribution/approval flow are
next.

## Layout

- `src/index.ts` — Worker: `POST /mcp` (authed), public read REST, landing page.
- `src/mcp.ts` — MCP JSON-RPC tools: `catalog_release`, `search_sensors`, `get_part`, `list_tree`, `suggest_for_task`, `add_part`.
- `migrations/0001_init.sql` — D1 schema (`parts`, `api_keys`, `contributions`).
- `scripts/gen-seed.mjs` — turns `crates/cog-market/catalog/catalog.json` into `seed.sql` and records its version and SHA-256 in `catalog_release`.
- `scripts/publish-catalog-artifact.mjs` — copies that file to `public/catalog.json` and writes `public/catalog-release.json`. `npm run seed:gen` runs both.

## Deploy (you run these — I can't auth to your Cloudflare account)

```bash
npm install

# 1. Create the D1 database, then paste its database_id into wrangler.jsonc
wrangler d1 create sensor-explorer

# 2. Schema + seed
npm run db:migrate            # applies migrations/0001_init.sql (remote)
# catalog_release is migrations/0007_catalog_release.sql. Apply pending
# migrations before the seed, or the release INSERT fails:
#   npx wrangler d1 migrations apply sensor-explorer --remote
npm run seed:gen              # canonical catalog.json -> seed.sql
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

`/api/tree` · `/api/search?q=&type=&limit=` · `/api/parts/:id` · `/api/catalog.json` (D1 export)
· `/catalog.json` (raw canonical bytes) · `/catalog-release.json` (version and SHA-256 of those
bytes) · `/api/catalog/release` (the same identity from the seed row) · `/healthz`.

## Refresh the catalog

From this directory, regenerate the seed from the canonical file and load it:

```bash
npm run seed:gen
```

`npm run db:seed` applies `seed.sql` to the remote D1 database. `npm run db:seed:local` applies it locally. The remote database is not the canonical catalog.
