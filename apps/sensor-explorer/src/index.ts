// WeftOS Sensor Explorer — Cloudflare Worker.
// MCP agent surface at POST /mcp (API-key auth) + public read REST + a rich tree-browse UI at GET /.

import { Hono } from "hono";
import { cors } from "hono/cors";
import { handleRpc, type Env } from "./mcp";

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
<title>${name}</title>
<style>
:root{--bg:#f6efe9;--card:#fff;--ink:#23262b;--grey:#7c818a;--line:#e7ddd3;--wl:#6c9ce8;--accent:#e08a3a;--green:#3f9d63;--side:#efe6dc;--noteb:#fff6e9}
@media(prefers-color-scheme:dark){:root:not([data-theme=light]){--bg:#1b1d20;--card:#24272c;--ink:#e6e8ea;--grey:#9aa0a8;--line:#34383e;--side:#202327;--noteb:#2a2620}}
:root[data-theme=dark]{--bg:#1b1d20;--card:#24272c;--ink:#e6e8ea;--grey:#9aa0a8;--line:#34383e;--side:#202327;--noteb:#2a2620}
*{box-sizing:border-box}
html,body{margin:0}
body{background:var(--bg);color:var(--ink);font:15px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}
a{color:var(--wl);text-decoration:none}a:hover{text-decoration:underline}
header{padding:18px 20px 10px}
h1{margin:0;font-size:23px;letter-spacing:-.01em}
.sub{color:var(--grey);margin:3px 0 0;font-size:13px}
.bar{position:sticky;top:0;z-index:20;background:var(--bg);border-bottom:1px solid var(--line);padding:10px 20px;display:flex;gap:10px;align-items:center;flex-wrap:wrap}
#q{flex:1;min-width:200px;padding:9px 13px;border:1px solid var(--line);border-radius:20px;background:var(--card);color:var(--ink);font-size:14px}
#q:focus{outline:none;border-color:var(--wl)}
#count{color:var(--grey);font-size:13px;white-space:nowrap}
.filtbtn{display:none;padding:9px 14px;border:1px solid var(--line);border-radius:20px;background:var(--card);color:var(--ink);font-weight:600;cursor:pointer}
#active{display:none;gap:7px;flex-wrap:wrap;padding:8px 20px 0}
.achip{background:var(--wl);color:#fff;border-radius:14px;padding:4px 10px;font-size:12px;cursor:pointer;user-select:none}
.achip.qchip{background:var(--accent)}
.wrap{display:flex;align-items:flex-start;gap:0}
.side{width:260px;flex:none;padding:14px 14px 40px;border-right:1px solid var(--line);background:var(--side);position:sticky;top:57px;max-height:calc(100vh - 57px);overflow:auto}
.sec{margin-bottom:6px}
.sechead{width:100%;text-align:left;background:none;border:none;color:var(--ink);font-weight:700;font-size:13px;letter-spacing:.02em;text-transform:uppercase;padding:8px 4px;cursor:pointer;display:flex;align-items:center;gap:7px}
.caret{display:inline-block;transition:transform .15s;color:var(--grey);font-size:11px}
.sec.open .caret{transform:rotate(90deg)}
.facets{list-style:none;margin:0 0 4px;padding:0;display:none}
.sec.open .facets{display:block}
.facet{display:flex;justify-content:space-between;align-items:center;gap:8px;padding:5px 9px;border-radius:7px;cursor:pointer;font-size:13.5px}
.facet:hover{background:var(--bg)}
.facet.on{background:var(--wl);color:#fff}
.facet.on .fn{color:#fff}
.fl{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.fn{color:var(--grey);font-size:12px;font-variant-numeric:tabular-nums}
.main{flex:1;min-width:0;padding:16px 20px 60px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(248px,1fr));gap:13px}
.card{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:13px 15px;cursor:pointer;transition:border-color .12s,transform .12s}
.card:hover{border-color:var(--wl);transform:translateY(-1px)}
.cardhead{display:flex;align-items:center;gap:8px;margin-bottom:5px}
.badge{font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.03em;padding:2px 8px;border-radius:10px;color:#fff}
.b-module{background:var(--wl)}.b-chip{background:var(--accent)}.b-project{background:var(--green)}
.vend{color:var(--grey);font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.card h3{margin:0 0 4px;font-size:15.5px;line-height:1.25}
.sum{margin:0 0 9px;color:var(--ink);font-size:13px;opacity:.85}
.pills{display:flex;flex-wrap:wrap;gap:5px}
.pill{font-size:11px;padding:2px 8px;border-radius:11px;border:1px solid var(--line);color:var(--grey);background:var(--bg)}
.pill.cat{border-color:var(--wl);color:var(--wl)}
.pill.if{border-color:var(--grey)}
.empty{color:var(--grey);padding:30px 4px}
/* detail panel */
#scrim{position:fixed;inset:0;background:rgba(0,0,0,.4);opacity:0;pointer-events:none;transition:opacity .18s;z-index:40}
#scrim.on{opacity:1;pointer-events:auto}
#panel{position:fixed;top:0;right:0;height:100%;width:440px;max-width:94vw;background:var(--card);border-left:1px solid var(--line);transform:translateX(100%);transition:transform .22s;z-index:50;display:flex;flex-direction:column}
#panel.open{transform:translateX(0)}
.ptop{display:flex;justify-content:flex-end;padding:10px 12px 0}
.pclose{background:none;border:1px solid var(--line);color:var(--ink);border-radius:50%;width:32px;height:32px;font-size:18px;cursor:pointer;line-height:1}
#pbody{padding:6px 22px 40px;overflow:auto}
.phead{display:flex;align-items:center;gap:8px;margin:4px 0 6px}
#pbody h2{margin:2px 0 4px;font-size:21px;line-height:1.2}
.role{color:var(--grey);margin:0 0 8px;font-size:13px}
.note{background:var(--noteb);border-left:3px solid var(--accent);padding:8px 11px;border-radius:0 6px 6px 0;margin:7px 0;font-size:13px}
.lb{margin:10px 0;font-size:13.5px}.lb ul{margin:4px 0 0;padding-left:18px}.lb.good b{color:var(--green)}.lb.bad b{color:var(--accent)}
table.spec{width:100%;border-collapse:collapse;margin:12px 0;font-size:13px}
table.spec th{text-align:left;color:var(--grey);font-weight:600;padding:4px 10px 4px 0;vertical-align:top;white-space:nowrap;width:1%}
table.spec td{padding:4px 0;border-bottom:1px solid var(--line)}
.links{margin:12px 0}.ext{display:inline-block;padding:7px 13px;border:1px solid var(--wl);border-radius:18px;font-size:13px}
.buys{display:flex;flex-wrap:wrap;gap:8px;margin:12px 0}
.buy{display:inline-flex;gap:7px;align-items:center;background:var(--green);color:#fff;padding:7px 13px;border-radius:18px;font-size:13px;font-weight:600}
.buy:hover{text-decoration:none;opacity:.9}.price{font-weight:800}
.xref{margin:14px 0}.xref b{display:block;color:var(--grey);font-size:12px;text-transform:uppercase;letter-spacing:.03em;margin-bottom:6px}
.xlinks{display:flex;flex-wrap:wrap;gap:6px}
.xlink{display:inline-block;padding:5px 11px;border:1px solid var(--line);border-radius:14px;font-size:13px;cursor:pointer;background:var(--bg)}
.xlink:hover{border-color:var(--wl);text-decoration:none}
.tags{display:flex;flex-wrap:wrap;gap:5px}.tagp{font-size:11px;padding:2px 8px;border:1px solid var(--line);border-radius:10px;color:var(--grey)}
.loading{color:var(--grey)}
@media(max-width:820px){
  .side{position:fixed;top:0;left:0;height:100%;z-index:60;transform:translateX(-100%);transition:transform .2s;max-height:100%;width:270px;box-shadow:2px 0 18px rgba(0,0,0,.25)}
  body.drawer .side{transform:translateX(0)}
  .filtbtn{display:inline-block}
  #panel{width:100%}
}
</style></head><body>
<header>
  <h1>${name}</h1>
  <p class=sub>A browsable, contributable hardware catalog with an MCP agent surface. <a href="/api/facets">facets</a> · <a href="/api/catalog.json">catalog.json</a> · <code>POST /mcp</code></p>
</header>
<div class=bar>
  <button class=filtbtn id=filtbtn>Filters</button>
  <input id=q type=search placeholder="Search sensors, chips, projects…" autocomplete=off>
  <span id=count></span>
</div>
<div id=active></div>
<div class=wrap>
  <aside class=side id=side><nav id=tree><p class=loading>Loading…</p></nav></aside>
  <main class=main><div class=grid id=grid><p class=loading>Loading catalog…</p></div></main>
</div>
<div id=scrim></div>
<section id=panel aria-hidden=true>
  <div class=ptop><button class=pclose id=pclose aria-label=Close>×</button></div>
  <div id=pbody></div>
</section>
<script>
var CAT_RULES=${CATS};
var IF_RULES=${IFS};
var INDEX=[],BY_ID={},PROJECTS=[],MODULES=[];
var FILTERS={type:{},category:{},interface:{},vendor:{}};
var Q='',searchIds=null;

function esc(s){s=(s==null?'':String(s));return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');}
function trim(s,n){s=String(s);return s.length>n?s.slice(0,n-1)+'\\u2026':s;}
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
function keyOf(m){return m.type+':'+m.id;}

function loadCatalog(cat){
  [['module',cat.modules||[]],['chip',cat.chips||[]],['project',cat.projects||[]]].forEach(function(g){
    var type=g[0];
    g[1].forEach(function(r){
      var m={rec:r,id:r.id,type:type,name:r.name||r.id,vendor:vendorOf(r),cats:deriveCats(r),ifaces:deriveIfaces(r),summary:r.summary||'',hay:hayOf(r)};
      INDEX.push(m);BY_ID[type+':'+r.id]=m;if(!BY_ID[r.id])BY_ID[r.id]=m;
    });
  });
  MODULES=INDEX.filter(function(m){return m.type==='module';});
  PROJECTS=cat.projects||[];
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

function cardHTML(m){
  var pills='';
  m.cats.forEach(function(c){pills+='<span class="pill cat">'+esc(c)+'</span>';});
  m.ifaces.forEach(function(i){pills+='<span class="pill if">'+esc(i)+'</span>';});
  return '<article class=card data-id="'+esc(m.id)+'" data-key="'+esc(keyOf(m))+'">'
    +'<div class=cardhead><span class="badge b-'+m.type+'">'+m.type+'</span>'+(m.vendor?'<span class=vend>'+esc(m.vendor)+'</span>':'')+'</div>'
    +'<h3>'+esc(m.name)+'</h3>'
    +(m.summary?'<p class=sum>'+esc(trim(m.summary,150))+'</p>':'')
    +'<div class=pills>'+pills+'</div></article>';
}

function render(){
  var items=INDEX.filter(passes);
  items.sort(function(a,b){return a.type.localeCompare(b.type)||a.name.localeCompare(b.name);});
  var g=document.getElementById('grid');
  g.innerHTML=items.length?items.map(cardHTML).join(''):'<p class=empty>Nothing matches these filters.</p>';
  document.getElementById('count').textContent=items.length+' result'+(items.length===1?'':'s');
  renderActive();
  syncTree();
}

function section(title,group,items,open){
  var rows=items.map(function(it){
    return '<li class=facet data-group="'+group+'" data-key="'+esc(it.key)+'"><span class=fl>'+esc(it.key)+'</span><span class=fn>'+it.n+'</span></li>';
  }).join('');
  return '<div class="sec'+(open?' open':'')+'"><button class=sechead><span class=caret>\\u25b8</span>'+esc(title)+'</button><ul class=facets>'+rows+'</ul></div>';
}
function renderTree(f){
  document.getElementById('tree').innerHTML=
    section('Type','type',f.types||[],true)
   +section('Sensing category','category',f.categories||[],true)
   +section('Interface','interface',f.interfaces||[],false)
   +section('Vendor','vendor',f.vendors||[],false);
}
function syncTree(){
  var nodes=document.querySelectorAll('.facet');
  for(var i=0;i<nodes.length;i++){
    var n=nodes[i],g=n.getAttribute('data-group'),k=n.getAttribute('data-key');
    if(FILTERS[g]&&FILTERS[g][k])n.classList.add('on');else n.classList.remove('on');
  }
}
function renderActive(){
  var wrap=document.getElementById('active'),html='';
  ['type','category','interface','vendor'].forEach(function(g){
    Object.keys(FILTERS[g]).forEach(function(v){html+='<span class=achip data-group="'+g+'" data-key="'+esc(v)+'">'+esc(v)+' \\u2715</span>';});
  });
  if(Q)html+='<span class="achip qchip" data-clearq=1>\\u201c'+esc(Q)+'\\u201d \\u2715</span>';
  wrap.innerHTML=html;
  wrap.style.display=html?'flex':'none';
}

function toggleFacet(group,key){
  if(FILTERS[group][key])delete FILTERS[group][key];else FILTERS[group][key]=1;
  render();
}

// ---- detail panel ----
function openDetail(id){
  var panel=document.getElementById('panel'),body=document.getElementById('pbody');
  body.innerHTML='<p class=loading>Loading\\u2026</p>';
  panel.classList.add('open');panel.setAttribute('aria-hidden','false');
  document.getElementById('scrim').classList.add('on');
  fetch('/api/parts/'+encodeURIComponent(id)).then(function(r){return r.json();}).then(function(p){
    if(!p||p.error){var m=BY_ID[id];if(m){body.innerHTML=detailHTML(m.rec);return;}body.innerHTML='<p>Not found.</p>';return;}
    body.innerHTML=detailHTML(p);body.scrollTop=0;
  }).catch(function(){var m=BY_ID[id];body.innerHTML=m?detailHTML(m.rec):'<p>Failed to load.</p>';});
}
function closeDetail(){
  document.getElementById('panel').classList.remove('open');
  document.getElementById('panel').setAttribute('aria-hidden','true');
  document.getElementById('scrim').classList.remove('on');
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
    return '<a class=xlink data-open="'+esc(id)+'">'+esc(nameFn(id))+'</a>';
  }).join('')+'</div></div>';
}
function nameOf(id){var m=BY_ID[id];return m?m.name:id;}
function projectsUsing(id){
  return PROJECTS.filter(function(p){return (p.modules||[]).indexOf(id)>=0;}).map(function(p){return p.id;});
}
function modulesWithChip(id){
  return MODULES.filter(function(m){return (m.rec.chips||[]).indexOf(id)>=0;}).map(function(m){return m.id;});
}
function detailHTML(p){
  var meta=BY_ID[p.id]||{};
  var type=meta.type||(p.manufacturer?'chip':((p.difficulty||p.modules)?'project':'module'));
  var vend=vendorOf(p);
  var h='<div class=phead><span class="badge b-'+type+'">'+type+'</span>'+(vend?'<span class=vend>'+esc(vend)+'</span>':'')+'</div>';
  h+='<h2>'+esc(p.name||p.id)+'</h2>';
  if(p.role)h+='<p class=role>'+esc(p.role)+'</p>';
  if(p.summary)h+='<p class=sum style="font-size:14px;opacity:1">'+esc(p.summary)+'</p>';
  var pills='';(meta.cats||[]).forEach(function(c){pills+='<span class="pill cat">'+esc(c)+'</span>';});(meta.ifaces||[]).forEach(function(i){pills+='<span class="pill if">'+esc(i)+'</span>';});
  if(pills)h+='<div class=pills style="margin:8px 0">'+pills+'</div>';
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

// ---- search ----
var stimer;
function onSearch(e){Q=e.target.value.trim();clearTimeout(stimer);stimer=setTimeout(doSearch,180);}
function doSearch(){
  if(!Q){searchIds=null;render();return;}
  fetch('/api/search?q='+encodeURIComponent(Q.toLowerCase())+'&limit=200').then(function(r){return r.json();}).then(function(d){
    searchIds={};(d.results||[]).forEach(function(r){searchIds[r.id]=1;});render();
  }).catch(function(){var ql=Q.toLowerCase();searchIds={};INDEX.forEach(function(m){if(m.hay.indexOf(ql)>=0)searchIds[m.id]=1;});render();});
}

// ---- events (delegated) ----
document.addEventListener('click',function(e){
  var o=e.target.closest('[data-open]');
  if(o){e.stopPropagation();openDetail(o.getAttribute('data-open'));return;}
  var f=e.target.closest('.facet');
  if(f){toggleFacet(f.getAttribute('data-group'),f.getAttribute('data-key'));return;}
  var a=e.target.closest('.achip');
  if(a){if(a.getAttribute('data-clearq')){Q='';document.getElementById('q').value='';searchIds=null;render();}else{toggleFacet(a.getAttribute('data-group'),a.getAttribute('data-key'));}return;}
  var sh=e.target.closest('.sechead');
  if(sh){sh.parentNode.classList.toggle('open');return;}
  var card=e.target.closest('.card');
  if(card){openDetail(card.getAttribute('data-id'));return;}
});
document.getElementById('q').addEventListener('input',onSearch);
document.getElementById('pclose').addEventListener('click',closeDetail);
document.getElementById('scrim').addEventListener('click',closeDetail);
document.getElementById('filtbtn').addEventListener('click',function(){document.body.classList.toggle('drawer');});
document.addEventListener('keydown',function(e){if(e.key==='Escape')closeDetail();});

// ---- boot ----
Promise.all([
  fetch('/api/facets').then(function(r){return r.json();}),
  fetch('/api/catalog.json').then(function(r){return r.json();})
]).then(function(res){
  renderTree(res[0]);
  loadCatalog(res[1]);
  render();
}).catch(function(){
  document.getElementById('grid').innerHTML='<p class=empty>Failed to load catalog.</p>';
});
</script>
</body></html>`;
}
