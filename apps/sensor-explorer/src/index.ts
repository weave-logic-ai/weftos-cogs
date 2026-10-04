// WeftOS Sensor Explorer — Cloudflare Worker.
// MCP agent surface at POST /mcp (API-key auth) + public read REST + a rich tree-browse UI at GET /.

import { Hono } from "hono";
import { cors } from "hono/cors";
import { handleRpc, type Env } from "./mcp";
import { expandedSearch, searchCogs } from "./search";

type Scope = "read" | "contribute" | "admin";

// ---- facet derivation (shared by /api/facets and, injected, the browser UI) ----
// Keyword tables are injected into the page so server-side counts and client-side
// filtering derive identical facets from the same source of truth.
const CATEGORY_RULES: Array<[string, string[]]> = [
  ["radar/presence", ["radar", "mmwave", "mm-wave", "60ghz", "24ghz", "presence", "pcr", "doppler", "fmcw", "occupancy", "micro-motion", "ld2410", "ld2450", "ld6002", "hlk-", "rd-03", "xm125", "respiratory", "breath radar"]],
  ["ranging/tof", ["time-of-flight", "tof", "lidar", "vl53", "rangefinder", "ultrasonic", "hc-sr04", "proximity", "distance sensor", "laser distance"]],
  ["thermal", ["thermal", "thermopile", "grid-eye", "grideye", "amg8833", "mlx906", "mlx90640", "mlx90614", "ir array", "thermal camera", "flir", "lepton"]],
  ["vision/camera", ["camera", "image sensor", "ov5640", "ov2640", "ov7670", "cmos image", "mipi csi", "imx", "gc0", "machine vision", "esp32-cam"]],
  ["audio/mic", ["microphone", "mems mic", "i2s mic", "inmp441", "sph0645", "sound sensor", "ky-038", "audio", "acoustic", "hydrophone"]],
  ["imu/motion", ["imu", "accelerometer", "gyroscope", "magnetometer", "mpu6050", "mpu9250", "mpu-", "bmi160", "bmi270", "icm-", "lsm6", "lsm9", "6-axis", "9-axis", "tilt", "vibration", "motion sensor", "adxl"]],
  ["biometric", ["heart rate", "ppg", "ecg", "ekg", "spo2", "max30", "pulse oximeter", "respiration", "bioimpedance", "gsr", "eeg", "emg", "biometric"]],
  ["environmental/gas", ["gas", "co2", "voc", "bme680", "bme280", "bmp", "sgp30", "sgp40", "ccs811", "humidity", "air quality", "particulate", "pm2.5", "pm2_5", "sht3", "sht4", "dht", "environmental", "barometric", "pressure sensor", "temperature"]],
  ["positioning/uwb", ["uwb", "ultra-wideband", "dw1000", "dw3000", "decawave", "gps", "gnss", "glonass", "rtk", "neo-m", "positioning", "u-blox"]],
  ["rf/sdr", ["sdr", "rtl-sdr", "hackrf", "software-defined", "software defined radio", "lora", "sx1276", "sx1262", "sx127", "sx126", "nrf24", "sub-ghz", "433mhz", "915mhz", "868mhz", "zigbee", "rf transceiver"]],
  ["display", ["display", "lcd", "oled", "tft", "epaper", "e-paper", "e-ink", "amoled", "screen", "ssd1306", "ili9341"]],
  ["board", ["esp32", "esp8266", "esp32-s3", "esp32-c3", "raspberry pi", "pi zero", "pi 5", "rp2040", "pico", "stm32", "teensy", "arduino", "nrf52", "fpga", "microcontroller", "dev board", "single-board", "sbc", "soc", "risc-v"]],
];
const INTERFACE_RULES: Array<[string, string[]]> = [
  ["i2c", ["i2c", "i²c", "qwiic", "stemma", "sda/scl", "sda ", "scl "]],
  ["spi", ["spi", "mosi", "miso", "sclk"]],
  ["uart", ["uart", "usart", " serial", "ttl serial", "tx/rx"]],
  ["mipi", ["mipi", "csi-2", "dsi", "camera serial"]],
  ["analog", ["analog", "adc ", "0-3.3v", "voltage output", "ads1115", "ads1015", "analog out"]],
  ["sdr", ["sdr", "iq stream", "rf samples", "i/q "]],
  ["uwb", ["uwb", "ultra-wideband"]],
  ["ble", ["ble", "bluetooth"]],
  ["usb", ["usb"]],
];

function safeJson(s: any): any {
  try { return JSON.parse(s); } catch { return s; }
}

function haystackOf(r: any): string {
  const parts: string[] = [];
  const push = (v: any) => {
    if (v == null) return;
    if (Array.isArray(v)) v.forEach(push);
    else if (typeof v === "object") Object.values(v).forEach(push);
    else parts.push(String(v));
  };
  push(r.name); push(r.vendor); push(r.manufacturer); push(r.kind); push(r.role);
  push(r.summary); push(r.category); push(r.difficulty); push(r.tags); push(r.good_for);
  push(r.not_for); push(r.notes); push(r.chips); push(r.pins); push(r.spec); push(r.modules);
  return parts.join(" ").toLowerCase();
}
function deriveCats(r: any): string[] {
  const hay = haystackOf(r);
  const kind = String(r.kind || "").toLowerCase();
  const out = new Set<string>();
  if (kind === "display") out.add("display");
  if (kind === "board") out.add("board");
  for (const [cat, kws] of CATEGORY_RULES) if (kws.some((k) => hay.includes(k))) out.add(cat);
  if (out.size === 0) out.add("other");
  return [...out];
}
function deriveIfaces(r: any): string[] {
  const hay = haystackOf(r);
  const out = new Set<string>();
  for (const [i, kws] of INTERFACE_RULES) if (kws.some((k) => hay.includes(k))) out.add(i);
  return [...out];
}

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

// Browsable taxonomy with counts, derived from the data. Pure read over the parts table;
// the full record JSON is parsed and the sensing-category / interface facets are derived in JS.
app.get("/api/facets", async (c) => {
  const rows = (await c.env.DB.prepare("SELECT type,data FROM parts WHERE status='published'").all()).results as any[];
  const typeC = new Map<string, number>();
  const catC = new Map<string, number>();
  const ifC = new Map<string, number>();
  const venC = new Map<string, number>();
  const inc = (m: Map<string, number>, k: string) => { if (!k) return; m.set(k, (m.get(k) || 0) + 1); };
  for (const row of rows) {
    let r: any;
    try { r = JSON.parse(row.data as string); } catch { continue; }
    inc(typeC, row.type);
    for (const cat of deriveCats(r)) inc(catC, cat);
    for (const i of deriveIfaces(r)) inc(ifC, i);
    inc(venC, String(r.vendor || r.manufacturer || "").trim());
  }
  const arr = (m: Map<string, number>, lim?: number) => {
    let a = [...m].filter(([k]) => k).map(([key, n]) => ({ key, n })).sort((x, y) => y.n - x.n || x.key.localeCompare(y.key));
    if (lim) a = a.slice(0, lim);
    return a;
  };
  return c.json({ types: arr(typeC), categories: arr(catC), interfaces: arr(ifC), vendors: arr(venC, 30) });
});

// Expanding search: curated catalog + (when thin) the imported pool + the cog registry, so a query
// never dead-ends. `cogs` and `pool` entries are labeled with a `source`.
app.get("/api/search", async (c) => {
  const q = c.req.query("q") || "";
  const type = c.req.query("type") || undefined;
  const limit = Math.min(Math.max(1, Number(c.req.query("limit")) || 30), 200);
  const exp = await expandedSearch(c.env.DB, q, { type, limit });
  return c.json(exp);
});

app.get("/api/parts/:id", async (c) => {
  const row = await c.env.DB.prepare("SELECT data FROM parts WHERE id=?").bind(c.req.param("id")).first<{ data: string }>();
  return row ? c.json(JSON.parse(row.data)) : c.json({ error: "not found" }, 404);
});

// ---- cog registry ----
function cogRow(r: any) {
  return { ...r, hardware: safeJson(r.hardware), maps_to: safeJson(r.maps_to) };
}
app.get("/api/cogs", async (c) => {
  const q = c.req.query("q");
  if (q) {
    const cogs = await searchCogs(c.env.DB, q, 100);
    return c.json({ count: cogs.length, cogs });
  }
  const { results } = await c.env.DB.prepare(
    "SELECT id,name,category,version,description,store_id,hardware,bind_port,maps_to,hash FROM cogs ORDER BY name"
  ).all();
  const cogs = (results as any[]).map(cogRow);
  return c.json({ count: cogs.length, cogs });
});
app.get("/api/cogs/:id", async (c) => {
  const row = await c.env.DB.prepare("SELECT data FROM cogs WHERE id=?").bind(c.req.param("id")).first<{ data: string }>();
  return row ? c.json(JSON.parse(row.data)) : c.json({ error: "not found" }, 404);
});

// The cog (if any) and firmware (if any) that map to a given part id.
async function cogsForPart(env: Env, partId: string) {
  const { results } = await env.DB.prepare("SELECT data FROM cogs WHERE maps_to LIKE ?").bind(`%"${partId}"%`).all();
  return (results as any[]).map((r) => JSON.parse(r.data)).filter((c) => (c.maps_to || []).includes(partId));
}
async function firmwareForPart(env: Env, partId: string) {
  const { results } = await env.DB.prepare("SELECT data FROM firmware WHERE maps_to LIKE ?").bind(`%"${partId}"%`).all();
  return (results as any[]).map((r) => JSON.parse(r.data)).filter((f) => (f.maps_to || []).includes(partId));
}

