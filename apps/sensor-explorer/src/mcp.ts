// MCP agent surface for the Sensor Explorer — stateless JSON-RPC 2.0 over HTTP (Streamable HTTP,
// single-response mode). A harness connects here as an MCP tool server and can search, read, browse
// the tree, get a suggestion for a task, and (with contribute scope) propose a new part.

import { type Env, expandedSearch, searchPool, searchCogs } from "./search";
export type { Env };

const VERSION = "0.2.0";

type Scope = "read" | "contribute" | "admin";

const TOOLS = [
  {
    name: "search_sensors",
    description: "Search the hardware catalog (sensors, modules, chips, projects) by keyword. Query is tokenized and ALL tokens must match (with a substring fallback), so 'QM33120W' matches 'QM33120WTR13'. When the curated catalog is thin, the result also includes matching imported-pool parts (source:'pool') and cog registry hits (source:'cog') so a search never dead-ends.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "keywords — name, vendor, what it senses, interface, spec" },
        type: { type: "string", enum: ["module", "chip", "project", "any"], description: "restrict to a type (default any)" },
        kind: { type: "string", enum: ["board", "sensor", "display", "actuator"], description: "for modules: restrict by kind" },
        limit: { type: "number", description: "max results (default 20, max 100)" },
      },
      required: ["query"],
    },
  },
  {
    name: "get_part",
    description: "Get the full record for one part by id (module, chip or project) — specs, datasheet, buy link, notes, chips-on-board, projects using it.",
    inputSchema: { type: "object", properties: { id: { type: "string" } }, required: ["id"] },
  },
  {
    name: "list_tree",
    description: "Browse the catalog taxonomy: counts by type, by module kind, by top vendors, and by project category. Use to see the shape of what's available before searching.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "suggest_for_task",
    description: "Given what you need to sense or build, suggest candidate parts from the catalog, ranked by keyword relevance. E.g. 'detect a person sitting still through a wall' or 'measure tilt and vibration'.",
    inputSchema: { type: "object", properties: { need: { type: "string" } }, required: ["need"] },
  },
  {
    name: "search_pool",
    description: "Search the large imported PARTS POOL (the bulk LCSC/JLCPCB + distributor catalog, ~thousands of parts) by keyword. Use this to find candidate parts that aren't yet in the curated catalog; then promote_from_pool to bring one in.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "keywords — MPN, manufacturer, category, what it senses" },
        category: { type: "string", description: "optional category filter (e.g. imu/motion, radar/presence, environmental)" },
        limit: { type: "number", description: "max results (default 20, max 100)" },
      },
      required: ["query"],
    },
  },
  {
    name: "list_cogs",
    description: "List the cog registry — the installable WeftOS/Cognitum cogs (sensor readers, bridges, apps) and the catalog part ids each works with (maps_to). Use to discover whether a sensor already has a cog.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "find_cog",
    description: "Find cogs by keyword (cog name, sensor, category). E.g. 'radar', 'ecg', 'ld2450'. Returns the matching cogs and the catalog parts they map to.",
    inputSchema: { type: "object", properties: { query: { type: "string" } }, required: ["query"] },
  },
  {
    name: "promote_from_pool",
    description: "Promote a pool part (by MPN) toward the curated catalog — lands as a pending contribution for review. Requires a contribute-scoped API key.",
    inputSchema: { type: "object", properties: { mpn: { type: "string" } }, required: ["mpn"] },
  },
  {
    name: "add_part",
    description: "Propose a new part (or an update) for the catalog. Requires a contribute-scoped API key. Lands as a pending contribution for review.",
    inputSchema: {
      type: "object",
      properties: {
        type: { type: "string", enum: ["module", "chip", "project"] },
        part: { type: "object", description: "the part record (must include id and name)" },
      },
      required: ["type", "part"],
    },
  },
];

function text(obj: unknown) {
  return { content: [{ type: "text", text: typeof obj === "string" ? obj : JSON.stringify(obj, null, 2) }] };
}

function safeJson(s: any): any {
  try { return JSON.parse(s); } catch { return s; }
}

