// Shared search helpers for the Sensor Explorer — used by both the public REST (index.ts) and the
// MCP tool surface (mcp.ts) so findability behaves identically everywhere.
//
// Matching: the query is tokenized (split on non-alphanumeric, lowercased) and a row matches when
// its `search` text contains ALL tokens (AND). A single token like "QM33120W" still matches
// "QM33120WTR13" because each token is a LIKE substring. If the AND match finds nothing, we fall
// back to a raw substring LIKE over the whole query.
//
// Expanding: a catalog search that dead-ends (few/no hits) does NOT stop there — the same query is
// also run against the imported `pool` and the `cogs` registry, so results are never a dead end.

export interface Env {
  DB: D1Database;
  EXPLORER_NAME?: string;
  BOOTSTRAP_API_KEY?: string;
}

export function clampLimit(n: unknown, def: number, max: number): number {
  return Math.min(Math.max(1, Number(n) || def), max);
}

export function tokenize(q: string): string[] {
  return String(q || "")
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter((t) => t.length > 0);
}

// WHERE fragment matching ALL tokens against the `search` column. Empty tokens → match everything.
function tokensWhere(tokens: string[]): { sql: string; binds: string[] } {
  if (!tokens.length) return { sql: "1=1", binds: [] };
  return { sql: tokens.map(() => "search LIKE ?").join(" AND "), binds: tokens.map((t) => `%${t}%`) };
}

export interface CatalogOpts { type?: string; kind?: string; limit?: number }

const CAT_COLS = "id,type,name,vendor,kind,category,hash";

// Curated catalog search: AND-of-tokens, with a raw-substring fallback when that dead-ends.
export async function searchCatalog(DB: D1Database, q: string, opts: CatalogOpts = {}): Promise<any[]> {
  const limit = clampLimit(opts.limit, 30, 200);
  const tokens = tokenize(q);
  const run = async (where: string, whereBinds: any[]): Promise<any[]> => {
    let sql = `SELECT ${CAT_COLS} FROM parts WHERE status='published' AND (${where})`;
    const binds = [...whereBinds];
    if (opts.type && opts.type !== "any") { sql += " AND type=?"; binds.push(opts.type); }
    if (opts.kind) { sql += " AND kind=?"; binds.push(opts.kind); }
    sql += " ORDER BY type,name LIMIT ?"; binds.push(limit);
    return (await DB.prepare(sql).bind(...binds).all()).results as any[];
  };
  const tw = tokensWhere(tokens);
  let rows = await run(tw.sql, tw.binds);
  // substring fallback over the whole query when AND-of-tokens found nothing
  if (!rows.length && tokens.length) {
    rows = await run("search LIKE ?", [`%${q.toLowerCase().trim()}%`]);
  }
  return rows;
}

// Imported pool search (uncurated). Same tokenized AND matching.
export async function searchPool(DB: D1Database, q: string, category?: string, limit = 20): Promise<any[]> {
  const lim = clampLimit(limit, 20, 200);
  const tokens = tokenize(q);
  const run = async (where: string, whereBinds: any[]): Promise<any[]> => {
    let sql = `SELECT mpn,manufacturer,name,category,price,datasheet,hash FROM pool WHERE (${where})`;
    const binds = [...whereBinds];
    if (category) { sql += " AND category=?"; binds.push(category); }
    sql += " ORDER BY manufacturer,mpn LIMIT ?"; binds.push(lim);
    return (await DB.prepare(sql).bind(...binds).all()).results as any[];
  };
  const tw = tokensWhere(tokens);
  let rows = await run(tw.sql, tw.binds);
  if (!rows.length && tokens.length) rows = await run("search LIKE ?", [`%${q.toLowerCase().trim()}%`]);
  return rows.map((r) => ({ ...r, source: "pool" }));
}

// Cog registry search.
export async function searchCogs(DB: D1Database, q: string, limit = 20): Promise<any[]> {
  const lim = clampLimit(limit, 20, 100);
  const tokens = tokenize(q);
  const tw = tokensWhere(tokens);
  const sql = `SELECT sensor_name,sensor_id,id,name,category,version,maps_to,bind_port,hash FROM cogs WHERE (${tw.sql}) ORDER BY sensor_name IS NULL, sensor_name, name LIMIT ?`;
  const rows = (await DB.prepare(sql).bind(...tw.binds, lim).all()).results as any[];
  return rows.map((r) => ({ ...r, maps_to: safeArr(r.maps_to), source: "cog" }));
}

function safeArr(s: any): string[] {
  try { const a = JSON.parse(s); return Array.isArray(a) ? a : []; } catch { return []; }
}

// Cogs whose maps_to includes any of the given catalog part ids — so a matched sensor surfaces the
// cog that reads it even when the cog's own text does not contain the query.
export async function cogsMappedTo(DB: D1Database, partIds: string[]): Promise<any[]> {
  const ids = [...new Set(partIds)].filter(Boolean);
  if (!ids.length) return [];
  const where = ids.map(() => "maps_to LIKE ?").join(" OR ");
  const binds = ids.map((id) => `%"${id}"%`);
  const sql = `SELECT sensor_name,sensor_id,id,name,category,version,maps_to,bind_port,hash FROM cogs WHERE ${where} ORDER BY sensor_name IS NULL, sensor_name, name LIMIT 20`;
  const rows = (await DB.prepare(sql).bind(...binds).all()).results as any[];
  return rows
    .map((r) => ({ ...r, maps_to: safeArr(r.maps_to), source: "cog" }))
    .filter((c) => c.maps_to.some((m: string) => ids.includes(m)));
}

// The expanding search used by /api/search and the MCP search_sensors tool. Always returns catalog
// hits; when the catalog is thin (< EXPAND_THRESHOLD) it also runs pool; it always runs cogs so a
// cog name / sensor surfaces even when a catalog part exists too.
export const EXPAND_THRESHOLD = 5;

export interface ExpandedResult {
  count: number;
  results: any[];
  cogs: any[];
  pool: any[];
  expanded: boolean;
}

export async function expandedSearch(DB: D1Database, q: string, opts: CatalogOpts = {}): Promise<ExpandedResult> {
  const results = await searchCatalog(DB, q, opts);
  let cogs: any[] = [];
  let pool: any[] = [];
  let expanded = false;
  if (q.trim()) {
    // cogs that match the query by text, plus cogs mapped to any matched catalog part
    const [byText, byPart] = await Promise.all([
      searchCogs(DB, q),
      cogsMappedTo(DB, results.map((r) => r.id)),
    ]);
    const seen = new Set<string>();
    cogs = [...byText, ...byPart].filter((c) => (seen.has(c.id) ? false : seen.add(c.id)));
    if (results.length < EXPAND_THRESHOLD) {
      pool = await searchPool(DB, q, undefined, 20);
      expanded = true;
    }
  }
  return { count: results.length, results, cogs, pool, expanded };
}

// Stable item hash, Web Crypto (Worker side) — mirrors the Node gen-script formula.
export async function itemHash(type: string, id: string): Promise<string> {
  const buf = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`${type}:${id}`));
  const hex = [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
  return "wh_" + hex.slice(0, 16);
}