// ---- research queue + cog requests (plain writes; no auth, these are user-raised intents) ----
app.post("/api/research", async (c) => {
  let b: any = {};
  try { b = await c.req.json(); } catch {}
  const refId = String(b.ref_id || b.id || "").trim();
  if (!refId) return c.json({ error: "ref_id required" }, 400);
  const refType = String(b.ref_type || "part");
  const refHash = String(b.ref_hash || "");
  const note = String(b.note || "");
  await c.env.DB.prepare(
    "INSERT INTO research (ref_hash,ref_type,ref_id,note,status,created) VALUES (?,?,?,?, 'queued', ?)"
  ).bind(refHash, refType, refId, note, new Date().toISOString()).run();
  return c.json({ ok: true, status: "queued", ref_type: refType, ref_id: refId });
});
app.post("/api/cog-request", async (c) => {
  let b: any = {};
  try { b = await c.req.json(); } catch {}
  const partId = String(b.part_id || b.id || "").trim();
  if (!partId) return c.json({ error: "part_id required" }, 400);
  await c.env.DB.prepare(
    "INSERT INTO cog_requests (part_id,part_hash,note,status,created) VALUES (?,?,?, 'requested', ?)"
  ).bind(partId, String(b.part_hash || ""), String(b.note || ""), new Date().toISOString()).run();
  return c.json({ ok: true, status: "requested", part_id: partId });
});

// Full export, so the console/appliance can embed a snapshot.
app.get("/api/catalog.json", async (c) => {
  const rows = (await c.env.DB.prepare("SELECT type,data FROM parts WHERE status='published'").all()).results as any[];
  const cat: any = { schema: 1, source: "sensor-explorer", projects: [], modules: [], chips: [] };
  for (const r of rows) (cat[`${r.type}s`] ||= []).push(JSON.parse(r.data as string));
  return c.json(cat);
});

// Imported parts pool (bulk LCSC/JLCPCB + distributor). Separate from the curated catalog.
app.get("/api/pool/search", async (c) => {
  const q = (c.req.query("q") || "").toLowerCase();
  const cat = c.req.query("category");
  const limit = Math.min(Math.max(1, Number(c.req.query("limit")) || 30), 200);
  let sql = "SELECT mpn,manufacturer,name,category,price,datasheet FROM pool WHERE search LIKE ?";
  const binds: any[] = [`%${q}%`];
  if (cat) { sql += " AND category=?"; binds.push(cat); }
  sql += " ORDER BY manufacturer,mpn LIMIT ?"; binds.push(limit);
  const { results } = await c.env.DB.prepare(sql).bind(...binds).all();
  return c.json({ count: results.length, results });
});
app.get("/api/pool/stats", async (c) => {
  const total = (await c.env.DB.prepare("SELECT COUNT(*) n FROM pool").first<{ n: number }>())?.n ?? 0;
  const byCat = (await c.env.DB.prepare("SELECT category,COUNT(*) n FROM pool GROUP BY category ORDER BY n DESC LIMIT 30").all()).results;
  return c.json({ total, by_category: byCat });
});

app.get("/healthz", (c) => c.json({ ok: true }));

// ---- item-detail page (server-rendered, tabbed) ----
app.get("/part/:id", async (c) => {
  const id = c.req.param("id");
  const row = await c.env.DB.prepare("SELECT type,hash,data FROM parts WHERE id=?").bind(id).first<{ type: string; hash: string; data: string }>();
  if (!row) return c.html(PART_NOT_FOUND(id), 404);
  const part = JSON.parse(row.data);
  const [cogs, firmware] = await Promise.all([cogsForPart(c.env, id), firmwareForPart(c.env, id)]);
  return c.html(PART_PAGE(c.env.EXPLORER_NAME || "WeftOS Sensor Explorer", row.type, row.hash || "", part, cogs, firmware));
});

// ---- rich tree-browse UI ----
app.get("/", (c) => {
  const name = c.env.EXPLORER_NAME || "WeftOS Sensor Explorer";
  return c.html(PAGE(name));
});

export default app;