async function callTool(name: string, args: any, env: Env, scope: Scope) {
  const DB = env.DB;
  if (name === "search_sensors") {
    const limit = Math.min(Math.max(1, args.limit || 20), 100);
    const exp = await expandedSearch(DB, String(args.query || ""), { type: args.type, kind: args.kind, limit });
    return text(exp);
  }
  if (name === "get_part") {
    const row = await DB.prepare("SELECT data FROM parts WHERE id=?").bind(args.id).first<{ data: string }>();
    if (!row) return { ...text(`no part with id '${args.id}'`), isError: true };
    return text(JSON.parse(row.data));
  }
  if (name === "list_tree") {
    const byType = (await DB.prepare("SELECT type,COUNT(*) n FROM parts WHERE status='published' GROUP BY type").all()).results;
    const byKind = (await DB.prepare("SELECT kind,COUNT(*) n FROM parts WHERE type='module' AND status='published' GROUP BY kind").all()).results;
    const byVendor = (await DB.prepare("SELECT vendor,COUNT(*) n FROM parts WHERE vendor<>'' AND status='published' GROUP BY vendor ORDER BY n DESC LIMIT 20").all()).results;
    const byCat = (await DB.prepare("SELECT category,COUNT(*) n FROM parts WHERE category<>'' AND status='published' GROUP BY category ORDER BY n DESC LIMIT 20").all()).results;
    return text({ by_type: byType, module_kinds: byKind, top_vendors: byVendor, categories: byCat });
  }
  if (name === "suggest_for_task") {
    const toks = String(args.need || "").toLowerCase().split(/[^a-z0-9]+/).filter((t) => t.length > 2).slice(0, 8);
    if (!toks.length) return text({ suggestions: [] });
    const where = toks.map(() => "search LIKE ?").join(" OR ");
    const binds = toks.map((t) => `%${t}%`);
    const { results } = await DB.prepare(
      `SELECT id,type,name,vendor,kind,category FROM parts WHERE status='published' AND (${where}) LIMIT 40`
    ).bind(...binds).all();
    // rank by how many tokens appear in the row's searchable text
    const scored = results.map((r: any) => ({ ...r, score: toks.filter((t) => JSON.stringify(r).toLowerCase().includes(t)).length }));
    scored.sort((a, b) => b.score - a.score);
    return text({ need: args.need, suggestions: scored.slice(0, 12) });
  }
  if (name === "search_pool") {
    const limit = Math.min(Math.max(1, args.limit || 20), 100);
    const results = await searchPool(DB, String(args.query || ""), args.category, limit);
    return text({ count: results.length, results });
  }
  if (name === "list_cogs") {
    const { results } = await DB.prepare("SELECT id,name,category,version,description,store_id,hardware,bind_port,maps_to,hash FROM cogs ORDER BY name").all();
    const cogs = (results as any[]).map((r) => ({ ...r, hardware: safeJson(r.hardware), maps_to: safeJson(r.maps_to) }));
    return text({ count: cogs.length, cogs });
  }
  if (name === "find_cog") {
    const cogs = await searchCogs(DB, String(args.query || ""), 50);
    return text({ count: cogs.length, cogs });
  }
  if (name === "promote_from_pool") {
    if (scope !== "contribute" && scope !== "admin") return { ...text("promote_from_pool needs a contribute-scoped API key"), isError: true };
    const row = await DB.prepare("SELECT data FROM pool WHERE mpn=?").bind(args.mpn).first<{ data: string }>();
    if (!row) return { ...text(`no pool part with mpn '${args.mpn}'`), isError: true };
    const part = JSON.parse(row.data);
    await DB.prepare("INSERT INTO contributions (part_id,type,data,author,created,status) VALUES (?,?,?,?,?, 'pending')")
      .bind(part.mpn, "chip", JSON.stringify(part), "mcp:promote", new Date().toISOString()).run();
    return text({ ok: true, status: "pending", mpn: part.mpn, note: "Promoted from pool; pending review." });
  }
  if (name === "add_part") {
    if (scope !== "contribute" && scope !== "admin") return { ...text("add_part needs a contribute-scoped API key"), isError: true };
    const p = args.part || {};
    if (!p.id || !p.name) return { ...text("part must include id and name"), isError: true };
    await DB.prepare("INSERT INTO contributions (part_id,type,data,author,created,status) VALUES (?,?,?,?,?, 'pending')")
      .bind(p.id, args.type, JSON.stringify(p), "mcp", new Date().toISOString()).run();
    return text({ ok: true, status: "pending", id: p.id, note: "Submitted for review." });
  }
  return { ...text(`unknown tool '${name}'`), isError: true };
}

/** Handle one JSON-RPC message. Returns the response object, or null for notifications. */
export async function handleRpc(msg: any, env: Env, scope: Scope): Promise<any | null> {
  const { id, method, params } = msg || {};
  const ok = (result: any) => ({ jsonrpc: "2.0", id, result });
  const err = (code: number, message: string) => ({ jsonrpc: "2.0", id, error: { code, message } });
  try {
    switch (method) {
      case "initialize":
        return ok({
          protocolVersion: "2024-11-05",
          capabilities: { tools: {} },
          serverInfo: { name: env.EXPLORER_NAME || "WeftOS Sensor Explorer", version: VERSION },
        });
      case "notifications/initialized":
      case "notifications/cancelled":
        return null; // notification, no response
      case "ping":
        return ok({});
      case "tools/list":
        return ok({ tools: TOOLS });
      case "tools/call": {
        const res = await callTool(params?.name, params?.arguments || {}, env, scope);
        return ok(res);
      }
      default:
        return err(-32601, `method not found: ${method}`);
    }
  } catch (e: any) {
    return err(-32603, `internal error: ${e?.message || e}`);
  }
}

export { TOOLS };
