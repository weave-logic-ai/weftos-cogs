// MCP agent surface for the Sensor Explorer — stateless JSON-RPC 2.0 over HTTP (Streamable HTTP,
// single-response mode). A harness connects here as an MCP tool server and can search, read, browse
// the tree, get a suggestion for a task, and (with contribute scope) propose a new part.

export interface Env {
  DB: D1Database;
  EXPLORER_NAME?: string;
  BOOTSTRAP_API_KEY?: string;
}

const VERSION = "0.1.0";

type Scope = "read" | "contribute" | "admin";

const TOOLS = [
  {
    name: "search_sensors",
    description: "Search the hardware catalog (sensors, modules, chips, projects) by keyword. Returns matching parts with id, type, name, vendor and a one-line summary.",
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

async function callTool(name: string, args: any, env: Env, scope: Scope) {
  const DB = env.DB;
  if (name === "search_sensors") {
    const limit = Math.min(Math.max(1, args.limit || 20), 100);
    const like = `%${String(args.query || "").toLowerCase()}%`;
    let sql = "SELECT id,type,name,vendor,kind,category FROM parts WHERE status='published' AND search LIKE ?";
    const binds: any[] = [like];
    if (args.type && args.type !== "any") { sql += " AND type=?"; binds.push(args.type); }
    if (args.kind) { sql += " AND kind=?"; binds.push(args.kind); }
    sql += " ORDER BY type,name LIMIT ?"; binds.push(limit);
    const { results } = await DB.prepare(sql).bind(...binds).all();
    return text({ count: results.length, results });
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