// The single-page app. Vanilla JS, no build step. Facet keyword tables are injected so the
// client derives the same sensing-category / interface facets the server counts in /api/facets.
// NOTE: the client script below must not use backticks or ${...} — it lives inside this template literal.
function PAGE(name: string): string {
  const CATS = JSON.stringify(CATEGORY_RULES);
  const IFS = JSON.stringify(INTERFACE_RULES);
  return `<!doctype html><html lang=en><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<meta name=color-scheme content="light dark">
<title>${name}</title>
<style>
:root{
  --bg:#f4ede5;--panel:#efe6dc;--card:#fffdfb;--ink:#20242b;--ink-soft:#4b515b;--grey:#6a6f78;
  --line:#e3d8cb;--line-soft:#ece3d8;--wl:#4f84d6;--wl-ink:#3a6bb8;--accent:#d97b2b;--green:#2f8b57;
  --pool:#7a5cc0;--noteb:#fdf4e6;--chip-bg:#f2ebe2;--focus:#2b6fd6;--shadow:rgba(60,45,30,.14);
  --fs-xs:12px;--fs-sm:13px;--fs-base:14px;--fs-md:15px;--fs-lg:17px;--fs-xl:21px;--fs-2xl:25px;
  --sp-1:4px;--sp-2:8px;--sp-3:12px;--sp-4:16px;--sp-5:20px;--sp-6:28px;--r:12px;--r-sm:8px;--r-pill:999px;
}
@media(prefers-color-scheme:dark){:root:not([data-theme=light]){
  --bg:#17191d;--panel:#1d2024;--card:#23272d;--ink:#e8eaed;--ink-soft:#c3c8cf;--grey:#9aa0a8;
  --line:#33373e;--line-soft:#2b2f35;--wl:#7aa6ec;--wl-ink:#9cc0f5;--accent:#e8a158;--green:#56b882;
  --pool:#a98bea;--noteb:#2a2519;--chip-bg:#2a2e34;--focus:#8ab4ff;--shadow:rgba(0,0,0,.45);
}}
:root[data-theme=dark]{
  --bg:#17191d;--panel:#1d2024;--card:#23272d;--ink:#e8eaed;--ink-soft:#c3c8cf;--grey:#9aa0a8;
  --line:#33373e;--line-soft:#2b2f35;--wl:#7aa6ec;--wl-ink:#9cc0f5;--accent:#e8a158;--green:#56b882;
  --pool:#a98bea;--noteb:#2a2519;--chip-bg:#2a2e34;--focus:#8ab4ff;--shadow:rgba(0,0,0,.45);
}
*{box-sizing:border-box}
button{font-family:inherit}
html,body{margin:0}
body{background:var(--bg);color:var(--ink);font:var(--fs-md)/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;-webkit-font-smoothing:antialiased}
a{color:var(--wl-ink);text-decoration:none}a:hover{text-decoration:underline}
:focus{outline:none}
:focus-visible{outline:2px solid var(--focus);outline-offset:2px;border-radius:4px}
.vh{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap;border:0}

/* header */
header{padding:var(--sp-5) var(--sp-5) var(--sp-3);max-width:1500px}
h1{margin:0;font-size:var(--fs-2xl);font-weight:700;letter-spacing:-.015em}
.sub{color:var(--grey);margin:6px 0 0;font-size:var(--fs-sm);line-height:1.5}
.sub a{color:var(--grey);text-decoration:underline;text-underline-offset:2px}
.sub code{font-size:var(--fs-xs);background:var(--chip-bg);padding:1px 6px;border-radius:5px;color:var(--ink-soft)}

/* toolbar */
.bar{position:sticky;top:0;z-index:30;background:color-mix(in srgb,var(--bg) 92%,transparent);backdrop-filter:blur(8px);border-bottom:1px solid var(--line);padding:10px var(--sp-5);display:flex;gap:10px;align-items:center;flex-wrap:wrap}
.searchbox{flex:1;min-width:200px;position:relative;display:flex;align-items:center}
.searchbox svg{position:absolute;left:13px;width:16px;height:16px;color:var(--grey);pointer-events:none}
#q{width:100%;padding:10px 38px 10px 38px;border:1px solid var(--line);border-radius:var(--r-pill);background:var(--card);color:var(--ink);font-size:var(--fs-base)}
#q::placeholder{color:var(--grey)}
#q:focus-visible{outline:2px solid var(--focus);outline-offset:1px;border-color:transparent}
.qclear{position:absolute;right:8px;display:none;width:24px;height:24px;border:none;background:var(--chip-bg);color:var(--ink-soft);border-radius:50%;cursor:pointer;font-size:14px;line-height:1;align-items:center;justify-content:center}
.qclear.on{display:inline-flex}
.seg{display:inline-flex;border:1px solid var(--line);border-radius:var(--r-pill);overflow:hidden;background:var(--card)}
.seg button{border:none;background:none;color:var(--ink-soft);font:600 var(--fs-sm)/1 inherit;padding:9px 15px;cursor:pointer;display:flex;align-items:center;gap:6px}
.seg button .ct{color:var(--grey);font-weight:600;font-variant-numeric:tabular-nums}
.seg button[aria-pressed=true]{background:var(--wl);color:#fff}
.seg button[aria-pressed=true] .ct{color:rgba(255,255,255,.8)}
#count{color:var(--grey);font-size:var(--fs-sm);white-space:nowrap;font-variant-numeric:tabular-nums}
.filtbtn{display:none;padding:9px 14px;border:1px solid var(--line);border-radius:var(--r-pill);background:var(--card);color:var(--ink);font-weight:600;font-size:var(--fs-sm);cursor:pointer}

/* active filter chips */
#active{display:none;gap:7px;flex-wrap:wrap;align-items:center;padding:10px var(--sp-5) 0}
.achip{display:inline-flex;align-items:center;gap:6px;background:var(--wl);color:#fff;border:none;border-radius:var(--r-pill);padding:5px 10px;font-size:var(--fs-xs);font-weight:600;cursor:pointer}
.achip.qchip{background:var(--accent)}
.achip .x{opacity:.85;font-size:13px;line-height:1}
.clearall{background:none;border:none;color:var(--wl-ink);font:600 var(--fs-sm)/1 inherit;cursor:pointer;padding:5px 6px;text-decoration:underline;text-underline-offset:2px}

/* layout */
.wrap{display:flex;align-items:flex-start;gap:0;max-width:1500px}
.side{width:264px;flex:none;padding:var(--sp-4) var(--sp-3) 40px;border-right:1px solid var(--line);background:var(--panel);position:sticky;top:60px;max-height:calc(100vh - 60px);overflow:auto}
.sec{margin-bottom:var(--sp-1);border-bottom:1px solid var(--line-soft);padding-bottom:var(--sp-2)}
.sec:last-child{border-bottom:none}
.sechead{width:100%;text-align:left;background:none;border:none;color:var(--ink);font-weight:700;font-size:var(--fs-xs);letter-spacing:.05em;text-transform:uppercase;padding:9px 4px;cursor:pointer;display:flex;align-items:center;gap:8px}
.caret{display:inline-block;transition:transform .15s;color:var(--grey);font-size:10px;width:10px}
.sec.open .caret{transform:rotate(90deg)}
.facets{list-style:none;margin:0 0 4px;padding:0;display:none}
.sec.open .facets{display:block}
.facet{display:flex;justify-content:space-between;align-items:center;gap:8px;padding:6px 9px;border-radius:var(--r-sm);cursor:pointer;font-size:var(--fs-sm);color:var(--ink-soft)}
.facet:hover{background:var(--bg)}
.facet.on{background:var(--wl);color:#fff;font-weight:600}
.facet.on .fn{color:rgba(255,255,255,.85)}
.fl{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.fn{color:var(--grey);font-size:var(--fs-xs);font-variant-numeric:tabular-nums;flex:none}

/* results */
.main{flex:1;min-width:0;padding:var(--sp-4) var(--sp-5) 64px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(250px,1fr));gap:var(--sp-3)}
.card{display:flex;flex-direction:column;width:100%;background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:14px 15px;cursor:pointer;transition:border-color .12s,box-shadow .12s,transform .12s;text-align:left;color:var(--ink);font-family:inherit}
.card:hover{border-color:var(--wl);box-shadow:0 4px 14px var(--shadow);transform:translateY(-1px)}
.card:focus-visible{outline:2px solid var(--focus);outline-offset:2px}
.cardhead{display:flex;align-items:center;gap:8px;margin-bottom:8px}
.badge{font-size:10px;font-weight:700;text-transform:uppercase;letter-spacing:.04em;padding:3px 8px;border-radius:var(--r-pill);color:#fff;flex:none}
.b-module{background:var(--wl)}.b-chip{background:var(--accent)}.b-project{background:var(--green)}.b-pool{background:var(--pool)}.b-cog{background:var(--pool)}
.xgroup{grid-column:1/-1;margin:18px 0 2px;display:flex;align-items:center;gap:10px;color:var(--grey);font-size:var(--fs-xs);text-transform:uppercase;letter-spacing:.05em;font-weight:700}
.xgroup::after{content:"";flex:1;height:1px;background:var(--line)}
.maps{font-size:11px;color:var(--pool)}
.vend{color:var(--grey);font-size:var(--fs-xs);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin-left:auto}
.card h3{margin:0 0 6px;font-size:var(--fs-md);font-weight:600;line-height:1.3}
.sum{margin:0 0 12px;color:var(--ink-soft);font-size:var(--fs-sm);line-height:1.45;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
.cardfoot{margin-top:auto;display:flex;flex-wrap:wrap;gap:6px;align-items:center}
.pill{font-size:11px;padding:3px 9px;border-radius:var(--r-pill);border:1px solid var(--line);color:var(--grey);background:var(--bg);white-space:nowrap}
.pill.cat{border-color:color-mix(in srgb,var(--wl) 55%,var(--line));color:var(--wl-ink)}
.price{margin-left:auto;font-size:var(--fs-sm);font-weight:700;color:var(--green);white-space:nowrap;font-variant-numeric:tabular-nums}
.promote{font-size:11px;color:var(--pool);border:1px dashed color-mix(in srgb,var(--pool) 60%,var(--line));border-radius:var(--r-pill);padding:3px 9px}

/* states */
.empty{color:var(--grey);padding:48px 8px;text-align:center}
.empty b{display:block;color:var(--ink);font-size:var(--fs-lg);font-weight:600;margin-bottom:6px}
.skel{background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:14px 15px;height:148px;overflow:hidden;position:relative}
.skel::after{content:"";position:absolute;inset:0;transform:translateX(-100%);background:linear-gradient(90deg,transparent,color-mix(in srgb,var(--ink) 7%,transparent),transparent);animation:sh 1.3s infinite}
.skel .l{height:10px;border-radius:5px;background:color-mix(in srgb,var(--ink) 9%,transparent);margin-bottom:10px}
.skel .l.w1{width:40%}.skel .l.w2{width:88%}.skel .l.w3{width:70%}
@keyframes sh{100%{transform:translateX(100%)}}
@media(prefers-reduced-motion:reduce){.skel::after{animation:none}.card,#panel,#scrim{transition:none}}

/* detail panel */
#scrim{position:fixed;inset:0;background:rgba(0,0,0,.45);opacity:0;pointer-events:none;transition:opacity .18s;z-index:40}
#scrim.on{opacity:1;pointer-events:auto}
#panel{position:fixed;top:0;right:0;height:100%;width:460px;max-width:94vw;background:var(--card);border-left:1px solid var(--line);box-shadow:-8px 0 30px var(--shadow);transform:translateX(100%);transition:transform .22s;z-index:50;display:flex;flex-direction:column}
#panel.open{transform:translateX(0)}
.ptop{display:flex;justify-content:flex-end;padding:12px 14px 0}
.pclose{background:var(--chip-bg);border:1px solid var(--line);color:var(--ink);border-radius:50%;width:34px;height:34px;font-size:19px;cursor:pointer;line-height:1}
#pbody{padding:6px 24px 48px;overflow:auto}
.phead{display:flex;align-items:center;gap:8px;margin:4px 0 8px}
#pbody h2{margin:2px 0 4px;font-size:var(--fs-xl);font-weight:700;line-height:1.2}
.pid{font-size:var(--fs-xs);color:var(--grey);font-family:ui-monospace,SFMono-Regular,Menlo,monospace}
.role{color:var(--grey);margin:0 0 10px;font-size:var(--fs-sm)}
.psum{font-size:var(--fs-base);color:var(--ink);margin:6px 0 10px;line-height:1.5}
.note{background:var(--noteb);border-left:3px solid var(--accent);padding:9px 12px;border-radius:0 6px 6px 0;margin:8px 0;font-size:var(--fs-sm);color:var(--ink)}
.lb{margin:12px 0;font-size:var(--fs-sm)}.lb ul{margin:5px 0 0;padding-left:18px}.lb li{margin:2px 0}
.lb.good b{color:var(--green)}.lb.bad b{color:var(--accent)}
.phint{background:var(--chip-bg);border:1px dashed color-mix(in srgb,var(--pool) 55%,var(--line));color:var(--ink-soft);padding:10px 12px;border-radius:var(--r-sm);font-size:var(--fs-sm);margin:10px 0}
table.spec{width:100%;border-collapse:collapse;margin:14px 0;font-size:var(--fs-sm)}
table.spec th{text-align:left;color:var(--grey);font-weight:600;padding:6px 12px 6px 0;vertical-align:top;white-space:nowrap;width:1%}
table.spec td{padding:6px 0;border-bottom:1px solid var(--line-soft);color:var(--ink)}
.links{margin:14px 0}.ext{display:inline-flex;align-items:center;gap:6px;padding:8px 14px;border:1px solid var(--wl);color:var(--wl-ink);border-radius:var(--r-pill);font-size:var(--fs-sm);font-weight:600}
.ext:hover{text-decoration:none;background:color-mix(in srgb,var(--wl) 12%,transparent)}
.buys{display:flex;flex-wrap:wrap;gap:8px;margin:14px 0}
.buy{display:inline-flex;gap:8px;align-items:center;background:var(--green);color:#fff;padding:8px 14px;border-radius:var(--r-pill);font-size:var(--fs-sm);font-weight:600}
.buy:hover{text-decoration:none;opacity:.92}.buy .price{color:#fff;margin:0}
.xref{margin:16px 0}.xref>b{display:block;color:var(--grey);font-size:var(--fs-xs);text-transform:uppercase;letter-spacing:.04em;margin-bottom:8px}
.xlinks{display:flex;flex-wrap:wrap;gap:7px}
.xlink{display:inline-block;padding:6px 12px;border:1px solid var(--line);border-radius:var(--r-pill);font-size:var(--fs-sm);cursor:pointer;background:var(--bg);color:var(--ink-soft)}
.xlink:hover{border-color:var(--wl);text-decoration:none;color:var(--wl-ink)}
.tags{display:flex;flex-wrap:wrap;gap:6px}.tagp{font-size:11px;padding:3px 9px;border:1px solid var(--line);border-radius:var(--r-pill);color:var(--grey)}
.loading{color:var(--grey)}

/* mobile */
.sideback{position:fixed;inset:0;background:rgba(0,0,0,.4);opacity:0;pointer-events:none;transition:opacity .2s;z-index:45}
@media(max-width:860px){
  header{padding:var(--sp-4) var(--sp-4) var(--sp-2)}
  .bar{padding:10px var(--sp-4)}
  #active{padding:10px var(--sp-4) 0}
  .main{padding:var(--sp-4) var(--sp-4) 64px}
  .side{position:fixed;top:0;left:0;height:100%;z-index:50;transform:translateX(-100%);transition:transform .2s;max-height:100%;width:284px;box-shadow:2px 0 22px var(--shadow)}
  body.drawer .side{transform:translateX(0)}
  body.drawer .sideback{opacity:1;pointer-events:auto}
  .filtbtn{display:inline-flex;align-items:center;gap:6px}
  #panel{width:100%;max-width:100%}
}
</style></head><body>
<header>
  <h1>${name}</h1>
  <p class=sub>A browsable, contributable hardware catalog with an MCP agent surface. <a href="/api/facets">facets</a> &middot; <a href="/api/catalog.json">catalog.json</a> &middot; <code>POST /mcp</code></p>
</header>
<div class=bar>
  <button class=filtbtn id=filtbtn aria-controls=side aria-expanded=false><span aria-hidden=true>&#9776;</span> Filters</button>
  <div class=searchbox role=search>
    <svg viewBox="0 0 24 24" fill=none stroke=currentColor stroke-width=2 aria-hidden=true><circle cx=11 cy=11 r=7/><path d="m21 21-4.3-4.3"/></svg>
    <label class=vh for=q>Search the catalog</label>
    <input id=q type=search placeholder="Search sensors, chips, projects…" autocomplete=off>
    <button class=qclear id=qclear type=button aria-label="Clear search">&#10005;</button>
  </div>
  <div class=seg role=group aria-label="Result source">
    <button id=mode-catalog aria-pressed=true>Catalog <span class=ct id=ct-catalog></span></button>
    <button id=mode-cogs aria-pressed=false>Cogs <span class=ct id=ct-cogs></span></button>
    <button id=mode-pool aria-pressed=false>Pool <span class=ct id=ct-pool></span></button>
  </div>
  <span id=count role=status aria-live=polite></span>
</div>
<div id=active></div>
<div class=wrap>
  <aside class=side id=side><nav id=tree aria-label="Filters"><p class=loading>Loading…</p></nav></aside>
  <main class=main><div class=grid id=grid aria-busy=true></div></main>
</div>
<div class=sideback id=sideback></div>
<div id=scrim></div>
<section id=panel role=dialog aria-modal=true aria-labelledby=ptitle aria-hidden=true>
  <div class=ptop><button class=pclose id=pclose aria-label="Close details">&times;</button></div>
  <div id=pbody></div>
</section>
<script>
var CAT_RULES=${CATS};
var IF_RULES=${IFS};
var INDEX=[],BY_ID={},PROJECTS=[],MODULES=[];
var FILTERS={type:{},category:{},interface:{},vendor:{}};
var Q='',searchIds=null,CAT_READY=false;
var MODE='catalog';
var POOL={stats:null,cat:null,results:[],byId:{},loaded:false,loading:false};
var EXTRA={cogs:[],pool:[],expanded:false};
var COGS={all:null,results:[],byId:{},loaded:false,loading:false};
var lastFocus=null;

var CAT_LABELS={'radar/presence':'Radar / Presence','ranging/tof':'Ranging / ToF','thermal':'Thermal','vision/camera':'Vision / Camera','audio/mic':'Audio / Mic','imu/motion':'IMU / Motion','biometric':'Biometric','environmental/gas':'Environmental / Gas','positioning/uwb':'Positioning / UWB','rf/sdr':'RF / SDR','display':'Display','board':'Board','other':'Other'};
var IF_LABELS={'i2c':'I\\u00b2C','spi':'SPI','uart':'UART','mipi':'MIPI CSI','analog':'Analog','sdr':'SDR','uwb':'UWB','ble':'BLE','usb':'USB'};
function titleCase(s){s=String(s||'');return s.charAt(0).toUpperCase()+s.slice(1);}
function labelCat(k){return CAT_LABELS[k]||String(k).split('/').map(titleCase).join(' / ');}
function labelIf(k){return IF_LABELS[k]||String(k).toUpperCase();}
function labelType(k){return titleCase(k);}
function labelFor(group,k){if(group==='category')return labelCat(k);if(group==='interface')return labelIf(k);if(group==='type')return labelType(k);return k;}

function esc(s){s=(s==null?'':String(s));return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');}
function trim(s,n){s=String(s);return s.length>n?s.slice(0,n-1)+'\\u2026':s;}
function nfmt(n){return String(n).replace(/\\B(?=(\\d{3})+(?!\\d))/g,',');}
function hayOf(r){
  var parts=[];
  function push(v){if(v==null)return;if(Array.isArray(v)){v.forEach(push);}else if(typeof v==='object'){Object.keys(v).forEach(function(k){push(v[k]);});}else{parts.push(String(v));}}
  push(r.name);push(r.vendor);push(r.manufacturer);push(r.kind);push(r.role);push(r.summary);push(r.category);push(r.difficulty);push(r.tags);push(r.good_for);push(r.not_for);push(r.notes);push(r.chips);push(r.pins);push(r.spec);push(r.modules);
  return parts.join(' ').toLowerCase();
}
function deriveCats(r){
  var hay=hayOf(r),kind=String(r.kind||'').toLowerCase(),out={};
  if(kind==='display')out['display']=1;
  if(kind==='board')out['board']=1;
  CAT_RULES.forEach(function(p){var cat=p[0],kws=p[1];for(var i=0;i<kws.length;i++){if(hay.indexOf(kws[i])>=0){out[cat]=1;break;}}});
  var keys=Object.keys(out);return keys.length?keys:['other'];
}
function deriveIfaces(r){
  var hay=hayOf(r),out={};
  IF_RULES.forEach(function(p){var id=p[0],kws=p[1];for(var j=0;j<kws.length;j++){if(hay.indexOf(kws[j])>=0){out[id]=1;break;}}});
  return Object.keys(out);
}
function vendorOf(r){return (r.vendor||r.manufacturer||'').trim();}
function priceOf(r){if(r&&r.buy&&r.buy[0]&&r.buy[0].price)return r.buy[0].price;if(r&&r.spec&&r.spec.price_ref)return r.spec.price_ref;return '';}
function keyOf(m){return m.type+':'+m.id;}

function loadCatalog(cat){
  [['module',cat.modules||[]],['chip',cat.chips||[]],['project',cat.projects||[]]].forEach(function(g){
    var type=g[0];
    g[1].forEach(function(r){
      var m={rec:r,id:r.id,type:type,name:r.name||r.id,vendor:vendorOf(r),cats:deriveCats(r),ifaces:deriveIfaces(r),summary:r.summary||'',price:priceOf(r),hay:hayOf(r)};
      INDEX.push(m);BY_ID[type+':'+r.id]=m;if(!BY_ID[r.id])BY_ID[r.id]=m;
    });
  });
  MODULES=INDEX.filter(function(m){return m.type==='module';});
  PROJECTS=cat.projects||[];
  CAT_READY=true;
  document.getElementById('ct-catalog').textContent=nfmt(INDEX.length);
}

function matchFacet(m,group,sel){
  var keys=Object.keys(sel);if(!keys.length)return true;
  if(group==='type')return !!sel[m.type];
  if(group==='vendor')return !!sel[m.vendor];
  if(group==='category')return m.cats.some(function(c){return sel[c];});
  if(group==='interface')return m.ifaces.some(function(i){return sel[i];});
  return true;
}
function passes(m){
  if(searchIds!==null && !searchIds[m.id])return false;
  return matchFacet(m,'type',FILTERS.type)&&matchFacet(m,'category',FILTERS.category)&&matchFacet(m,'interface',FILTERS.interface)&&matchFacet(m,'vendor',FILTERS.vendor);
}

function catPills(cats){var h='';cats.slice(0,2).forEach(function(c){h+='<span class="pill cat">'+esc(labelCat(c))+'</span>';});return h;}
function cardHTML(m){
  return '<article class=card role=button tabindex=0 data-id="'+esc(m.id)+'" data-key="'+esc(keyOf(m))+'" aria-label="'+esc(m.name)+', '+esc(m.type)+'">'
    +'<div class=cardhead><span class="badge b-'+m.type+'">'+m.type+'</span>'+(m.vendor?'<span class=vend>'+esc(m.vendor)+'</span>':'')+'</div>'
    +'<h3>'+esc(m.name)+'</h3>'
    +(m.summary?'<p class=sum>'+esc(trim(m.summary,160))+'</p>':'')
    +'<div class=cardfoot>'+catPills(m.cats)+(m.price?'<span class=price>'+esc(m.price)+'</span>':'')+'</div></article>';
}
function skeletons(n){var h='';for(var i=0;i<n;i++)h+='<div class=skel aria-hidden=true><div class="l w1"></div><div class="l w2"></div><div class="l w3"></div></div>';return h;}

function setCount(txt){document.getElementById('count').textContent=txt;}

function render(){
  if(MODE==='pool'){renderPool();return;}
  if(MODE==='cogs'){renderCogs();return;}
  var g=document.getElementById('grid');
  if(!CAT_READY){g.setAttribute('aria-busy','true');g.innerHTML=skeletons(8);return;}
  var items=INDEX.filter(passes);
  items.sort(function(a,b){return a.type.localeCompare(b.type)||a.name.localeCompare(b.name);});
  g.setAttribute('aria-busy','false');
  var html=items.length?items.map(cardHTML).join(''):'';
  // Expanding search: surface cog + pool hits so a catalog query never dead-ends.
  var extra='';
  if(Q&&EXTRA.cogs&&EXTRA.cogs.length){
    extra+='<div class=xgroup>Cogs</div>'+EXTRA.cogs.map(cogCardHTML).join('');
  }
  if(Q&&EXTRA.pool&&EXTRA.pool.length){
    extra+='<div class=xgroup>From the parts pool (uncurated)</div>'+EXTRA.pool.map(poolCardHTML).join('');
  }
  if(!items.length&&!extra){g.innerHTML=emptyHTML();}
  else{g.innerHTML=html+extra;}
  g.setAttribute('aria-busy','false');
  var extraN=(EXTRA.cogs?EXTRA.cogs.length:0)+(EXTRA.pool?EXTRA.pool.length:0);
  var msg=nfmt(items.length)+' result'+(items.length===1?'':'s');
  if(Q&&extraN)msg+=' + '+nfmt(extraN)+' beyond the catalog';
  setCount(msg);
  renderActive();
  syncTree();
}
function emptyHTML(){
  if(Q)return '<p class=empty><b>No matches for \\u201c'+esc(Q)+'\\u201d</b>Try a different term or clear your filters.</p>';
  return '<p class=empty><b>Nothing matches these filters</b>Remove a filter to see more parts.</p>';
}

function section(title,group,items,open){
  var rows=items.map(function(it){
    var on=MODE==='catalog'&&FILTERS[group]&&FILTERS[group][it.key]?' on':'';
    return '<li class="facet'+on+'" role=button tabindex=0 aria-pressed="'+(on?'true':'false')+'" data-group="'+group+'" data-key="'+esc(it.key)+'"><span class=fl>'+esc(labelFor(group,it.key))+'</span><span class=fn>'+nfmt(it.n)+'</span></li>';
  }).join('');
  var hid=('sec-'+group);
  return '<div class="sec'+(open?' open':'')+'"><button class=sechead aria-expanded="'+(open?'true':'false')+'" aria-controls="'+hid+'"><span class=caret aria-hidden=true>\\u25b8</span>'+esc(title)+'</button><ul class=facets id="'+hid+'">'+rows+'</ul></div>';
}
function renderTree(f){
  document.getElementById('tree').innerHTML=
    section('Type','type',f.types||[],true)
   +section('Sensing category','category',f.categories||[],true)
   +section('Interface','interface',f.interfaces||[],false)
   +section('Vendor','vendor',f.vendors||[],false);
}
function syncTree(){
  if(MODE!=='catalog')return;
  var nodes=document.querySelectorAll('.facet');
  for(var i=0;i<nodes.length;i++){
    var n=nodes[i],g=n.getAttribute('data-group'),k=n.getAttribute('data-key');
    var on=FILTERS[g]&&FILTERS[g][k];
    if(on){n.classList.add('on');n.setAttribute('aria-pressed','true');}else{n.classList.remove('on');n.setAttribute('aria-pressed','false');}
  }
}
function anyFilter(){
  if(Q)return true;
  if(MODE==='pool')return !!POOL.cat;
  return ['type','category','interface','vendor'].some(function(g){return Object.keys(FILTERS[g]).length;});
}
function renderActive(){
  var wrap=document.getElementById('active'),html='';
  if(MODE==='catalog'){
    ['type','category','interface','vendor'].forEach(function(g){
      Object.keys(FILTERS[g]).forEach(function(v){html+='<button class=achip data-group="'+g+'" data-key="'+esc(v)+'">'+esc(labelFor(g,v))+'<span class=x aria-hidden=true>\\u2715</span><span class=vh>remove filter</span></button>';});
    });
  }else if(POOL.cat){
    html+='<button class=achip data-poolcat=1>'+esc(POOL.cat)+'<span class=x aria-hidden=true>\\u2715</span><span class=vh>remove filter</span></button>';
  }
  if(Q)html+='<button class="achip qchip" data-clearq=1>\\u201c'+esc(Q)+'\\u201d<span class=x aria-hidden=true>\\u2715</span><span class=vh>clear search</span></button>';
  if(anyFilter())html+='<button class=clearall id=clearall type=button>Clear all</button>';
  wrap.innerHTML=html;
  wrap.style.display=html?'flex':'none';
}
function clearAll(){
  FILTERS={type:{},category:{},interface:{},vendor:{}};
  POOL.cat=null;Q='';searchIds=null;
  document.getElementById('q').value='';document.getElementById('qclear').classList.remove('on');
  if(MODE==='pool')poolSearch();else render();
}
function toggleFacet(group,key){
  if(FILTERS[group][key])delete FILTERS[group][key];else FILTERS[group][key]=1;
  render();
  closeDrawerIfMobile();
}

/* ---- pool mode ---- */
function ensurePool(cb){
  if(POOL.loaded){cb&&cb();return;}
  if(POOL.loading)return;
  POOL.loading=true;
  fetch('/api/pool/stats').then(function(r){return r.json();}).then(function(d){
    POOL.stats=d;POOL.loaded=true;POOL.loading=false;
    document.getElementById('ct-pool').textContent=nfmt(d.total||0);
    cb&&cb();
  }).catch(function(){POOL.loading=false;document.getElementById('tree').innerHTML='<p class=empty><b>Pool unavailable</b>Could not load the imported parts pool.</p>';});
}
function renderPoolTree(){
  var cats=(POOL.stats&&POOL.stats.by_category)||[];
  var rows=cats.map(function(c){
    var on=POOL.cat===c.category?' on':'';
    return '<li class="facet'+on+'" role=button tabindex=0 aria-pressed="'+(on?'true':'false')+'" data-poolkey="'+esc(c.category)+'"><span class=fl>'+esc(c.category||'(uncategorized)')+'</span><span class=fn>'+nfmt(c.n)+'</span></li>';
  }).join('');
  document.getElementById('tree').innerHTML='<div class="sec open"><button class=sechead aria-expanded=true aria-controls=sec-poolcat><span class=caret aria-hidden=true>\\u25b8</span>Category</button><ul class=facets id=sec-poolcat>'+(rows||'<li class=facet style="cursor:default">No categories</li>')+'</ul></div>';
}
function poolCardHTML(p){
  return '<article class=card role=button tabindex=0 data-pool="'+esc(p.mpn)+'" aria-label="'+esc(p.name||p.mpn)+', pool part">'
    +'<div class=cardhead><span class="badge b-pool">pool</span>'+(p.manufacturer?'<span class=vend>'+esc(p.manufacturer)+'</span>':'')+'</div>'
    +'<h3>'+esc(p.name||p.mpn)+'</h3>'
    +'<p class=sum>'+esc(p.mpn||'')+(p.category?' \\u00b7 '+esc(p.category):'')+'</p>'
    +'<div class=cardfoot><span class=promote>promotable</span>'+(p.price?'<span class=price>'+esc(p.price)+'</span>':'')+'</div></article>';
}
function renderPool(){
  var g=document.getElementById('grid');
  if(!POOL.loaded){g.setAttribute('aria-busy','true');g.innerHTML=skeletons(8);return;}
  g.setAttribute('aria-busy','false');
  var items=POOL.results;
  g.innerHTML=items.length?items.map(poolCardHTML).join(''):poolEmptyHTML();
  var tot=POOL.stats&&POOL.stats.total?POOL.stats.total:0;
  var shown=nfmt(items.length)+(items.length>=200?'+':'')+' of '+nfmt(tot)+' pool parts';
  setCount(shown);
  renderActive();
}
function poolEmptyHTML(){
  if(Q)return '<p class=empty><b>No pool parts match \\u201c'+esc(Q)+'\\u201d</b>Try an MPN, manufacturer or category.</p>';
  return '<p class=empty><b>No pool parts</b>Search or pick a category to explore the imported pool.</p>';
}
var ptimer;
function poolSearch(){
  var g=document.getElementById('grid');g.setAttribute('aria-busy','true');g.innerHTML=skeletons(8);
  var url='/api/pool/search?limit=200';
  if(Q)url+='&q='+encodeURIComponent(Q.toLowerCase());
  if(POOL.cat)url+='&category='+encodeURIComponent(POOL.cat);
  fetch(url).then(function(r){return r.json();}).then(function(d){
    POOL.results=d.results||[];POOL.byId={};POOL.results.forEach(function(p){POOL.byId[p.mpn]=p;});
    renderPool();syncPoolTree();
  }).catch(function(){POOL.results=[];renderPool();});
}
function syncPoolTree(){
  var nodes=document.querySelectorAll('[data-poolkey]');
  for(var i=0;i<nodes.length;i++){var n=nodes[i],on=n.getAttribute('data-poolkey')===POOL.cat;
    if(on){n.classList.add('on');n.setAttribute('aria-pressed','true');}else{n.classList.remove('on');n.setAttribute('aria-pressed','false');}}
}
function selectPoolCat(k){POOL.cat=(POOL.cat===k?null:k);poolSearch();closeDrawerIfMobile();}

/* ---- cogs mode ---- */
function cogMapsTo(c){return (c.maps_to||c.rec&&c.rec.maps_to)||[];}
function cogCardHTML(c){
  var maps=cogMapsTo(c);
  var sub=(c.category||'')+(c.version?' \\u00b7 v'+c.version:'');
  return '<article class=card role=button tabindex=0 data-cog="'+esc(c.id)+'" aria-label="'+esc(c.name||c.id)+', cog">'
    +'<div class=cardhead><span class="badge b-cog">cog</span>'+(sub?'<span class=vend>'+esc(sub)+'</span>':'')+'</div>'
    +'<h3>'+esc(c.name||c.id)+'</h3>'
    +(c.description?'<p class=sum>'+esc(trim(c.description,160))+'</p>':'')
    +'<div class=cardfoot>'+(maps.length?'<span class=maps>works with '+nfmt(maps.length)+' part'+(maps.length===1?'':'s')+'</span>':'<span class=promote>no part yet</span>')+'</div></article>';
}
function ensureCogs(cb){
  if(COGS.loaded){cb&&cb();return;}
  if(COGS.loading)return;
  COGS.loading=true;
  fetch('/api/cogs').then(function(r){return r.json();}).then(function(d){
    COGS.all=d.cogs||[];COGS.loaded=true;COGS.loading=false;COGS.byId={};
    COGS.all.forEach(function(c){COGS.byId[c.id]=c;});
    document.getElementById('ct-cogs').textContent=nfmt(COGS.all.length);
    cb&&cb();
  }).catch(function(){COGS.loading=false;});
}
function cogFilter(){
  if(!Q)return COGS.all||[];
  var ql=Q.toLowerCase();
  return (COGS.all||[]).filter(function(c){
    var hay=((c.id||'')+' '+(c.name||'')+' '+(c.category||'')+' '+(c.description||'')+' '+cogMapsTo(c).join(' ')).toLowerCase();
    return ql.split(/[^a-z0-9]+/).filter(Boolean).every(function(t){return hay.indexOf(t)>=0;});
  });
}
function renderCogs(){
  var g=document.getElementById('grid');
  if(!COGS.loaded){g.setAttribute('aria-busy','true');g.innerHTML=skeletons(6);return;}
  g.setAttribute('aria-busy','false');
  var items=cogFilter();COGS.results=items;
  g.innerHTML=items.length?items.map(cogCardHTML).join(''):'<p class=empty><b>No cogs match</b>Try a sensor name, radar, ecg, tof\\u2026</p>';
  setCount(nfmt(items.length)+' cog'+(items.length===1?'':'s'));
  renderActive();
}
function renderCogsTree(){
  var cats={};(COGS.all||[]).forEach(function(c){var k=c.category||'other';cats[k]=(cats[k]||0)+1;});
  var rows=Object.keys(cats).sort().map(function(k){
    return '<li class=facet style="cursor:default"><span class=fl>'+esc(k)+'</span><span class=fn>'+nfmt(cats[k])+'</span></li>';
  }).join('');
  document.getElementById('tree').innerHTML='<div class="sec open"><button class=sechead aria-expanded=true><span class=caret aria-hidden=true>\\u25b8</span>Cog category</button><ul class=facets>'+rows+'</ul></div>';
}

function setMode(mode){
  if(mode===MODE)return;
  MODE=mode;
  document.getElementById('mode-catalog').setAttribute('aria-pressed',String(mode==='catalog'));
  document.getElementById('mode-cogs').setAttribute('aria-pressed',String(mode==='cogs'));
  document.getElementById('mode-pool').setAttribute('aria-pressed',String(mode==='pool'));
  var ph='Search sensors, chips, projects\\u2026';
  if(mode==='pool')ph='Search the imported pool (MPN, maker, category)\\u2026';
  if(mode==='cogs')ph='Search cogs (radar, ecg, tof, a cog name)\\u2026';
  document.getElementById('q').placeholder=ph;
  if(mode==='pool'){
    ensurePool(function(){renderPoolTree();poolSearch();});
    if(!POOL.loaded){renderActive();}
  }else if(mode==='cogs'){
    ensureCogs(function(){renderCogsTree();renderCogs();});
    if(!COGS.loaded){renderActive();}
  }else{
    renderTree(window.__facets||{});render();
  }
}

/* ---- detail panel ---- */
function openPanel(){
  lastFocus=document.activeElement;
  var panel=document.getElementById('panel');
  panel.classList.add('open');panel.setAttribute('aria-hidden','false');
  document.getElementById('scrim').classList.add('on');
  document.getElementById('pclose').focus();
}
function openDetail(id){
  var body=document.getElementById('pbody');
  body.innerHTML='<p class=loading>Loading\\u2026</p>';openPanel();
  fetch('/api/parts/'+encodeURIComponent(id)).then(function(r){return r.json();}).then(function(p){
    if(!p||p.error){var m=BY_ID[id];body.innerHTML=m?detailHTML(m.rec):'<p>Not found.</p>';return;}
    body.innerHTML=detailHTML(p);body.scrollTop=0;document.getElementById('pclose').focus();
  }).catch(function(){var m=BY_ID[id];body.innerHTML=m?detailHTML(m.rec):'<p>Failed to load.</p>';});
}
function openCogDetail(id){
  var c=(COGS.byId&&COGS.byId[id])||null;
  if(!c){fetch('/api/cogs/'+encodeURIComponent(id)).then(function(r){return r.json();}).then(function(d){if(d&&!d.error){COGS.byId=COGS.byId||{};COGS.byId[id]=d;openCogDetail(id);}});openPanel();document.getElementById('pbody').innerHTML='<p class=loading>Loading\\u2026</p>';return;}
  openPanel();
  var maps=cogMapsTo(c);
  var h='<div class=phead><span class="badge b-cog">cog</span><span class=vend>'+esc((c.category||'')+(c.version?' \\u00b7 v'+c.version:''))+'</span></div>';
  h+='<h2 id=ptitle>'+esc(c.name||c.id)+'</h2>';
  h+='<p class=pid>'+esc(c.id)+'</p>';
  if(c.description)h+='<p class=psum>'+esc(c.description)+'</p>';
  var spec={};if(c.bind_port)spec['API port']=c.bind_port;if(c.store_id)spec['Base store id']=c.store_id;if(c.hardware_requirement&&c.hardware_requirement.length)spec['Hardware']=c.hardware_requirement.join(', ');if(c.binary)spec['Binary']=c.binary;
  h+=specTable(spec);
  if(maps.length)h+=xref('Works with these parts',maps,nameOf);
  else h+='<div class=phint>No catalog part maps to this cog yet.</div>';
  h+='<div class=note>Install on a Seed over MCP: <code>cog install '+esc(c.id)+'</code></div>';
  document.getElementById('pbody').innerHTML=h;document.getElementById('pbody').scrollTop=0;document.getElementById('pclose').focus();
}
function openPoolDetail(mpn){
  var p=POOL.byId[mpn];if(!p){return;}
  openPanel();
  var h='<div class=phead><span class="badge b-pool">pool</span>'+(p.manufacturer?'<span class=vend>'+esc(p.manufacturer)+'</span>':'')+'</div>';
  h+='<h2 id=ptitle>'+esc(p.name||p.mpn)+'</h2>';
  h+='<p class=pid>'+esc(p.mpn||'')+'</p>';
  var spec={};if(p.manufacturer)spec['Manufacturer']=p.manufacturer;if(p.category)spec['Category']=p.category;if(p.price)spec['Price']=p.price;
  h+=specTable(spec);
  if(p.datasheet)h+='<div class=links><a class=ext href="'+esc(p.datasheet)+'" target=_blank rel=noopener>Datasheet \\u2197</a></div>';
  h+='<div class=phint>Imported pool part. It can be promoted into the curated catalog via a <code>contribute</code>-scoped MCP call.</div>';
  document.getElementById('pbody').innerHTML=h;document.getElementById('pbody').scrollTop=0;document.getElementById('pclose').focus();
}
function closeDetail(){
  var panel=document.getElementById('panel');
  panel.classList.remove('open');panel.setAttribute('aria-hidden','true');
  document.getElementById('scrim').classList.remove('on');
  if(lastFocus&&lastFocus.focus){lastFocus.focus();}
}
function specTable(spec){
  if(!spec)return '';var ks=Object.keys(spec);if(!ks.length)return '';
  var rows=ks.map(function(k){return '<tr><th>'+esc(k)+'</th><td>'+esc(spec[k])+'</td></tr>';}).join('');
  return '<table class=spec>'+rows+'</table>';
}
function listBlock(label,arr,cls){
  if(!arr||!arr.length)return '';
  return '<div class="lb '+cls+'"><b>'+label+'</b><ul>'+arr.map(function(x){return '<li>'+esc(x)+'</li>';}).join('')+'</ul></div>';
}
function noteBlock(notes){
  if(!notes||!notes.length)return '';
  return notes.map(function(n){return '<div class=note>'+esc(n)+'</div>';}).join('');
}
function buyBlock(buy){
  if(!buy||!buy.length)return '';
  return '<div class=buys>'+buy.map(function(b){
    return '<a class=buy href="'+esc(b.url||'#')+'" target=_blank rel=noopener>'+esc(b.vendor||'Buy')+(b.price?' <span class=price>'+esc(b.price)+'</span>':'')+'</a>';
  }).join('')+'</div>';
}
function xref(label,ids,nameFn){
  if(!ids||!ids.length)return '';
  return '<div class=xref><b>'+label+'</b><div class=xlinks>'+ids.map(function(id){
    return '<span class=xlink role=button tabindex=0 data-open="'+esc(id)+'">'+esc(nameFn(id))+'</span>';
  }).join('')+'</div></div>';
}
function nameOf(id){var m=BY_ID[id];return m?m.name:id;}
function projectsUsing(id){return PROJECTS.filter(function(p){return (p.modules||[]).indexOf(id)>=0;}).map(function(p){return p.id;});}
function modulesWithChip(id){return MODULES.filter(function(m){return (m.rec.chips||[]).indexOf(id)>=0;}).map(function(m){return m.id;});}
function detailHTML(p){
  var meta=BY_ID[p.id]||{};
  var type=meta.type||(p.manufacturer?'chip':((p.difficulty||p.modules)?'project':'module'));
  var vend=vendorOf(p);
  var h='<div class=phead><span class="badge b-'+type+'">'+type+'</span>'+(vend?'<span class=vend>'+esc(vend)+'</span>':'')+'</div>';
  h+='<h2 id=ptitle>'+esc(p.name||p.id)+'</h2>';
  if(p.id)h+='<p class=pid>'+esc(p.id)+' \\u00b7 <a href="/part/'+encodeURIComponent(p.id)+'">full page (cog, firmware, research) \\u2197</a></p>';
  if(p.role)h+='<p class=role>'+esc(p.role)+'</p>';
  if(p.summary)h+='<p class=psum>'+esc(p.summary)+'</p>';
  var pills='';(meta.cats||[]).forEach(function(c){pills+='<span class="pill cat">'+esc(labelCat(c))+'</span>';});(meta.ifaces||[]).forEach(function(i){pills+='<span class=pill>'+esc(labelIf(i))+'</span>';});
  if(pills)h+='<div class=cardfoot style="margin:8px 0">'+pills+'</div>';
  h+=noteBlock(p.notes);
  h+=listBlock('Good for',p.good_for,'good');
  h+=listBlock('Not for',p.not_for,'bad');
  h+=specTable(p.spec);
  if(p.datasheet)h+='<div class=links><a class=ext href="'+esc(p.datasheet)+'" target=_blank rel=noopener>Datasheet \\u2197</a></div>';
  h+=buyBlock(p.buy);
  if(type==='module'){h+=xref('Chips on board',p.chips,nameOf);h+=xref('Projects using this',projectsUsing(p.id),nameOf);}
  if(type==='chip'){h+=xref('Modules with this chip',modulesWithChip(p.id),nameOf);}
  if(type==='project'){h+=xref('Modules',p.modules,nameOf);}
  if(p.pins&&p.pins.length)h+='<div class=xref><b>Pins</b><div class=tags>'+p.pins.map(function(x){return '<span class=tagp>'+esc(x)+'</span>';}).join('')+'</div></div>';
  return h;
}

/* ---- search ---- */
var stimer;
function onSearch(e){
  Q=e.target.value.trim();
  document.getElementById('qclear').classList.toggle('on',!!Q);
  clearTimeout(stimer);stimer=setTimeout(doSearch,200);
}
function doSearch(){
  if(MODE==='pool'){poolSearch();return;}
  if(MODE==='cogs'){renderCogs();return;}
  if(!Q){searchIds=null;EXTRA={cogs:[],pool:[],expanded:false};render();return;}
  fetch('/api/search?q='+encodeURIComponent(Q.toLowerCase())+'&limit=200').then(function(r){return r.json();}).then(function(d){
    searchIds={};(d.results||[]).forEach(function(r){searchIds[r.id]=1;});
    EXTRA={cogs:d.cogs||[],pool:d.pool||[],expanded:!!d.expanded};render();
  }).catch(function(){var ql=Q.toLowerCase();searchIds={};INDEX.forEach(function(m){if(m.hay.indexOf(ql)>=0)searchIds[m.id]=1;});EXTRA={cogs:[],pool:[],expanded:false};render();});
}
function clearSearch(){Q='';document.getElementById('q').value='';document.getElementById('qclear').classList.remove('on');EXTRA={cogs:[],pool:[],expanded:false};if(MODE==='pool'){poolSearch();}else if(MODE==='cogs'){renderCogs();}else{searchIds=null;render();}}

/* ---- drawer ---- */
function closeDrawerIfMobile(){if(window.matchMedia('(max-width:860px)').matches)setDrawer(false);}
function setDrawer(open){document.body.classList.toggle('drawer',open);document.getElementById('filtbtn').setAttribute('aria-expanded',String(open));}

/* ---- events (delegated) ---- */
document.addEventListener('click',function(e){
  var o=e.target.closest('[data-open]');
  if(o){e.stopPropagation();openDetail(o.getAttribute('data-open'));return;}
  var pc=e.target.closest('[data-poolkey]');
  if(pc){selectPoolCat(pc.getAttribute('data-poolkey'));return;}
  var f=e.target.closest('.facet[data-group]');
  if(f){toggleFacet(f.getAttribute('data-group'),f.getAttribute('data-key'));return;}
  if(e.target.closest('#clearall')){clearAll();return;}
  var a=e.target.closest('.achip');
  if(a){
    if(a.getAttribute('data-clearq')){clearSearch();}
    else if(a.getAttribute('data-poolcat')){POOL.cat=null;poolSearch();}
    else{toggleFacet(a.getAttribute('data-group'),a.getAttribute('data-key'));}
    return;
  }
  var sh=e.target.closest('.sechead');
  if(sh){var sec=sh.parentNode;sec.classList.toggle('open');sh.setAttribute('aria-expanded',String(sec.classList.contains('open')));return;}
  var ccard=e.target.closest('.card[data-cog]');
  if(ccard){openCogDetail(ccard.getAttribute('data-cog'));return;}
  var pcard=e.target.closest('.card[data-pool]');
  if(pcard){openPoolDetail(pcard.getAttribute('data-pool'));return;}
  var card=e.target.closest('.card[data-id]');
  if(card){openDetail(card.getAttribute('data-id'));return;}
});
document.addEventListener('keydown',function(e){
  if(e.key==='Escape'){closeDetail();if(document.body.classList.contains('drawer'))setDrawer(false);return;}
  if(e.key!=='Enter'&&e.key!==' ')return;
  var t=e.target;
  if(t.classList&&(t.classList.contains('facet')||t.classList.contains('xlink')||t.classList.contains('card'))){e.preventDefault();t.click();}
});
document.getElementById('q').addEventListener('input',onSearch);
document.getElementById('qclear').addEventListener('click',clearSearch);
document.getElementById('pclose').addEventListener('click',closeDetail);
document.getElementById('scrim').addEventListener('click',closeDetail);
document.getElementById('sideback').addEventListener('click',function(){setDrawer(false);});
document.getElementById('filtbtn').addEventListener('click',function(){setDrawer(!document.body.classList.contains('drawer'));});
document.getElementById('mode-catalog').addEventListener('click',function(){setMode('catalog');});
document.getElementById('mode-cogs').addEventListener('click',function(){setMode('cogs');});
document.getElementById('mode-pool').addEventListener('click',function(){setMode('pool');});

/* ---- boot ---- */
document.getElementById('grid').innerHTML=skeletons(8);
Promise.all([
  fetch('/api/facets').then(function(r){return r.json();}),
  fetch('/api/catalog.json').then(function(r){return r.json();})
]).then(function(res){
  window.__facets=res[0];
  renderTree(res[0]);
  loadCatalog(res[1]);
  render();
}).catch(function(){
  document.getElementById('grid').setAttribute('aria-busy','false');
  document.getElementById('grid').innerHTML='<p class=empty><b>Failed to load catalog</b>Check your connection and reload.</p>';
});
/* Prefetch pool + cog counts so the toggles show a number without switching. */
fetch('/api/pool/stats').then(function(r){return r.json();}).then(function(d){
  POOL.stats=d;POOL.loaded=true;document.getElementById('ct-pool').textContent=nfmt(d.total||0);
}).catch(function(){});
fetch('/api/cogs').then(function(r){return r.json();}).then(function(d){
  COGS.all=d.cogs||[];COGS.loaded=true;COGS.byId={};COGS.all.forEach(function(c){COGS.byId[c.id]=c;});
  document.getElementById('ct-cogs').textContent=nfmt(COGS.all.length);
}).catch(function(){});
</script>
</body></html>`;
}

