// WeftOS Sensor Explorer — Cloudflare Worker.
// MCP agent surface at POST /mcp (API-key auth) + public read REST + a minimal landing page.
// The rich tree-browse UI and the human contribution flow are the next phase.

import { Hono } from "hono";
import { cors } from "hono/cors";
import { handleRpc, type Env } from "./mcp";

type Scope = "read" | "contribute" | "admin";

const app = new Hono<{ Bindings: Env }>();
app.use("*", cors());

async function resolveScope(req: Request, env: Env): Promise<Scope | null> {
  const auth = req.headers.get("authorization") || "";
  const key = auth.toLowerCase().startsWith("bearer ") ? auth.slice(7).trim() : auth.trim();
  if (!key) return null;
  if (env.BOOTSTRAP_API_KEY && key === env.BOOTSTRAP_API_KEY) return "admin";
  const row = await env.DB.prepare("SELECT scope FROM api_keys WHERE key=?").bind(key).first<{ scope: Scope }>();
  return row?.scope ?? null;
}

// ---- MCP agent surface (authed) ----
app.post("/mcp", async (c) => {
  const scope = await resolveScope(c.req.raw, c.env);
  if (!scope) return c.json({ jsonrpc: "2.0", id: null, error: { code: -32001, message: "unauthorized — send a Bearer API key" } }, 401);
  let body: any;
  try {
    body = await c.req.json();
  } catch {
    return c.json({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "parse error" } }, 400);
  }
  if (Array.isArray(body)) {
    const out = (await Promise.all(body.map((m) => handleRpc(m, c.env, scope)))).filter(Boolean);
    return c.json(out);
  }
  const res = await handleRpc(body, c.env, scope);
  return res ? c.json(res) : new Response(null, { status: 204 }); // 204 for notifications
});

// ---- public read REST ----
app.get("/api/tree", async (c) => {
  const byType = (await c.env.DB.prepare("SELECT type,COUNT(*) n FROM parts WHERE status='published' GROUP BY type").all()).results;
  const byKind = (await c.env.DB.prepare("SELECT kind,COUNT(*) n FROM parts WHERE type='module' AND status='published' GROUP BY kind").all()).results;
  const byVendor = (await c.env.DB.prepare("SELECT vendor,COUNT(*) n FROM parts WHERE vendor<>'' AND status='published' GROUP BY vendor ORDER BY n DESC LIMIT 25").all()).results;
  return c.json({ by_type: byType, module_kinds: byKind, top_vendors: byVendor });
});

app.get("/api/search", async (c) => {
  const q = (c.req.query("q") || "").toLowerCase();
  const type = c.req.query("type");
  const limit = Math.min(Math.max(1, Number(c.req.query("limit")) || 30), 200);
  let sql = "SELECT id,type,name,vendor,kind,category FROM parts WHERE status='published' AND search LIKE ?";
  const binds: any[] = [`%${q}%`];
  if (type && type !== "any") { sql += " AND type=?"; binds.push(type); }
  sql += " ORDER BY type,name LIMIT ?"; binds.push(limit);
  const { results } = await c.env.DB.prepare(sql).bind(...binds).all();
  return c.json({ count: results.length, results });
});

app.get("/api/parts/:id", async (c) => {
  const row = await c.env.DB.prepare("SELECT data FROM parts WHERE id=?").bind(c.req.param("id")).first<{ data: string }>();
  return row ? c.json(JSON.parse(row.data)) : c.json({ error: "not found" }, 404);
});

// Full export, so the console/appliance can embed a snapshot.
app.get("/api/catalog.json", async (c) => {
  const rows = (await c.env.DB.prepare("SELECT type,data FROM parts WHERE status='published'").all()).results as any[];
  const cat: any = { schema: 1, source: "sensor-explorer", projects: [], modules: [], chips: [] };
  for (const r of rows) (cat[`${r.type}s`] ||= []).push(JSON.parse(r.data as string));
  return c.json(cat);
});

app.get("/healthz", (c) => c.json({ ok: true }));

// ---- minimal landing (rich tree UI is next phase) ----
app.get("/", async (c) => {
  const n = (await c.env.DB.prepare("SELECT type,COUNT(*) c FROM parts WHERE status='published' GROUP BY type").all()).results as any[];
  const counts = Object.fromEntries(n.map((r) => [r.type, r.c]));
  const name = c.env.EXPLORER_NAME || "WeftOS Sensor Explorer";
  return c.html(`<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1">
<title>${name}</title>
<style>body{font:16px/1.6 system-ui,sans-serif;max-width:760px;margin:40px auto;padding:0 20px;color:#23262b;background:#f6efe9}
code{background:#eadfd4;padding:2px 6px;border-radius:5px}a{color:#6c9ce8}h1{margin-bottom:4px}.k{color:#7c818a}</style>
<h1>${name}</h1>
<p class=k>A browsable, contributable hardware catalog with an MCP agent surface.</p>
<p><b>${counts.module || 0}</b> modules · <b>${counts.chip || 0}</b> chips · <b>${counts.project || 0}</b> projects</p>
<h3>Agent surface (MCP)</h3>
<p>Point a harness at <code>POST /mcp</code> with <code>Authorization: Bearer &lt;key&gt;</code>. Tools:
<code>search_sensors</code>, <code>get_part</code>, <code>list_tree</code>, <code>suggest_for_task</code>, <code>add_part</code> (contribute scope).</p>
<h3>Read API</h3>
<p><a href="/api/tree">/api/tree</a> · <a href="/api/search?q=radar">/api/search?q=radar</a> · <code>/api/parts/:id</code> · <a href="/api/catalog.json">/api/catalog.json</a></p>
<p class=k>Rich tree-browse UI + human contribution flow: coming next.</p>`);
});

export default app;