// ---- server-rendered item-detail page helpers ----
function esc(s: any): string {
  return String(s == null ? "" : s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
function vendorOfSrv(r: any): string {
  return String(r.vendor || r.manufacturer || "").trim();
}
function specTableSrv(spec: any): string {
  if (!spec || typeof spec !== "object") return "";
  const ks = Object.keys(spec).filter((k) => spec[k] != null && spec[k] !== "");
  if (!ks.length) return "";
  return '<table class=spec>' + ks.map((k) => '<tr><th>' + esc(k) + '</th><td>' + esc(spec[k]) + '</td></tr>').join("") + '</table>';
}
function listBlockSrv(label: string, arr: any, cls: string): string {
  if (!Array.isArray(arr) || !arr.length) return "";
  return '<div class="lb ' + cls + '"><b>' + esc(label) + '</b><ul>' + arr.map((x) => '<li>' + esc(x) + '</li>').join("") + '</ul></div>';
}

function overviewTab(type: string, hash: string, p: any): string {
  const vend = vendorOfSrv(p);
  let h = '<div class=phead><span class="badge b-' + esc(type) + '">' + esc(type) + '</span>' + (vend ? '<span class=vend>' + esc(vend) + '</span>' : '') + '</div>';
  h += '<h1>' + esc(p.name || p.id) + '</h1>';
  h += '<p class=pid>' + esc(p.id || "") + '</p>';
  if (hash) h += '<p class=hash title="stable item hash">' + esc(hash) + '</p>';
  if (p.role) h += '<p class=role>' + esc(p.role) + '</p>';
  if (p.summary) h += '<p class=psum>' + esc(p.summary) + '</p>';
  if (Array.isArray(p.notes)) h += p.notes.map((n: any) => '<div class=note>' + esc(n) + '</div>').join("");
  h += listBlockSrv("Good for", p.good_for, "good");
  h += listBlockSrv("Not for", p.not_for, "bad");
  h += specTableSrv(p.spec);
  if (p.datasheet) h += '<div class=links><a class=ext href="' + esc(p.datasheet) + '" target=_blank rel=noopener>Datasheet ↗</a></div>';
  if (Array.isArray(p.buy) && p.buy.length) {
    h += '<div class=buys>' + p.buy.map((b: any) => '<a class=buy href="' + esc(b.url || "#") + '" target=_blank rel=noopener>' + esc(b.vendor || "Buy") + (b.price ? ' <span class=price>' + esc(b.price) + '</span>' : '') + '</a>').join("") + '</div>';
  }
  if (Array.isArray(p.chips) && p.chips.length) h += '<div class=xref><b>Chips on board</b><div class=tags>' + p.chips.map((x: any) => '<span class=tagp>' + esc(x) + '</span>').join("") + '</div></div>';
  if (Array.isArray(p.modules) && p.modules.length) h += '<div class=xref><b>Modules</b><div class=tags>' + p.modules.map((x: any) => '<span class=tagp>' + esc(x) + '</span>').join("") + '</div></div>';
  if (Array.isArray(p.pins) && p.pins.length) h += '<div class=xref><b>Pins</b><div class=tags>' + p.pins.map((x: any) => '<span class=tagp>' + esc(x) + '</span>').join("") + '</div></div>';
  return h;
}

function cogTab(cogs: any[]): string {
  if (!cogs.length) {
    return '<div class=cta><p class=ctatext>No cog maps to this part yet. A cog is the small reader that brings this sensor onto the mesh.</p>' +
      '<button class=btn id=createcog>Create a cog for this part</button><p class=msg id=cogmsg></p></div>';
  }
  return cogs.map((c) => {
    let h = '<div class=cogcard>';
    h += '<div class=phead><span class="badge b-cog">cog</span><span class=vend>' + esc(c.category || "") + (c.version ? ' · v' + esc(c.version) : '') + '</span></div>';
    h += '<h2>' + esc(c.name || c.id) + '</h2><p class=pid>' + esc(c.id) + '</p>';
    if (c.description) h += '<p class=psum>' + esc(c.description) + '</p>';
    const spec: any = {};
    if (c.bind_port) spec["API port"] = c.bind_port;
    if (c.store_id) spec["Base store id"] = c.store_id;
    if (Array.isArray(c.hardware_requirement) && c.hardware_requirement.length) spec["Hardware"] = c.hardware_requirement.join(", ");
    if (c.binary) spec["Binary"] = c.binary;
    h += specTableSrv(spec);
    h += '<div class=note>Install on a Seed over MCP: <code>cog install ' + esc(c.id) + '</code> (cog-dev / seed-mcp). The cog then serves its API on port ' + esc(c.bind_port || "?") + '.</div>';
    h += '</div>';
    return h;
  }).join("");
}

function firmwareTab(fw: any[]): string {
  if (!fw.length) return '<p class=ctatext>No firmware mapped to this part.</p>';
  return fw.map((f) => {
    let h = '<div class=cogcard>';
    h += '<div class=phead><span class="badge b-fw">firmware</span><span class=vend>' + esc(f.device || "") + (f.version ? ' · v' + esc(f.version) : '') + '</span></div>';
    h += '<h2>' + esc(f.name || f.id) + '</h2>';
    if (f.description) h += '<p class=psum>' + esc(f.description) + '</p>';
    const spec: any = {};
    if (f.device) spec["Device"] = f.device;
    if (f.repo) spec["Source"] = f.repo;
    h += specTableSrv(spec);
    h += '</div>';
    return h;
  }).join("");
}

function PART_NOT_FOUND(id: string): string {
  return '<!doctype html><meta charset=utf-8><title>Not found</title><body style="font-family:system-ui;padding:40px"><h1>Part not found</h1><p>No catalog part with id <code>' + esc(id) + '</code>.</p><p><a href="/">← Back to the explorer</a></p></body>';
}

function PART_PAGE(name: string, type: string, hash: string, part: any, cogs: any[], firmware: any[]): string {
  const hasCog = cogs.length > 0;
  const hasFw = firmware.length > 0;
  const ov = overviewTab(type, hash, part);
  const cg = cogTab(cogs);
  const fw = firmwareTab(firmware);
  const pid = String(part.id || "");
  return `<!doctype html><html lang=en><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<meta name=color-scheme content="light dark">
<title>${esc(part.name || pid)} — ${esc(name)}</title>
<style>
:root{--bg:#f4ede5;--card:#fffdfb;--ink:#20242b;--ink-soft:#4b515b;--grey:#6a6f78;--line:#e3d8cb;--line-soft:#ece3d8;--wl:#4f84d6;--wl-ink:#3a6bb8;--accent:#d97b2b;--green:#2f8b57;--pool:#7a5cc0;--noteb:#fdf4e6;--chip-bg:#f2ebe2;--focus:#2b6fd6;--shadow:rgba(60,45,30,.14);--r:12px;--r-pill:999px}
@media(prefers-color-scheme:dark){:root:not([data-theme=light]){--bg:#17191d;--card:#23272d;--ink:#e8eaed;--ink-soft:#c3c8cf;--grey:#9aa0a8;--line:#33373e;--line-soft:#2b2f35;--wl:#7aa6ec;--wl-ink:#9cc0f5;--accent:#e8a158;--green:#56b882;--pool:#a98bea;--noteb:#2a2519;--chip-bg:#2a2e34;--focus:#8ab4ff;--shadow:rgba(0,0,0,.45)}}
*{box-sizing:border-box}html,body{margin:0}
body{background:var(--bg);color:var(--ink);font:15px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif}
a{color:var(--wl-ink);text-decoration:none}a:hover{text-decoration:underline}
.wrap{max-width:820px;margin:0 auto;padding:16px 16px 72px}
.back{display:inline-block;color:var(--grey);font-size:13px;margin:8px 0 14px}
.main{background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:20px 22px;box-shadow:0 2px 12px var(--shadow)}
.phead{display:flex;align-items:center;gap:8px;margin-bottom:6px}
.badge{font-size:10px;font-weight:700;text-transform:uppercase;letter-spacing:.04em;padding:3px 8px;border-radius:var(--r-pill);color:#fff}
.b-module{background:var(--wl)}.b-chip{background:var(--accent)}.b-project{background:var(--green)}.b-cog{background:var(--pool)}.b-fw{background:#c0527a}
.vend{color:var(--grey);font-size:12px;margin-left:auto}
h1{margin:4px 0 2px;font-size:24px;font-weight:700;line-height:1.2}
h2{margin:2px 0 4px;font-size:18px}
.pid{font-size:12px;color:var(--grey);font-family:ui-monospace,SFMono-Regular,Menlo,monospace;margin:0 0 2px}
.hash{font-size:12px;color:var(--pool);font-family:ui-monospace,SFMono-Regular,Menlo,monospace;margin:0 0 8px}
.role{color:var(--grey);margin:0 0 10px;font-size:14px}
.psum{font-size:14px;margin:6px 0 10px;line-height:1.5}
.note{background:var(--noteb);border-left:3px solid var(--accent);padding:9px 12px;border-radius:0 6px 6px 0;margin:10px 0;font-size:13px}
.note code,.pid code{background:var(--chip-bg);padding:1px 6px;border-radius:5px;font-size:12px}
.lb{margin:12px 0;font-size:13px}.lb ul{margin:5px 0 0;padding-left:18px}.lb.good b{color:var(--green)}.lb.bad b{color:var(--accent)}
table.spec{width:100%;border-collapse:collapse;margin:14px 0;font-size:13px}
table.spec th{text-align:left;color:var(--grey);font-weight:600;padding:6px 12px 6px 0;vertical-align:top;white-space:nowrap;width:1%}
table.spec td{padding:6px 0;border-bottom:1px solid var(--line-soft)}
.links{margin:14px 0}.ext,.buy{display:inline-flex;align-items:center;gap:6px;padding:8px 14px;border-radius:var(--r-pill);font-size:13px;font-weight:600}
.ext{border:1px solid var(--wl);color:var(--wl-ink)}.buys{display:flex;flex-wrap:wrap;gap:8px;margin:14px 0}.buy{background:var(--green);color:#fff}.buy .price{color:#fff}
.xref{margin:16px 0}.xref>b{display:block;color:var(--grey);font-size:12px;text-transform:uppercase;letter-spacing:.04em;margin-bottom:8px}
.tags{display:flex;flex-wrap:wrap;gap:6px}.tagp{font-size:11px;padding:3px 9px;border:1px solid var(--line);border-radius:var(--r-pill);color:var(--grey)}
.tabs{display:flex;gap:4px;border-bottom:1px solid var(--line);margin:4px 0 18px}
.tab{background:none;border:none;border-bottom:2px solid transparent;color:var(--ink-soft);font:600 14px inherit;padding:10px 14px;cursor:pointer;font-family:inherit}
.tab[aria-selected=true]{color:var(--wl-ink);border-bottom-color:var(--wl)}
.tabpanel[hidden]{display:none}
.cogcard{border:1px solid var(--line);border-radius:var(--r);padding:14px 16px;margin-bottom:14px;background:var(--bg)}
.cta{text-align:center;padding:20px}.ctatext{color:var(--ink-soft);font-size:14px;margin:0 0 14px}
.btn{background:var(--wl);color:#fff;border:none;border-radius:var(--r-pill);padding:10px 18px;font:600 14px inherit;cursor:pointer;font-family:inherit}
.btn:disabled{opacity:.6;cursor:default}
.actions{display:flex;gap:10px;flex-wrap:wrap;margin:18px 0 0;padding-top:16px;border-top:1px solid var(--line-soft)}
.btn.ghost{background:none;color:var(--pool);border:1px solid color-mix(in srgb,var(--pool) 55%,var(--line))}
.msg{font-size:13px;color:var(--green);margin:10px 0 0;min-height:1em}
:focus-visible{outline:2px solid var(--focus);outline-offset:2px;border-radius:4px}
</style></head><body>
<div class=wrap>
<a class=back href="/">← ${esc(name)}</a>
<div class=main>
<div class=tabs role=tablist>
  <button class=tab id=t-overview role=tab aria-selected=true aria-controls=p-overview>Overview</button>
  <button class=tab id=t-cog role=tab aria-selected=false aria-controls=p-cog>Cog${hasCog ? " ✓" : ""}</button>
  ${hasFw ? '<button class=tab id=t-firmware role=tab aria-selected=false aria-controls=p-firmware>Firmware ✓</button>' : ''}
</div>
<div class=tabpanel id=p-overview role=tabpanel aria-labelledby=t-overview>${ov}</div>
<div class=tabpanel id=p-cog role=tabpanel aria-labelledby=t-cog hidden>${cg}</div>
${hasFw ? '<div class=tabpanel id=p-firmware role=tabpanel aria-labelledby=t-firmware hidden>' + fw + '</div>' : ''}
<div class=actions>
  <button class="btn ghost" id=research>Add to research</button>
  <span class=msg id=researchmsg></span>
</div>
</div>
</div>
<script>
var PART_ID=${JSON.stringify(pid)},PART_HASH=${JSON.stringify(hash)};
function sel(tab){
  var tabs=document.querySelectorAll('.tab');
  for(var i=0;i<tabs.length;i++){
    var t=tabs[i],on=t.id==='t-'+tab;
    t.setAttribute('aria-selected',on?'true':'false');
    var panel=document.getElementById('p-'+t.id.slice(2));
    if(panel){if(on){panel.removeAttribute('hidden');}else{panel.setAttribute('hidden','');}}
  }
}
var tabEls=document.querySelectorAll('.tab');
for(var i=0;i<tabEls.length;i++){(function(el){el.addEventListener('click',function(){sel(el.id.slice(2));});})(tabEls[i]);}
function post(url,body,btn,msgEl,okText){
  btn.disabled=true;
  fetch(url,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)})
    .then(function(r){return r.json();})
    .then(function(d){msgEl.textContent=d.ok?okText:(d.error||'Failed');if(!d.ok)btn.disabled=false;})
    .catch(function(){msgEl.textContent='Failed';btn.disabled=false;});
}
var rb=document.getElementById('research');
rb.addEventListener('click',function(){post('/api/research',{ref_type:'part',ref_id:PART_ID,ref_hash:PART_HASH},rb,document.getElementById('researchmsg'),'Added to the research queue.');});
var cc=document.getElementById('createcog');
if(cc){cc.addEventListener('click',function(){post('/api/cog-request',{part_id:PART_ID,part_hash:PART_HASH},cc,document.getElementById('cogmsg'),'Cog request recorded.');});}
</script>
</body></html>`;
}
