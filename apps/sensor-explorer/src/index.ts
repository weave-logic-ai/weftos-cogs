// WeftOS Sensor Explorer — Cloudflare Worker.
// MCP agent surface at POST /mcp (API-key auth) + public read REST + a rich tree-browse UI at GET /.

import { Hono } from "hono";
import { cors } from "hono/cors";
import { handleRpc, type Env } from "./mcp";
import { catalogRelease, expandedSearch, searchCogs } from "./search";
import { activityFor, pulse, recordEvent, recordLook, type Activity } from "./activity";
import { guideFor } from "./guides";

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

// cogs.weavelogic.ai is a Worker custom domain. Always Use HTTPS is a zone
// setting this login cannot change, so the worker sends browsers to https.
app.use("*", async (c, next) => {
  const url = new URL(c.req.url);
  const forwarded = (c.req.header("x-forwarded-proto") || "").split(",")[0].trim().toLowerCase();
  let visitor = "";
  try {
    visitor = String(JSON.parse(c.req.header("cf-visitor") || "{}").scheme || "");
  } catch {
    visitor = "";
  }
  const scheme = forwarded || visitor || url.protocol.replace(":", "");
  const host = (c.req.header("host") || url.hostname).split(":")[0].toLowerCase();
  if (host === "cogs.weavelogic.ai" && scheme === "http") {
    url.protocol = "https:";
    url.hostname = host;
    const status = c.req.method === "GET" || c.req.method === "HEAD" ? 301 : 308;
    return c.redirect(url.toString(), status);
  }
  await next();
  if (host === "cogs.weavelogic.ai" && scheme === "https") {
    c.res.headers.set("Strict-Transport-Security", "max-age=31536000");
  }
});

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
  const id = c.req.param("id");
  const row = await c.env.DB.prepare("SELECT data FROM parts WHERE id=?").bind(id).first<{ data: string }>();
  if (!row) return c.json({ error: "not found" }, 404);
  await recordLook(c.env.DB, id, "view");
  const rec = JSON.parse(row.data);
  if (guideFor(id)) rec.guide_url = "/api/guides/" + encodeURIComponent(id);
  return c.json(rec);
});

// The JSON a cog serves at GET /guide, so the browser popup can render it without a Seed.
app.get("/api/guides/:id", (c) => {
  const id = c.req.param("id");
  const guide = guideFor(id);
  if (!guide) return c.json({ error: "no guide" }, 404);
  return c.json(guide);
});

app.get("/api/activity", async (c) => {
  const ref = (c.req.query("ref") || "").trim();
  if (!ref) return c.json({ error: "ref required" }, 400);
  return c.json(await activityFor(c.env.DB, ref));
});

app.get("/api/pulse", async (c) => c.json(await pulse(c.env.DB)));

// ---- cog registry ----
// Folder of each cog in the public repo weave-logic-ai/weftos-cogs, branch main.
// sensor-ota-push is in the catalog and has no folder in that repo.
const COG_GIT_REPO = "https://github.com/weave-logic-ai/weftos-cogs";
const COG_GIT_ABSENT = new Set(["sensor-ota-push"]);

function cogGitUrl(c: { id?: string; git?: string } | null | undefined): string {
  const explicit = c && typeof c.git === "string" ? c.git : "";
  if (explicit.startsWith("https://github.com/")) return explicit;
  const id = String((c && c.id) || "");
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(id)) return "";
  if (COG_GIT_ABSENT.has(id)) return "";
  return COG_GIT_REPO + "/tree/main/src/cogs/" + id;
}

function cogRow(r: any) {
  const row = { ...r, hardware: safeJson(r.hardware), maps_to: safeJson(r.maps_to) };
  if (!row.git) row.git = cogGitUrl(row);
  if (guideFor(String(row.id || ""))) row.has_guide = true;
  return row;
}

// Canonical cog targets ⇄ COG-008 registry artifact keys. The download endpoint accepts either
// spelling; stored rows and the public API use the canonical armv7/aarch64/x86_64.
const TARGET_ALIAS: Record<string, string> = {
  arm: "armv7", armv7: "armv7", aarch64: "aarch64", arm64: "aarch64", x86_64: "x86_64", amd64: "x86_64",
};
const TARGET_REGKEY: Record<string, string> = { armv7: "arm", aarch64: "arm64", x86_64: "x86_64" };

function toHex(buf: ArrayBuffer): string {
  const b = new Uint8Array(buf);
  let s = "";
  for (let i = 0; i < b.length; i++) s += b[i].toString(16).padStart(2, "0");
  return s;
}
function shortHex(s: string, head = 8, tail = 8): string {
  s = String(s || "");
  return s.length <= head + tail + 1 ? s : s.slice(0, head) + "…" + s.slice(-tail);
}
function fmtBytes(n: number): string {
  n = Number(n) || 0;
  return n >= 1024 * 1024 ? (n / 1048576).toFixed(1) + " MB" : n >= 1024 ? (n / 1024).toFixed(0) + " KB" : n + " B";
}

// Public artifact list for a cog: one entry per built target, with a download URL + signed facts.
async function artifactsFor(env: Env, cogId: string) {
  const { results } = await env.DB.prepare(
    "SELECT version,target,r2_key,size,sha256,sig,signer_pubkey FROM cog_artifacts WHERE cog_id=? ORDER BY target"
  ).bind(cogId).all();
  return (results as any[]).map((a) => ({
    target: a.target,
    version: a.version,
    url: `/api/cogs/${encodeURIComponent(cogId)}/download?target=${a.target}`,
    size: a.size,
    sha256: a.sha256,
    sig: a.sig,
    signer_pubkey: a.signer_pubkey,
    filename: String(a.r2_key).split("/").pop(),
  }));
}
app.get("/api/cogs", async (c) => {
  const q = c.req.query("q");
  if (q) {
    const cogs = await searchCogs(c.env.DB, q, 100);
    return c.json({ count: cogs.length, cogs });
  }
  const { results } = await c.env.DB.prepare(
    "SELECT id,name,category,version,description,store_id,hardware,bind_port,maps_to,sensor_id,sensor_name,hash FROM cogs ORDER BY sensor_name IS NULL, sensor_name, name"
  ).all();
  const cogs = (results as any[]).map(cogRow);
  return c.json({ count: cogs.length, cogs });
});
app.get("/api/cogs/:id", async (c) => {
  const id = c.req.param("id");
  const row = await c.env.DB.prepare("SELECT data FROM cogs WHERE id=?").bind(id).first<{ data: string }>();
  if (!row) return c.json({ error: "not found" }, 404);
  const rec = JSON.parse(row.data);
  if (!rec.git) rec.git = cogGitUrl(rec);
  if (guideFor(id)) rec.guide_url = "/api/guides/" + encodeURIComponent(id);
  const arts = await artifactsFor(c.env, id);
  if (arts.length) rec.artifacts = arts;
  await recordLook(c.env.DB, id, "view");
  return c.json(rec);
});

// Signed binary download. Streams from R2 when the ASSETS bucket is bound, else serves the inline
// bytes fallback. The sha256 / Ed25519 sig / signer pubkey ride along in headers so the caller
// (weft-cog-repo, the cogrepo cog) can verify before trusting the bytes.
app.get("/api/cogs/:id/download", async (c) => {
  const id = c.req.param("id");
  const target = TARGET_ALIAS[(c.req.query("target") || "").toLowerCase()];
  if (!target) return c.json({ error: "target required: armv7 | aarch64 | x86_64" }, 400);
  const row = await c.env.DB.prepare(
    "SELECT version,r2_key,size,sha256,sig,signer_pubkey FROM cog_artifacts WHERE cog_id=? AND target=? ORDER BY version DESC LIMIT 1"
  ).bind(id, target).first<any>();
  if (!row) return c.json({ error: "no artifact for this cog/target" }, 404);
  const filename = String(row.r2_key).split("/").pop() || `cog-${id}-${target}`;
  const headers: Record<string, string> = {
    "content-type": "application/octet-stream",
    "content-disposition": `attachment; filename="${filename}"`,
    "x-cog-id": id,
    "x-cog-target": target,
    "x-cog-version": row.version,
    "x-cog-size": String(row.size),
    "x-cog-sha256": row.sha256,
    "x-cog-sig": row.sig,
    "x-cog-signer": row.signer_pubkey,
    "cache-control": "public, max-age=3600",
  };
  // Prefer R2 when bound.
  if (c.env.ASSETS) {
    const obj = await c.env.ASSETS.get(row.r2_key);
    if (obj) {
      headers["etag"] = obj.httpEtag;
      return new Response(obj.body, { headers });
    }
  }
  // Fallback: inline bytes stored in D1 (used while R2 is unbound).
  const blob = await c.env.DB.prepare(
    "SELECT bytes FROM cog_artifacts WHERE cog_id=? AND target=? AND version=?"
  ).bind(id, target, row.version).first<{ bytes: ArrayBuffer | Uint8Array | number[] | null }>();
  if (!blob || !blob.bytes) return c.json({ error: "artifact bytes not uploaded yet" }, 404);
  const b: any = blob.bytes;
  const body: BodyInit = b instanceof ArrayBuffer || ArrayBuffer.isView(b) ? b : new Uint8Array(b);
  return new Response(body, { headers });
});

// Upload the actual binary for a registered artifact row (admin). The body is re-hashed and
// REFUSED unless the sha256 matches the row, so a truncated or wrong upload can never land.
app.put("/api/cogs/:id/artifact", async (c) => {
  const scope = await resolveScope(c.req.raw, c.env);
  if (scope !== "admin") return c.json({ error: "admin scope required" }, 403);
  const id = c.req.param("id");
  const target = TARGET_ALIAS[(c.req.query("target") || "").toLowerCase()];
  if (!target) return c.json({ error: "target required: armv7 | aarch64 | x86_64" }, 400);
  const row = await c.env.DB.prepare(
    "SELECT version,r2_key,sha256 FROM cog_artifacts WHERE cog_id=? AND target=? ORDER BY version DESC LIMIT 1"
  ).bind(id, target).first<any>();
  if (!row) return c.json({ error: "register the artifact row first (cog_artifacts)" }, 404);
  const buf = await c.req.arrayBuffer();
  const got = toHex(await crypto.subtle.digest("SHA-256", buf));
  if (got !== row.sha256) return c.json({ error: "sha256 mismatch", expected: row.sha256, got, size: buf.byteLength }, 422);
  if (c.env.ASSETS) await c.env.ASSETS.put(row.r2_key, buf);
  await c.env.DB.prepare(
    "UPDATE cog_artifacts SET bytes=?, size=? WHERE cog_id=? AND target=? AND version=?"
  ).bind(c.env.ASSETS ? null : buf, buf.byteLength, id, target, row.version).run();
  return c.json({ ok: true, cog_id: id, target, version: row.version, size: buf.byteLength, sha256: got, stored: c.env.ASSETS ? "r2" : "d1" });
});

// COG-008 signed registry, rebuilt from cog_artifacts + cog_versions so a WeaveLogic cog-source /
// the on-device cogrepo cog can install from it (weft-cog-repo signed-install contract).
app.get("/registry.json", async (c) => {
  const today = new Date().toISOString().slice(0, 10);
  const arts = (await c.env.DB.prepare(
    "SELECT a.cog_id,a.version,a.target,a.r2_key,a.size,a.sha256,a.sig,a.manifest_key " +
    "FROM cog_artifacts a JOIN cog_versions v ON v.cog_id=a.cog_id AND v.version=a.version " +
    "WHERE v.yanked=0 ORDER BY a.cog_id,a.target"
  ).all()).results as any[];
  if (!arts.length) return c.json({ schema: 1, repo: "weavelogic", updated: today, cogs: [] });
  const ids = [...new Set(arts.map((a) => a.cog_id))];
  const metaRows = (await c.env.DB.prepare(
    `SELECT id,name,category,version,description,hardware FROM cogs WHERE id IN (${ids.map(() => "?").join(",")})`
  ).bind(...ids).all()).results as any[];
  const meta = new Map(metaRows.map((m) => [m.id, m]));
  const cogs = ids.map((id) => {
    const m: any = meta.get(id) || {};
    const artifacts: any = {};
    for (const a of arts.filter((x) => x.cog_id === id)) {
      artifacts[TARGET_REGKEY[a.target] || a.target] = {
        path: a.r2_key, size: a.size, sha256: a.sha256, sig: a.sig, manifest_path: a.manifest_key,
      };
    }
    return {
      id,
      name: m.name,
      version: m.version,
      category: m.category,
      description: m.description,
      hardware_requirement: safeJson(m.hardware) || [],
      artifacts,
    };
  });
  return c.json({ schema: 1, repo: "weavelogic", updated: today, cogs });
});

// COG-008 repo layout: serve each signed binary at its registry `path` (== the R2 key), so a
// generic client (weft-cog-repo, the on-device cogrepo cog) can fetch `<base>/<path>` as the
// signed-install contract requires — not only via /api/cogs/:id/download. The key is looked up in
// cog_artifacts, so ONLY a registered, signed artifact is served; the sha256 / Ed25519 sig /
// signer pubkey ride in headers for verify-before-trust. Streams from R2, falls back to D1 bytes.
app.get("/cogs/:arch/:file", async (c) => {
  const key = `cogs/${c.req.param("arch")}/${c.req.param("file")}`;
  const row = await c.env.DB.prepare(
    "SELECT cog_id,target,version,size,sha256,sig,signer_pubkey FROM cog_artifacts WHERE r2_key=? LIMIT 1"
  ).bind(key).first<any>();
  if (!row) return c.json({ error: "no such artifact" }, 404);
  const headers: Record<string, string> = {
    "content-type": "application/octet-stream",
    "content-disposition": `attachment; filename="${c.req.param("file")}"`,
    "x-cog-id": row.cog_id,
    "x-cog-target": row.target,
    "x-cog-version": row.version,
    "x-cog-size": String(row.size),
    "x-cog-sha256": row.sha256,
    "x-cog-sig": row.sig,
    "x-cog-signer": row.signer_pubkey,
    "cache-control": "public, max-age=3600",
  };
  if (c.env.ASSETS) {
    const obj = await c.env.ASSETS.get(key);
    if (obj) {
      headers["etag"] = obj.httpEtag;
      return new Response(obj.body, { headers });
    }
  }
  const blob = await c.env.DB.prepare(
    "SELECT bytes FROM cog_artifacts WHERE r2_key=?"
  ).bind(key).first<{ bytes: ArrayBuffer | Uint8Array | number[] | null }>();
  if (!blob || !blob.bytes) return c.json({ error: "artifact bytes not uploaded yet" }, 404);
  const b: any = blob.bytes;
  const body: BodyInit = b instanceof ArrayBuffer || ArrayBuffer.isView(b) ? b : new Uint8Array(b);
  return new Response(body, { headers });
});

// The cog (if any) and firmware (if any) that map to a given part id.
async function cogsForPart(env: Env, partId: string) {
  const { results } = await env.DB.prepare("SELECT data FROM cogs WHERE maps_to LIKE ?").bind(`%"${partId}"%`).all();
  const cogs = (results as any[]).map((r) => JSON.parse(r.data)).filter((c) => (c.maps_to || []).includes(partId));
  for (const c of cogs) {
    if (!c.git) c.git = cogGitUrl(c);
    const arts = await artifactsFor(env, c.id);
    if (arts.length) c.artifacts = arts;
  }
  return cogs;
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
  await recordEvent(c.env.DB, "research", refId, "", note, refType);
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
  await recordEvent(c.env.DB, "cog_request", partId, "", String(b.note || ""));
  return c.json({ ok: true, status: "requested", part_id: partId });
});

// Identity of the canonical catalog file this database was seeded from.
app.get("/api/catalog/release", async (c) => {
  const row = await catalogRelease(c.env.DB);
  if (!row) return c.json({ error: "catalog release not seeded" }, 404);
  return c.json(row);
});

// Full export, so the console/appliance can embed a snapshot. The digest is the seed's
// record of the canonical file, not a hash of this reconstructed JSON.
app.get("/api/catalog.json", async (c) => {
  const rows = (await c.env.DB.prepare("SELECT type,data FROM parts WHERE status='published'").all()).results as any[];
  const rel = await catalogRelease(c.env.DB);
  const cat: any = {
    schema: rel?.schema ?? 1,
    generated: rel?.generated ?? "",
    version: rel?.version ?? "",
    digest: rel?.digest ?? "",
    source: rel?.source_path ?? "crates/cog-market/catalog/catalog.json",
    projects: [],
    modules: [],
    chips: [],
  };
  for (const r of rows) (cat[`${r.type}s`] ||= []).push(JSON.parse(r.data as string));
  return c.json(cat);
});

// Imported parts pool (bulk LCSC/JLCPCB + distributor). Separate from the curated catalog.
app.get("/api/pool/search", async (c) => {
  const q = (c.req.query("q") || "").toLowerCase();
  const cat = c.req.query("category");
  const limit = Math.min(Math.max(1, Number(c.req.query("limit")) || 30), 200);
  let sql = "SELECT mpn,manufacturer,name,category,price,datasheet, json_extract(data,'$.image') AS image, json_extract(data,'$.summary') AS summary FROM pool WHERE search LIKE ?";
  const binds: any[] = [`%${q}%`];
  if (cat) { sql += " AND category=?"; binds.push(cat); }
  sql += " ORDER BY manufacturer,mpn LIMIT ?"; binds.push(limit);
  const { results } = await c.env.DB.prepare(sql).bind(...binds).all();
  return c.json({ count: results.length, results });
});
app.get("/api/pool/part", async (c) => {
  const mpn = (c.req.query("mpn") || "").trim();
  if (!mpn) return c.json({ error: "mpn required" }, 400);
  const row = await c.env.DB.prepare("SELECT data, hash FROM pool WHERE mpn=?").bind(mpn).first<{ data: string; hash: string }>();
  if (!row) return c.json({ error: "not found" }, 404);
  let data: any = {};
  try { data = JSON.parse(row.data); } catch { data = {}; }
  await recordLook(c.env.DB, mpn, "view");
  return c.json({ ...data, mpn: data.mpn || mpn, hash: row.hash || data.hash || "", source: "pool" });
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
  await recordLook(c.env.DB, id, "view");
  const [cogs, firmware, activity] = await Promise.all([
    cogsForPart(c.env, id),
    firmwareForPart(c.env, id),
    activityFor(c.env.DB, id),
  ]);
  return c.html(PART_PAGE(c.env.EXPLORER_NAME || "WeftOS Sensor Explorer", row.type, row.hash || "", part, cogs, firmware, activity));
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
<meta name=color-scheme content="dark">
<title>${name}</title>
<style>
:root{
  --bg:#08080A;--panel:#2A2A30;--card:#16161C;--hover:#202028;--active:#2C2C36;
  --ink:#E0DEE8;--ink-soft:#AAA8B4;--grey:#706E7A;
  --line:rgba(255,255,255,.094);--line-soft:rgba(255,255,255,.055);
  --wl:#C4A25C;--wl-ink:#E0DEE8;--accent:#C4A25C;--green:#6EC896;--warn:#DCAF55;--crit:#DC5F5F;
  --pool:#AAA8B4;--noteb:#16161C;--chip-bg:#202028;--focus:#C4A25C;--shadow:rgba(0,0,0,.45);
  --fs-xs:12px;--fs-sm:13px;--fs-base:15px;--fs-md:16px;--fs-lg:18px;--fs-xl:24px;--fs-2xl:28px;
  --sp-1:4px;--sp-2:8px;--sp-3:16px;--sp-4:20px;--sp-5:28px;--sp-6:36px;--r:10px;--r-sm:8px;--r-pill:999px;
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
header{display:flex;justify-content:space-between;align-items:flex-start;gap:24px;padding:var(--sp-5) var(--sp-5) var(--sp-3);max-width:1500px}
.brand{min-width:0}
h1{margin:0;font-size:var(--fs-2xl);font-weight:700;letter-spacing:-.015em}
.seedbanner{flex:none;display:flex;flex-direction:column;align-items:flex-start;gap:2px;margin-top:2px;padding:12px 14px;width:228px;border:1px solid var(--line);border-radius:var(--r);background:var(--card);color:var(--ink)}
.seedbanner:hover{text-decoration:none;border-color:rgba(196,162,92,.55);background:var(--hover)}
.seedk{display:flex;align-items:center;font-size:10px;letter-spacing:.08em;text-transform:uppercase;color:var(--ink-soft);font-weight:700}
.seedbanner b{font-size:16px;font-weight:700;letter-spacing:-.02em}
.seedbanner>span{color:var(--grey);font-size:12px;line-height:1.35}
.sub{color:var(--grey);margin:6px 0 0;font-size:var(--fs-sm);line-height:1.5}
.sub a{color:var(--grey);text-decoration:underline;text-underline-offset:2px}
.sub code{font-size:var(--fs-xs);background:var(--chip-bg);padding:1px 6px;border-radius:5px;color:var(--ink-soft)}

/* toolbar */
.bar{position:sticky;top:0;z-index:30;background:var(--bg);border-bottom:1px solid var(--line);padding:14px var(--sp-5);display:flex;gap:12px;align-items:center;flex-wrap:wrap}
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
.seg button[aria-pressed=true]{background:var(--active);color:var(--ink);box-shadow:inset 0 -2px 0 var(--accent)}
.seg button[aria-pressed=true] .ct{color:var(--ink-soft)}
#count{color:var(--grey);font-size:var(--fs-sm);white-space:nowrap;font-variant-numeric:tabular-nums}
.filtbtn{display:none;padding:9px 14px;border:1px solid var(--line);border-radius:var(--r-pill);background:var(--card);color:var(--ink);font-weight:600;font-size:var(--fs-sm);cursor:pointer}

/* active filter chips */
#active{display:none;gap:7px;flex-wrap:wrap;align-items:center;padding:10px var(--sp-5) 0}
.achip{display:inline-flex;align-items:center;gap:6px;background:var(--active);color:var(--ink);border:1px solid var(--line);border-radius:var(--r-sm);padding:5px 10px;font-size:var(--fs-xs);font-weight:600;cursor:pointer}
.achip.qchip{box-shadow:inset 2px 0 0 var(--accent)}
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
.facet.on{background:var(--active);color:var(--ink);font-weight:600;box-shadow:inset 2px 0 0 var(--accent)}
.facet.on .fn{color:var(--ink-soft)}
.fl{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.fn{color:var(--grey);font-size:var(--fs-xs);font-variant-numeric:tabular-nums;flex:none}

/* results */
.main{flex:1;min-width:0;padding:var(--sp-4) var(--sp-5) 64px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(270px,1fr));gap:16px}
.card{display:flex;flex-direction:column;width:100%;background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:16px 16px 14px;cursor:pointer;transition:border-color .12s,background .12s;text-align:left;color:var(--ink);font-family:inherit}
.card:hover{border-color:rgba(196,162,92,.55);background:var(--hover)}
.card:focus-visible{outline:2px solid var(--focus);outline-offset:2px}
.cardhead{display:flex;align-items:center;gap:8px;margin-bottom:8px}
.badge{font-size:10px;font-weight:700;text-transform:uppercase;letter-spacing:.06em;padding:3px 8px;border-radius:var(--r-sm);color:var(--ink-soft);border:1px solid var(--line);background:transparent;flex:none}
.b-module{color:var(--ink)}.b-chip{color:var(--accent);border-color:rgba(196,162,92,.45)}.b-project{color:var(--green);border-color:rgba(110,200,150,.4)}.b-pool,.b-cog{color:var(--ink-soft)}
.xgroup{grid-column:1/-1;margin:18px 0 2px;display:flex;align-items:center;gap:10px;color:var(--grey);font-size:var(--fs-xs);text-transform:uppercase;letter-spacing:.05em;font-weight:700}
.xgroup::after{content:"";flex:1;height:1px;background:var(--line)}
.maps{font-size:11px;color:var(--pool)}
.vend{color:var(--grey);font-size:var(--fs-xs);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin-left:auto}
.card h3{margin:0 0 6px;font-size:var(--fs-md);font-weight:600;line-height:1.3}
.sum{margin:0 0 12px;color:var(--ink-soft);font-size:var(--fs-sm);line-height:1.45;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
.cardfoot{margin-top:auto;display:flex;flex-wrap:wrap;gap:6px;align-items:center}
.pill{font-size:11px;padding:3px 9px;border-radius:var(--r-pill);border:1px solid var(--line);color:var(--grey);background:var(--bg);white-space:nowrap}
.pill.cat{border-color:color-mix(in srgb,var(--wl) 55%,var(--line));color:var(--wl-ink)}
.thumb{margin:-16px -16px 12px;height:132px;background:#0E0E12;border-bottom:1px solid var(--line);overflow:hidden}
.thumb img{width:100%;height:100%;object-fit:contain}
.price{margin-left:auto;font-size:var(--fs-sm);font-weight:700;color:var(--green);white-space:nowrap;font-variant-numeric:tabular-nums}
.promote{font-size:11px;color:var(--pool);border:1px dashed color-mix(in srgb,var(--pool) 60%,var(--line));border-radius:var(--r-pill);padding:3px 9px}

/* states */
.empty{color:var(--grey);padding:48px 8px;text-align:center}
.empty b{display:block;color:var(--ink);font-size:var(--fs-lg);font-weight:600;margin-bottom:6px}
.skel{background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:16px;height:156px;overflow:hidden}
.skel .l{height:10px;border-radius:5px;background:var(--hover);margin-bottom:12px}
.skel .l.w1{width:40%}.skel .l.w2{width:88%}.skel .l.w3{width:70%}
@media(prefers-reduced-motion:reduce){.card,#panel,#scrim{transition:none}}

/* detail panel */
#scrim{position:fixed;inset:0;background:rgba(0,0,0,.45);opacity:0;pointer-events:none;transition:opacity .18s;z-index:40}
#scrim.on{opacity:1;pointer-events:auto}
#panel{position:fixed;top:0;right:0;height:100%;width:640px;max-width:96vw;background:var(--card);border-left:1px solid var(--line);transform:translateX(100%);transition:transform .22s;z-index:50;display:flex;flex-direction:column}
#panel.open{transform:translateX(0)}
.ptop{display:flex;justify-content:flex-end;padding:12px 14px 0}
.pclose{background:var(--chip-bg);border:1px solid var(--line);color:var(--ink);border-radius:50%;width:34px;height:34px;font-size:19px;cursor:pointer;line-height:1}
#guidepop[hidden]{display:none}
#guidepop{position:fixed;inset:0;z-index:70;background:var(--bg);display:flex;flex-direction:column}
.guidebar{display:flex;align-items:center;gap:12px;padding:10px 14px;border-bottom:1px solid var(--line);background:var(--card)}
.guidebar strong{font-size:14px;font-weight:600}
.guidebar .pclose{margin-left:auto}
#guidecanvas{flex:1;width:100%;min-height:0;display:block;touch-action:none}
#guidestatus{margin:0;padding:0 14px 10px;color:var(--grey);font-size:13px}
#guidestatus:empty{display:none}
button.ext{font:inherit;cursor:pointer;background:transparent}
#pbody{padding:8px 28px 56px;overflow:auto}
.hero{display:grid;grid-template-columns:168px 1fr;gap:18px;align-items:start;margin-bottom:8px}
.shot{margin:0;background:#0E0E12;border:1px solid var(--line);border-radius:var(--r);min-height:148px;display:flex;flex-direction:column;align-items:center;justify-content:center;overflow:hidden;text-align:center}
.shot img{width:100%;height:168px;object-fit:contain;background:#0E0E12}
.shot.empty{color:var(--grey);font-size:12px;padding:14px;gap:4px}
.shot.empty em{font-style:normal;color:var(--ink-soft)}
.stack{display:flex;flex-direction:column;gap:12px;margin-top:16px}
.box{background:var(--bg);border:1px solid var(--line);border-radius:var(--r);padding:16px 16px 14px}
.box h3,.feedbox>b{margin:0 0 10px;font-size:11px;letter-spacing:.08em;text-transform:uppercase;color:var(--grey);font-weight:600}
.box.good h3{color:var(--green)}.box.bad h3{color:var(--crit)}
.box ul{margin:0;padding-left:18px}.box li{margin:4px 0}
.box table.spec{margin:0}
.box .note{background:transparent;border:0;border-top:1px solid var(--line-soft);border-radius:0;margin:0;padding:10px 0}
.box .note:first-of-type{border-top:0;padding-top:0}
.pulse[hidden]{display:none}
.home[hidden],.cathead[hidden]{display:none}
.home{margin:0 0 28px}
.welcome{display:grid;grid-template-columns:minmax(0,1.45fr) minmax(240px,.8fr);gap:18px;align-items:start}
.welcome h2{margin:0 0 8px;font-size:var(--fs-xl);font-weight:700;letter-spacing:-.02em;line-height:1.2}
.lead{margin:0 0 14px;color:var(--ink-soft);font-size:var(--fs-base);line-height:1.5;max-width:68ch}
.points{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:10px;margin:0 0 14px}
.point{background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:12px 14px}
.point b{display:block;font-size:13px;margin-bottom:4px;color:var(--ink)}
.point span{display:block;color:var(--grey);font-size:12px;line-height:1.4}
.homeacts{display:flex;flex-wrap:wrap;gap:8px}
.homeacts .ext{margin:0}
.feature>b{display:block;margin:0 0 10px;font-size:11px;letter-spacing:.08em;text-transform:uppercase;color:var(--grey);font-weight:600}
.another{margin-top:10px;background:transparent;border:1px solid var(--line);color:var(--ink-soft);border-radius:var(--r-pill);padding:7px 12px;font:600 12px/1 inherit;cursor:pointer}
.another:hover{background:var(--hover);color:var(--ink)}
.cathead{margin:4px 0 12px;font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:var(--grey);font-weight:700}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:12px}
.pins{display:flex;flex-wrap:wrap;gap:6px}
.pin{display:inline-flex;flex-direction:column;gap:2px;align-items:flex-start;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px;padding:8px 10px;border:1px solid var(--line);border-radius:6px;background:var(--card);color:var(--ink)}
.pin b{font-weight:600}
.pin span{color:var(--grey);font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;font-size:11px;font-weight:500}
.meters{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;margin-bottom:8px}
.meter b{display:block;font-size:20px;font-weight:600;letter-spacing:-.02em;color:var(--ink);font-variant-numeric:tabular-nums}
.meter span{font-size:10px;letter-spacing:.06em;text-transform:uppercase;color:var(--grey)}
.ev{display:flex;justify-content:space-between;gap:12px;padding:7px 0;border-bottom:1px solid var(--line-soft);font-size:12px;color:var(--ink-soft)}
.ev time{color:var(--grey);font-variant-numeric:tabular-nums;flex:none}
.hint{margin:0;color:var(--grey);font-size:13px}
.pulse{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:12px;margin:0 0 20px}
.stat{background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:14px 16px}
.stat b{display:block;font-size:22px;font-weight:600;letter-spacing:-.02em;color:var(--ink);font-variant-numeric:tabular-nums}
.stat span{color:var(--grey);font-size:11px;letter-spacing:.06em;text-transform:uppercase}
.feedbox{grid-column:1/-1;background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:14px 16px}
.mark{display:inline-block;width:8px;height:8px;background:var(--accent);margin-right:10px;vertical-align:1px}
.phead{display:flex;align-items:center;gap:8px;margin:4px 0 8px}
#pbody h2{margin:2px 0 4px;font-size:var(--fs-xl);font-weight:700;line-height:1.2}
.pid{font-size:var(--fs-xs);color:var(--grey);font-family:ui-monospace,SFMono-Regular,Menlo,monospace}
.cogsub{margin:0 0 10px;font-size:var(--fs-sm);color:var(--grey);display:flex;align-items:center;gap:7px;flex-wrap:wrap}
.cogslug{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:11px;color:var(--grey);background:var(--chip-bg);padding:2px 7px;border-radius:var(--r-pill)}
.role{color:var(--grey);margin:0 0 10px;font-size:var(--fs-sm)}
.psum{font-size:var(--fs-base);color:var(--ink);margin:6px 0 10px;line-height:1.5}
.note{background:var(--noteb);border-left:3px solid var(--accent);padding:9px 12px;border-radius:0 6px 6px 0;margin:8px 0;font-size:var(--fs-sm);color:var(--ink)}
.lb{margin:12px 0;font-size:var(--fs-sm)}.lb ul{margin:5px 0 0;padding-left:18px}.lb li{margin:2px 0}
.lb.good b{color:var(--green)}.lb.bad b{color:var(--crit)}
.phint{background:var(--chip-bg);border:1px dashed color-mix(in srgb,var(--pool) 55%,var(--line));color:var(--ink-soft);padding:10px 12px;border-radius:var(--r-sm);font-size:var(--fs-sm);margin:10px 0}
table.spec{width:100%;border-collapse:collapse;margin:14px 0;font-size:var(--fs-sm)}
table.spec th{text-align:left;color:var(--grey);font-weight:600;padding:6px 12px 6px 0;vertical-align:top;white-space:nowrap;width:1%}
table.spec td{padding:6px 0;border-bottom:1px solid var(--line-soft);color:var(--ink)}
.links{margin:14px 0}.ext{display:inline-flex;align-items:center;gap:6px;padding:8px 14px;border:1px solid var(--wl);color:var(--wl-ink);border-radius:var(--r-pill);font-size:var(--fs-sm);font-weight:600}
.ext:hover{text-decoration:none;background:color-mix(in srgb,var(--wl) 12%,transparent)}
.dlgrid{display:flex;flex-wrap:wrap;gap:8px;margin:12px 0}
.ext.dl{background:transparent;color:var(--ink);border-color:var(--line)}.ext.dl:hover{background:var(--hover)}
.dlmeta{opacity:.85;font-weight:500;font-size:var(--fs-xs)}
.sigfacts{margin:12px 0;display:grid;grid-template-columns:auto 1fr;gap:4px 12px;font-size:var(--fs-xs)}
.sigfacts>div{display:contents}.sigfacts dt{color:var(--grey);text-transform:uppercase;font-size:10px;letter-spacing:.04em;align-self:center}
.sigfacts dd{margin:0;color:var(--ink)}
.mono{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:11px;word-break:break-all}
.buys{display:flex;flex-wrap:wrap;gap:8px;margin:10px 0 0}
.buy{display:inline-flex;gap:8px;align-items:center;background:transparent;color:var(--ink);border:1px solid var(--line);padding:8px 12px;border-radius:var(--r-sm);font-size:var(--fs-sm);font-weight:600}
.buy:hover{text-decoration:none;border-color:rgba(110,200,150,.55);background:var(--hover)}.buy .price{color:var(--green);margin:0}
.buy small{color:var(--grey);font-weight:500}
.xref{margin:16px 0}.xref>b{display:block;color:var(--grey);font-size:var(--fs-xs);text-transform:uppercase;letter-spacing:.04em;margin-bottom:8px}
.xlinks{display:flex;flex-wrap:wrap;gap:7px}
.xlink{display:inline-block;padding:6px 12px;border:1px solid var(--line);border-radius:var(--r-pill);font-size:var(--fs-sm);cursor:pointer;background:var(--bg);color:var(--ink-soft)}
.xlink:hover{border-color:var(--wl);text-decoration:none;color:var(--wl-ink)}
.tags{display:flex;flex-wrap:wrap;gap:6px}.tagp{font-size:11px;padding:3px 9px;border:1px solid var(--line);border-radius:var(--r-pill);color:var(--grey)}
.loading{color:var(--grey)}

/* mobile */
.sideback{position:fixed;inset:0;background:rgba(0,0,0,.4);opacity:0;pointer-events:none;transition:opacity .2s;z-index:45}
@media(max-width:860px){
  header{flex-direction:column;align-items:stretch;padding:var(--sp-4) var(--sp-4) var(--sp-2)}
  .seedbanner{width:auto;max-width:280px;align-self:flex-end}
  .welcome,.points{grid-template-columns:1fr}
  .bar{padding:10px var(--sp-4)}
  #active{padding:10px var(--sp-4) 0}
  .main{padding:var(--sp-4) var(--sp-4) 64px}
  .side{position:fixed;top:0;left:0;height:100%;z-index:50;transform:translateX(-100%);transition:transform .2s;max-height:100%;width:284px;box-shadow:2px 0 22px var(--shadow)}
  body.drawer .side{transform:translateX(0)}
  body.drawer .sideback{opacity:1;pointer-events:auto}
  .filtbtn{display:inline-flex;align-items:center;gap:6px}
  #panel{width:100%;max-width:100%}
  .hero,.pair{grid-template-columns:1fr}
  .meters,.pulse{grid-template-columns:1fr 1fr}
  .shot img{height:140px}
  #pbody{padding:8px 16px 48px}
}
</style></head><body>
<header>
  <div class=brand>
    <h1><span class=mark aria-hidden=true></span>${name}</h1>
    <p class=sub>A browsable, contributable hardware catalog with an MCP agent surface. <a href="/api/facets">facets</a> &middot; <a href="/api/catalog.json">catalog export</a> &middot; <a href="/catalog.json">canonical catalog</a> &middot; <a href="/catalog-release.json">catalog artifact</a> &middot; <a href="/api/catalog/release">catalog release</a> &middot; <code>POST /mcp</code></p>
    <p class=sub id=catalogrel></p>
  </div>
  <a class=seedbanner href="https://cognitum.one/a/9PUnUY" target=_blank rel=noopener>
    <span class=seedk><span class=mark aria-hidden=true></span>Cognitum</span>
    <b>Seed</b>
    <span>The appliance these cogs run on.</span>
  </a>
</header>
<div class=bar>
  <button class=filtbtn id=filtbtn aria-controls=side aria-expanded=false><span aria-hidden=true>&#9776;</span> Filters</button>
  <div class=searchbox role=search>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>
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
  <main class=main>
    <section id=home class=home hidden>
      <div class=welcome>
        <div>
          <h2>The catalog, and the cog that reads each part.</h2>
          <p class=lead>Sensor Explorer lists the sensors and boards, and the cogs that read them. A cog is a small signed program for one part. It writes a JSON line. That same cog runs on a Cognitum Seed and on a WeftOS host, so a reading from the appliance is a reading WeftOS can fuse, keep, and act on.</p>
          <div class=points>
            <div class=point><b>Catalog</b><span>Filter from the sidebar, or search by name. A card opens the spec, the pins, and where to buy.</span></div>
            <div class=point><b>One cog, one part</b><span>Radar, ECG, time of flight, sound, attitude. Each cog ships with a guide and a signature.</span></div>
            <div class=point><b>Seed and WeftOS</b><span>Cognitum Seed runs the cog at the edge. WeftOS runs it on a host. One cog, both places.</span></div>
          </div>
          <div class=homeacts>
            <a class=ext href="https://cognitum.one/a/9PUnUY" target=_blank rel=noopener>Cognitum Seed</a>
            <a class=ext href="https://weftos.weavelogic.ai" target=_blank rel=noopener>WeftOS</a>
          </div>
        </div>
        <div class=feature>
          <b>A part from the catalog</b>
          <div id=feature></div>
          <button type=button class=another id=another>Show another part</button>
        </div>
      </div>
    </section>
    <div id=pulse class=pulse hidden></div>
    <p id=catlabel class=cathead hidden>Browse the catalog</p>
    <div class=grid id=grid aria-busy=true></div>
  </main>
</div>
<div class=sideback id=sideback></div>
<div id=scrim></div>
<div id=guidepop hidden role=dialog aria-modal=true aria-labelledby=guidetitle>
  <div class=guidebar>
    <strong id=guidetitle>Guide</strong>
    <button class=pclose id=guideclose type=button aria-label="Close guide">&times;</button>
  </div>
  <p id=guidestatus></p>
  <canvas id=guidecanvas></canvas>
</div>
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
  var photo=m.rec&&(m.rec.photo||m.rec.image);
  return '<article class=card role=button tabindex=0 data-id="'+esc(m.id)+'" data-key="'+esc(keyOf(m))+'" aria-label="'+esc(m.name)+', '+esc(m.type)+'">'
    +thumbHTML(photo)
    +'<div class=cardhead><span class="badge b-'+m.type+'">'+m.type+'</span>'+(m.vendor?'<span class=vend>'+esc(m.vendor)+'</span>':'')+'</div>'
    +'<h3>'+esc(m.name)+'</h3>'
    +(m.summary?'<p class=sum>'+esc(trim(m.summary,160))+'</p>':'')
    +'<div class=cardfoot>'+catPills(m.cats)+(m.price?'<span class=price>'+esc(m.price)+'</span>':'')+'</div></article>';
}
function skeletons(n){var h='';for(var i=0;i<n;i++)h+='<div class=skel aria-hidden=true><div class="l w1"></div><div class="l w2"></div><div class="l w3"></div></div>';return h;}

function setCount(txt){document.getElementById('count').textContent=txt;}

var HOME_ID='';
function homePool(){
  var pool=INDEX.filter(function(m){return m.type==='module'&&m.summary;});
  if(!pool.length)pool=INDEX.filter(function(m){return m.type==='module';});
  if(!pool.length)pool=INDEX.slice();
  return pool;
}
function pickHome(next){
  var pool=homePool();
  if(!pool.length)return;
  var m=null;
  if(!next&&HOME_ID){
    for(var i=0;i<pool.length;i++){if(pool[i].id===HOME_ID){m=pool[i];break;}}
  }
  if(!m){
    var n=Math.floor(Math.random()*pool.length);
    if(pool.length>1&&pool[n].id===HOME_ID)n=(n+1)%pool.length;
    m=pool[n];HOME_ID=m.id;
  }
  var el=document.getElementById('feature');
  if(el)el.innerHTML=cardHTML(m);
}
function syncHome(){
  var el=document.getElementById('home');
  var label=document.getElementById('catlabel');
  var show=MODE==='catalog'&&!Q&&!filtersOn()&&CAT_READY;
  if(el){el.hidden=!show;if(show)pickHome(false);}
  if(label)label.hidden=!show;
}
function render(){
  if(MODE==='pool'){renderPool();return;}
  if(MODE==='cogs'){renderCogs();return;}
  syncHome();
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
  syncPulse();
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
  var blurb=p.summary||((p.mpn||'')+(p.category?' · '+p.category:''));
  return '<article class=card role=button tabindex=0 data-pool="'+esc(p.mpn)+'" aria-label="'+esc(p.name||p.mpn)+', pool part">'
    +thumbHTML(p.image)
    +'<div class=cardhead><span class="badge b-pool">pool</span>'+(p.manufacturer?'<span class=vend>'+esc(p.manufacturer)+'</span>':'')+'</div>'
    +'<h3>'+esc(p.name||p.mpn)+'</h3>'
    +'<p class=sum>'+esc(blurb)+'</p>'
    +'<div class=cardfoot><span class=promote>promotable</span>'+(p.price?'<span class=price>'+esc(p.price)+'</span>':'')+'</div></article>';
}
function renderPool(){
  syncHome();
  var g=document.getElementById('grid');
  if(!POOL.loaded){g.setAttribute('aria-busy','true');g.innerHTML=skeletons(8);return;}
  g.setAttribute('aria-busy','false');
  var items=POOL.results;
  g.innerHTML=items.length?items.map(poolCardHTML).join(''):poolEmptyHTML();
  var tot=POOL.stats&&POOL.stats.total?POOL.stats.total:0;
  var shown=nfmt(items.length)+(items.length>=200?'+':'')+' of '+nfmt(tot)+' pool parts';
  setCount(shown);
  renderActive();
  syncPulse();
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
/* Headline = the real product name of the sensor the cog reads; fall back to the cog's own
   function name only for sensorless cogs (bridge/catalog/ota). */
function cogHead(c){return c.sensor_name||c.name||c.id;}
function cogGit(c){
  if(c&&typeof c.git==='string'&&c.git.indexOf('https://github.com/')===0)return c.git;
  var id=c&&c.id?String(c.id):'';
  if(!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(id))return '';
  if(id==='sensor-ota-push')return '';
  return 'https://github.com/weave-logic-ai/weftos-cogs/tree/main/src/cogs/'+id;
}
function cogSourceHTML(c){
  var url=cogGit(c);
  if(!url)return '';
  return box('Source','<div class=links><a class=ext href="'+esc(url)+'" target=_blank rel=noopener>src/cogs/'+esc(c.id)+'</a></div>');
}
function cogCardHTML(c){
  var maps=cogMapsTo(c);
  var head=cogHead(c);
  var role=c.sensor_name?(c.name||''):'';            // the cog's function, shown as a subtitle
  var sub=(c.category||'')+(c.version?' \\u00b7 v'+c.version:'');
  return '<article class=card role=button tabindex=0 data-cog="'+esc(c.id)+'" aria-label="'+esc(head)+', cog">'
    +'<div class=cardhead><span class="badge b-cog">cog</span>'+(sub?'<span class=vend>'+esc(sub)+'</span>':'')+'</div>'
    +'<h3>'+esc(head)+'</h3>'
    +'<p class=cogsub>'+(role?esc(role)+' ':'')+'<code class=cogslug>'+esc(c.id)+'</code></p>'
    +(c.description?'<p class=sum>'+esc(trim(c.description,160))+'</p>':'')
    +'<div class=cardfoot>'+(c.has_guide?'<span class=pill>guide</span>':'')+(c.sensor_name?'<span class=maps>Reads '+esc(c.sensor_name)+'</span>':(maps.length?'<span class=maps>works with '+nfmt(maps.length)+' part'+(maps.length===1?'':'s')+'</span>':'<span class=promote>no part yet</span>'))+'</div></article>';
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
  syncHome();
  var g=document.getElementById('grid');
  if(!COGS.loaded){g.setAttribute('aria-busy','true');g.innerHTML=skeletons(6);return;}
  g.setAttribute('aria-busy','false');
  var items=cogFilter();COGS.results=items;
  g.innerHTML=items.length?items.map(cogCardHTML).join(''):'<p class=empty><b>No cogs match</b>Try a sensor name, radar, ecg, tof\\u2026</p>';
  setCount(nfmt(items.length)+' cog'+(items.length===1?'':'s'));
  renderActive();
  syncPulse();
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
function showPart(body, p, id){
  body.innerHTML=detailHTML(p);body.scrollTop=0;document.getElementById('pclose').focus();
  lookSeq++;loadActivity((p&&p.id)||id, lookSeq);
}
function openDetail(id){
  var body=document.getElementById('pbody');
  body.innerHTML='<p class=loading>Loading\\u2026</p>';openPanel();
  fetch('/api/parts/'+encodeURIComponent(id)).then(function(r){return r.json();}).then(function(p){
    if(!p||p.error){var m=BY_ID[id];if(m)showPart(body,m.rec,id);else body.innerHTML='<p>Not found.</p>';return;}
    showPart(body,p,id);
  }).catch(function(){var m=BY_ID[id];if(m)showPart(body,m.rec,id);else body.innerHTML='<p>Failed to load.</p>';});
}
function renderCogDetail(c){
  var maps=cogMapsTo(c);
  var head=cogHead(c);
  var h='<div class=hero><figure class="shot empty"><em>Cog</em><span>'+esc(c.id)+'</span></figure><div>';
  h+='<div class=phead><span class="badge b-cog">cog</span><span class=vend>'+esc((c.category||'')+(c.version?' · v'+c.version:''))+'</span></div>';
  h+='<h2 id=ptitle>'+esc(head)+'</h2>';
  if(c.sensor_name)h+='<p class=role>'+esc(c.name||'')+'</p>';
  h+='<p class=pid>'+esc(c.id)+(c.hash?' · '+esc(c.hash):'')+'</p>';
  if(c.description)h+='<p class=psum>'+esc(c.description)+'</p>';
  h+='</div></div><div class=stack>';
  h+=cogSourceHTML(c);
  h+=guideButton(c.guide_url, c.id, head);
  h+='<div id=activity class=box><h3>Attention</h3><p class=hint>Loading counts…</p></div>';
  var spec={};
  if(c.bind_port)spec['API port']=c.bind_port;
  if(c.store_id)spec['Base store id']=c.store_id;
  var hw=c.hardware_requirement||c.hardware;
  if(hw&&hw.length)spec['Hardware']=Array.isArray(hw)?hw.join(', '):String(hw);
  if(c.binary)spec['Binary']=c.binary;
  var st=specTable(spec);
  if(st)h+=box('Runtime', st);
  if(c.sensor_id)h+=box('Reads','<div class=xlinks><a class=xlink href="/part/'+encodeURIComponent(c.sensor_id)+'">'+esc(c.sensor_name||c.sensor_id)+'</a></div>');
  if(maps.length)h+=refList('Works with these parts', maps, nameOf, false);
  else if(!c.sensor_id)h+=box('Parts','<p class=hint>No catalog part maps to this cog yet.</p>');
  h+=box('Install', cogDownloadsHTML(c));
  h+=extraBox(c);
  h+='</div>';
  return h;
}
function openCogDetail(id){
  openPanel();
  var body=document.getElementById('pbody');
  body.innerHTML='<p class=loading>Loading…</p>';
  fetch('/api/cogs/'+encodeURIComponent(id)).then(function(r){return r.json();}).then(function(d){
    var c=(d&&!d.error)?d:((COGS.byId&&COGS.byId[id])||null);
    if(!c){body.innerHTML='<p>Cog not found.</p>';return;}
    if(d&&!d.error){COGS.byId=COGS.byId||{};COGS.byId[id]=d;}
    body.innerHTML=renderCogDetail(c);body.scrollTop=0;document.getElementById('pclose').focus();
    lookSeq++;loadActivity(c.id||id, lookSeq);
  }).catch(function(){
    var c=(COGS.byId&&COGS.byId[id])||null;
    if(!c){body.innerHTML='<p>Failed to load.</p>';return;}
    body.innerHTML=renderCogDetail(c);
    lookSeq++;loadActivity(c.id||id, lookSeq);
  });
}
var TARGET_LABEL={armv7:'ARM \\u00b7 armv7 (Pi Zero 2 W, 32-bit)',aarch64:'ARM64 \\u00b7 aarch64 (Pi 5, 64-bit)',x86_64:'Linux \\u00b7 x86_64 (glibc host)'};
function fmtBytes(n){n=Number(n)||0;return n>=1048576?(n/1048576).toFixed(1)+' MB':n>=1024?(n/1024).toFixed(0)+' KB':n+' B';}
function shortHex(s,head,tail){s=String(s||'');head=head||8;tail=tail||8;return s.length<=head+tail+1?s:s.slice(0,head)+'\\u2026'+s.slice(-tail);}
/* Real per-target download links + signed facts + the signed-install command. */
function cogDownloadsHTML(c){
  var arts=(c&&c.artifacts&&c.artifacts.length)?c.artifacts:null;
  if(!arts)return '<div class=note>Install on a Seed over MCP: <code>cog install '+esc(c.id)+'</code></div>';
  var signer=arts[0].signer_pubkey||'';
  var h='<div class=dlgrid>';
  for(var i=0;i<arts.length;i++){var a=arts[i];
    h+='<a class="ext dl" href="'+esc(a.url)+'" download>\\u2193 '+esc(TARGET_LABEL[a.target]||a.target)+' <span class=dlmeta>'+esc(fmtBytes(a.size))+'</span></a>';}
  h+='</div><dl class=sigfacts><div><dt>Version</dt><dd>v'+esc(arts[0].version)+'</dd></div>';
  for(var j=0;j<arts.length;j++){var b=arts[j];
    h+='<div><dt>'+esc(b.target)+' sha256</dt><dd class=mono>'+esc(b.sha256)+'</dd></div>';
    h+='<div><dt>'+esc(b.target)+' sig</dt><dd class=mono>'+esc(shortHex(b.sig,16,16))+'</dd></div>';}
  h+='<div><dt>Signed by</dt><dd class=mono title="'+esc(signer)+'">'+esc(shortHex(signer,12,8))+'</dd></div></dl>';
  h+='<div class=note>Signed install on a Seed over MCP: <code>cog install '+esc(c.id)+'</code> \\u2014 verified against the WeaveLogic release key (<code>'+esc(shortHex(signer,12,8))+'</code>) via the signed registry at <code>/registry.json</code>.</div>';
  return h;
}
function poolDetailHTML(p){
  p=p||{};
  var h='<div class=hero>'+photoFrame(p.image||p.photo, p.name||p.mpn);
  h+='<div><div class=phead><span class="badge b-pool">pool</span>'+(p.manufacturer?'<span class=vend>'+esc(p.manufacturer)+'</span>':'')+'</div>';
  h+='<h2 id=ptitle>'+esc(p.name||p.mpn||'Pool part')+'</h2>';
  h+='<p class=pid>'+esc(p.mpn||'')+(p.hash?' · '+esc(p.hash):'')+'</p>';
  if(p.summary)h+='<p class=psum>'+esc(p.summary)+'</p>';
  var bits=[];
  if(p.category)bits.push(p.category);
  if(p.package)bits.push(p.package);
  if(p.availability)bits.push(p.availability);
  if(bits.length)h+='<p class=role>'+bits.map(esc).join(' · ')+'</p>';
  h+='</div></div><div class=stack>';
  h+='<div id=activity class=box><h3>Attention</h3><p class=hint>Loading counts…</p></div>';
  var spec={};
  if(p.manufacturer)spec.Manufacturer=p.manufacturer;
  if(p.category)spec.Category=p.category;
  if(p.mouser_category)spec['Mouser category']=p.mouser_category;
  if(p.jlc_category)spec['JLC category']=p.jlc_category;
  if(p.package)spec.Package=p.package;
  if(p.lcsc_part)spec['LCSC part']=p.lcsc_part;
  if(p.price)spec.Price=p.price;
  if(p.availability)spec.Availability=p.availability;
  if(p.source)spec.Source=p.source;
  if(p.imported)spec.Imported=p.imported;
  var st=specTable(spec);
  if(st)h+=box('Record', st);
  if(p.attributes&&typeof p.attributes==='object'){var at=specTable(p.attributes);if(at)h+=box('Attributes', at);}
  h+=supplyBox(p);
  h+=box('Catalog status','<p class=hint>This row is in the imported pool. It stays out of the published catalog until a reviewer accepts a contribution.</p>');
  h+=extraBox(p);
  h+='</div>';
  return h;
}
function openPoolDetail(mpn){
  var cached=POOL.byId[mpn]||null;
  if(!cached && !mpn)return;
  openPanel();
  var body=document.getElementById('pbody');
  body.innerHTML='<p class=loading>Loading…</p>';
  fetch('/api/pool/part?mpn='+encodeURIComponent(mpn)).then(function(r){return r.json();}).then(function(p){
    if(!p||p.error)p=cached;
    if(!p){body.innerHTML='<p>Not found.</p>';return;}
    body.innerHTML=poolDetailHTML(p);body.scrollTop=0;document.getElementById('pclose').focus();
    lookSeq++;loadActivity(p.mpn||mpn, lookSeq);
  }).catch(function(){
    if(!cached){body.innerHTML='<p>Failed to load.</p>';return;}
    body.innerHTML=poolDetailHTML(cached);
    lookSeq++;loadActivity(mpn, lookSeq);
  });
}
function closeDetail(){
  var panel=document.getElementById('panel');
  panel.classList.remove('open');panel.setAttribute('aria-hidden','true');
  document.getElementById('scrim').classList.remove('on');
  if(lastFocus&&lastFocus.focus){lastFocus.focus();}
  fetch('/api/pulse').then(function(r){return r.json();}).then(renderPulse).catch(function(){});
}
function isHttp(s){s=String(s||'');return s.indexOf('http://')===0||s.indexOf('https://')===0;}
function thumbHTML(src){if(!isHttp(src))return '';return '<div class=thumb><img alt="" src="'+esc(src)+'"></div>';}
function photoFrame(src, alt){
  src=src==null?'':String(src);
  if(isHttp(src))return '<figure class=shot><img alt="'+esc(alt||'Part photo')+'" src="'+esc(src)+'"></figure>';
  if(src)return '<figure class="shot empty"><em>Photo on file</em><span>'+esc(src)+'</span></figure>';
  return '<figure class="shot empty"><em>No photo yet</em><span>A product photo can be added later.</span></figure>';
}
function box(title, inner){if(!inner)return '';return '<section class=box><h3>'+esc(title)+'</h3>'+inner+'</section>';}
function fmtVal(v){
  var text;
  if(v&&typeof v==='object'){try{text=JSON.stringify(v);}catch(e){text=String(v);}}
  else text=String(v==null?'':v);
  if(text.length>480)text=text.slice(0,479)+'…';
  return text;
}
function specTable(spec){
  if(!spec||typeof spec!=='object')return '';
  var rows='';
  Object.keys(spec).forEach(function(k){
    var v=spec[k];
    if(v==null||v==='')return;
    rows+='<tr><th>'+esc(k)+'</th><td>'+esc(fmtVal(v))+'</td></tr>';
  });
  if(!rows)return '';
  return '<table class=spec>'+rows+'</table>';
}
function buyBlock(buy){
  if(!buy||!buy.length)return '';
  return '<div class=buys>'+buy.map(function(b){
    var ship=b.ships_from?' <small>from '+esc(b.ships_from)+'</small>':'';
    return '<a class=buy href="'+esc(b.url||'#')+'" target=_blank rel=noopener>'+esc(b.vendor||'Buy')+(b.price?' <span class=price>'+esc(b.price)+'</span>':'')+ship+'</a>';
  }).join('')+'</div>';
}
function nameOf(id){var m=BY_ID[id];return m?m.name:id;}
function projectsUsing(id){return PROJECTS.filter(function(p){return (p.modules||[]).indexOf(id)>=0;}).map(function(p){return p.id;});}
function modulesWithChip(id){return MODULES.filter(function(m){return (m.rec.chips||[]).indexOf(id)>=0;}).map(function(m){return m.id;});}
function listInner(arr){
  if(!arr||!arr.length)return '';
  return '<ul>'+arr.map(function(x){return '<li>'+esc(typeof x==='object'?fmtVal(x):x)+'</li>';}).join('')+'</ul>';
}
function pairBlocks(p){
  var g=listInner(p.good_for), b=listInner(p.not_for);
  if(!g&&!b)return '';
  return '<div class=pair>'
    +(g?'<section class="box good"><h3>Good for</h3>'+g+'</section>':'<section class=box><h3>Good for</h3><p class=hint>Nothing noted yet.</p></section>')
    +(b?'<section class="box bad"><h3>Not for</h3>'+b+'</section>':'<section class=box><h3>Not for</h3><p class=hint>Nothing noted yet.</p></section>')
    +'</div>';
}
function notesBox(notes){
  if(!notes||!notes.length)return '';
  return box('Notes', notes.map(function(n){return '<div class=note>'+esc(n)+'</div>';}).join(''));
}
function supplyBox(p){
  var h='';
  if(p.datasheet&&isHttp(p.datasheet))h+='<a class=ext href="'+esc(p.datasheet)+'" target=_blank rel=noopener>Datasheet</a>';
  else if(p.datasheet)h+='<p class=hint>Datasheet on file: '+esc(p.datasheet)+'</p>';
  if(p.mouser_query)h+='<p class=hint>Mouser query: '+esc(p.mouser_query)+'</p>';
  if(p.url&&isHttp(p.url))h+='<a class=ext href="'+esc(p.url)+'" target=_blank rel=noopener>Product page</a>';
  h+=buyBlock(p.buy);
  if(!h)return '';
  return box('Supply','<div class=links>'+h+'</div>');
}
function pinGrid(pins){
  if(!pins||!pins.length)return '';
  var cells=pins.map(function(x){
    if(x&&typeof x==='object'){
      var label=x.name||x.pin||x.signal||x.label||'';
      var fn=x.function||x.fn||x.desc||x.note||'';
      return '<span class=pin><b>'+esc(label)+'</b>'+(fn?'<span>'+esc(fn)+'</span>':'')+'</span>';
    }
    return '<span class=pin><b>'+esc(x)+'</b></span>';
  }).join('');
  return box('Pinout','<div class=pins>'+cells+'</div>');
}
function refList(label, ids, nameFn, linkPage){
  if(!ids||!ids.length)return '';
  var links=ids.map(function(id){
    var name=nameFn?nameFn(id):id;
    if(linkPage)return '<a class=xlink href="/part/'+encodeURIComponent(id)+'">'+esc(name)+'</a>';
    return '<span class=xlink role=button tabindex=0 data-open="'+esc(id)+'">'+esc(name)+'</span>';
  }).join('');
  return box(label,'<div class=xlinks>'+links+'</div>');
}
function seenBox(seen){
  if(!seen||!seen.length)return '';
  var bits=seen.map(function(s){
    s=String(s);
    if(isHttp(s))return '<a class=xlink href="'+esc(s)+'" target=_blank rel=noopener>'+esc(s)+'</a>';
    return '<span class=pin><b>'+esc(s)+'</b></span>';
  }).join('');
  return box('Seen in','<div class=pins>'+bits+'</div>');
}
function tagsBox(tags){
  if(!tags||!tags.length)return '';
  return box('Tags','<div class=pins>'+tags.map(function(t){return '<span class=pin><b>'+esc(typeof t==='object'?fmtVal(t):t)+'</b></span>';}).join('')+'</div>');
}
var SHOWN={id:1,name:1,vendor:1,manufacturer:1,kind:1,type:1,role:1,summary:1,good_for:1,not_for:1,notes:1,spec:1,pins:1,chips:1,photo:1,datasheet:1,mouser_query:1,buy:1,seen_in:1,modules:1,tags:1,difficulty:1,category:1,projects:1,image:1,hash:1,status:1,source:1,artifacts:1,maps_to:1,description:1,bind_port:1,store_id:1,hardware:1,hardware_requirement:1,binary:1,sensor_id:1,sensor_name:1,version:1,git:1,availability:1,attributes:1,package:1,lcsc_part:1,jlc_category:1,mouser_category:1,pool_area:1,imported:1,url:1,price:1,search:1,mpn:1,guide:1,guide_url:1,has_guide:1};
function guideButton(url, id, title){
  if(!url||!id)return '';
  return '<p class=links><button type=button class="ext guidebtn" data-guide="'+esc(id)+'" data-guide-title="'+esc(title||id)+'">Open the guide</button></p>';
}
var guideMod=null,guideSeq=0;
function closeGuide(){
  var pop=document.getElementById('guidepop');
  if(pop)pop.hidden=true;
  if(guideMod&&guideMod.close_guide){try{guideMod.close_guide();}catch(e){}}
}
function openGuide(id, title){
  var pop=document.getElementById('guidepop');
  var status=document.getElementById('guidestatus');
  document.getElementById('guidetitle').textContent=title||id||'Guide';
  pop.hidden=false;
  status.textContent='Loading the guide…';
  var seq=++guideSeq;
  fetch('/api/guides/'+encodeURIComponent(id)).then(function(r){
    if(!r.ok)throw new Error('no guide');
    return r.text();
  }).then(function(json){
    if(seq!==guideSeq)return null;
    var ready=guideMod?Promise.resolve(guideMod):import('/guide-view/guide_view.js').then(function(m){
      return m.default().then(function(){guideMod=m;return m;});
    });
    return ready.then(function(m){
      if(seq!==guideSeq)return;
      var rel=window.__catalogRel;
      status.textContent=rel&&rel.digest?('catalog '+rel.version+' '+rel.digest):'';
      return m.open_guide('guidecanvas', json);
    });
  }).catch(function(){
    if(seq!==guideSeq)return;
    status.textContent='The guide did not open.';
  });
}
function extraBox(p){
  var rows='';
  Object.keys(p||{}).forEach(function(k){
    if(SHOWN[k])return;
    var v=p[k];
    if(v==null||v==='')return;
    if(Array.isArray(v)&&!v.length)return;
    rows+='<tr><th>'+esc(k)+'</th><td>'+esc(fmtVal(v))+'</td></tr>';
  });
  if(!rows)return '';
  return box('Also on file','<table class=spec>'+rows+'</table>');
}
function when(s){s=String(s||'');if(!s)return '';return s.replace('T',' ').replace('Z','').slice(0,16);}
function meter(n,label){return '<div class=meter><b>'+nfmt(Number(n)||0)+'</b><span>'+esc(label)+'</span></div>';}
function evRow(kind, detail, t){
  return '<div class=ev><span>'+esc(kind)+(detail?' · '+esc(detail):'')+'</span>'+(t?'<time>'+esc(when(t))+'</time>':'')+'</div>';
}
function attentionHTML(a){
  a=a||{};
  var h='<div class=meters>'
    +meter(a.views,'Opened')
    +meter(a.mcp_reads,'MCP reads')
    +meter((a.research||[]).length,'Reports')
    +meter((a.contributions||[]).length,'Contributions')
    +'</div>';
  var rows='';
  (a.events||[]).forEach(function(e){
    if(!e)return;
    if(e.kind==='view'||e.kind==='research'||e.kind==='cog_request'||e.kind==='contribution')return;
    if(e.kind==='mcp'&&(!e.tool||e.tool==='get_part'))return;
    rows+=evRow(e.tool||e.kind||'mcp', e.note||'', e.created);
  });
  (a.research||[]).forEach(function(r){rows+=evRow('report', ((r.status?r.status+' ':'')+(r.note||'')).trim(), r.created);});
  (a.cog_requests||[]).forEach(function(r){rows+=evRow('cog request', ((r.status?r.status+' ':'')+(r.note||'')).trim(), r.created);});
  (a.contributions||[]).forEach(function(c){rows+=evRow('contribution', ((c.status||'')+' '+(c.type||'')+(c.author?' by '+c.author:'')).trim(), c.created);});
  if(rows)h+=rows;
  else h+='<p class=hint>No reports yet. Opens and MCP reads still count above, including zero.</p>';
  if(a.last_at)h+='<p class=hint>Last look '+esc(when(a.last_at))+'</p>';
  return box('Attention', h);
}
var lookSeq=0;
function loadActivity(ref, seq){
  var slot=document.getElementById('activity');
  if(!slot||!ref)return;
  fetch('/api/activity?ref='+encodeURIComponent(ref)).then(function(r){return r.json();}).then(function(a){
    if(seq!==lookSeq)return;
    var slot2=document.getElementById('activity');
    if(slot2)slot2.outerHTML=attentionHTML(a);
  }).catch(function(){
    if(seq!==lookSeq)return;
    var slot2=document.getElementById('activity');
    if(slot2)slot2.innerHTML='<p class=hint>Attention counts are unavailable right now.</p>';
  });
}
function detailHTML(p){
  var meta=BY_ID[p.id]||{};
  var type=meta.type||(p.manufacturer?'chip':((p.difficulty||p.modules)?'project':'module'));
  var vend=vendorOf(p);
  var h='<div class=hero>'+photoFrame(p.photo||p.image, p.name||p.id)+'<div>';
  h+='<div class=phead><span class="badge b-'+type+'">'+esc(type)+'</span>'+(vend?'<span class=vend>'+esc(vend)+'</span>':'')+'</div>';
  h+='<h2 id=ptitle>'+esc(p.name||p.id)+'</h2>';
  if(p.id)h+='<p class=pid>'+esc(p.id)+(p.hash?' · '+esc(p.hash):'')+'</p>';
  if(p.id)h+='<p class=pid><a href="/part/'+encodeURIComponent(p.id)+'">Open the full page</a></p>';
  if(p.role)h+='<p class=role>'+esc(p.role)+'</p>';
  var metaBits=[];
  if(p.kind)metaBits.push(p.kind);
  if(p.category)metaBits.push(p.category);
  if(p.difficulty)metaBits.push(p.difficulty);
  if(p.manufacturer&&p.manufacturer!==vend)metaBits.push(p.manufacturer);
  if(metaBits.length)h+='<p class=role>'+metaBits.map(esc).join(' · ')+'</p>';
  if(p.summary)h+='<p class=psum>'+esc(p.summary)+'</p>';
  h+=guideButton(p.guide_url, p.id, p.name||p.id);
  var pills='';
  (meta.cats||[]).forEach(function(c){pills+='<span class="pill cat">'+esc(labelCat(c))+'</span>';});
  (meta.ifaces||[]).forEach(function(i){pills+='<span class=pill>'+esc(labelIf(i))+'</span>';});
  if(pills)h+='<div class=cardfoot>'+pills+'</div>';
  h+='</div></div><div class=stack>';
  h+='<div id=activity class=box><h3>Attention</h3><p class=hint>Loading counts…</p></div>';
  h+=notesBox(p.notes);
  h+=pairBlocks(p);
  var spec=specTable(p.spec);
  if(spec)h+=box('Spec', spec);
  h+=pinGrid(p.pins);
  h+=supplyBox(p);
  if(type==='module'){h+=refList('Chips on board', p.chips, nameOf, false);h+=refList('Projects using this', projectsUsing(p.id), nameOf, false);}
  if(type==='chip')h+=refList('Modules with this chip', modulesWithChip(p.id), nameOf, false);
  if(type==='project')h+=refList('Modules', p.modules, nameOf, false);
  h+=seenBox(p.seen_in);
  h+=tagsBox(p.tags);
  h+=extraBox(p);
  h+='</div>';
  return h;
}
function filtersOn(){
  return ['type','category','interface','vendor'].some(function(g){return Object.keys(FILTERS[g]).length;});
}
function syncPulse(){
  var el=document.getElementById('pulse');
  if(!el)return;
  el.hidden=!(MODE==='catalog'&&!Q&&!filtersOn()&&el.innerHTML);
}
function renderPulse(d){
  var el=document.getElementById('pulse');
  if(!el||!d||!d.totals)return;
  var t=d.totals;
  var h='<div class=stat><b>'+nfmt(t.views||0)+'</b><span>Opens</span></div>'
    +'<div class=stat><b>'+nfmt(t.mcp_reads||0)+'</b><span>MCP reads</span></div>'
    +'<div class=stat><b>'+nfmt(t.research_queued||0)+'</b><span>Reports queued</span></div>'
    +'<div class=stat><b>'+nfmt(t.contributions_pending||0)+'</b><span>Pending review</span></div>';
  var recent=d.recent||[];
  h+='<div class=feedbox><b>Recent</b>';
  if(recent.length){
    recent.forEach(function(e){
      var label=e.tool||e.kind||'event';
      var detail=((e.ref_id||'')+' '+(e.note||'')).trim();
      h+=evRow(label, detail, e.created);
    });
  }else h+='<p class=hint>No events recorded yet. Opens, MCP reads, reports, and contributions land here.</p>';
  h+='</div>';
  el.innerHTML=h;
  syncPulse();
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
  var gb=e.target.closest('[data-guide]');
  if(gb){e.preventDefault();e.stopPropagation();openGuide(gb.getAttribute('data-guide'), gb.getAttribute('data-guide-title')||'');return;}
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
window.addEventListener('keydown',function(e){
  if(e.key!=='Escape')return;
  var pop=document.getElementById('guidepop');
  if(!pop||pop.hidden)return;
  e.preventDefault();
  e.stopPropagation();
  closeGuide();
},true);
document.addEventListener('keydown',function(e){
  if(e.key==='Escape'){
    var pop=document.getElementById('guidepop');
    if(pop&&!pop.hidden){closeGuide();return;}
    closeDetail();if(document.body.classList.contains('drawer'))setDrawer(false);return;
  }
  if(e.key!=='Enter'&&e.key!==' ')return;
  var t=e.target;
  if(t.classList&&(t.classList.contains('facet')||t.classList.contains('xlink')||t.classList.contains('card'))){e.preventDefault();t.click();}
});
document.getElementById('q').addEventListener('input',onSearch);
document.getElementById('qclear').addEventListener('click',clearSearch);
document.getElementById('pclose').addEventListener('click',closeDetail);
document.getElementById('guideclose').addEventListener('click',closeGuide);
document.getElementById('scrim').addEventListener('click',closeDetail);
document.getElementById('sideback').addEventListener('click',function(){setDrawer(false);});
document.getElementById('filtbtn').addEventListener('click',function(){setDrawer(!document.body.classList.contains('drawer'));});
document.getElementById('mode-catalog').addEventListener('click',function(){setMode('catalog');});
document.getElementById('mode-cogs').addEventListener('click',function(){setMode('cogs');});
document.getElementById('mode-pool').addEventListener('click',function(){setMode('pool');});
document.getElementById('another').addEventListener('click',function(e){e.stopPropagation();pickHome(true);});

/* ---- boot ---- */
document.getElementById('grid').innerHTML=skeletons(8);
(function(){
  var q=location.search.replace(/^\\?/,'').split('&');
  for(var i=0;i<q.length;i++){
    var kv=q[i].split('=');
    if(kv[0]==='guide'&&kv[1]){openGuide(decodeURIComponent(kv[1]), decodeURIComponent(kv[1].replace(/\\+/g,' ')));break;}
  }
})();
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
fetch('/api/pulse').then(function(r){return r.json();}).then(renderPulse).catch(function(){});
fetch('/api/catalog/release').then(function(r){return r.json();}).then(function(d){
  if(d&&d.digest)return d;
  return fetch('/catalog-release.json').then(function(r){return r.json();});
}).then(function(d){
  if(!d||!d.digest)return;
  window.__catalogRel=d;
  var el=document.getElementById('catalogrel');
  if(el)el.textContent='catalog '+d.version+' '+d.digest;
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
function isHttpSrv(s: unknown): boolean {
  const v = String(s || "");
  return v.startsWith("https://") || v.startsWith("http://");
}
function fmtValSrv(v: unknown): string {
  const text = v && typeof v === "object" ? JSON.stringify(v) : String(v ?? "");
  return text.length > 480 ? text.slice(0, 479) + "…" : text;
}
function specTableSrv(spec: any): string {
  if (!spec || typeof spec !== "object") return "";
  const rows = Object.keys(spec).filter((k) => spec[k] != null && spec[k] !== "").map((k) => '<tr><th>' + esc(k) + '</th><td>' + esc(fmtValSrv(spec[k])) + '</td></tr>').join("");
  return rows ? '<table class=spec>' + rows + '</table>' : "";
}
function boxSrv(title: string, inner: string): string {
  if (!inner) return "";
  return '<section class=box><h3>' + esc(title) + '</h3>' + inner + '</section>';
}
function photoSrv(src: unknown, alt: string): string {
  const s = String(src || "");
  if (isHttpSrv(s)) return '<figure class=shot><img alt="' + esc(alt) + '" src="' + esc(s) + '"></figure>';
  if (s) return '<figure class="shot empty"><em>Photo on file</em><span>' + esc(s) + '</span></figure>';
  return '<figure class="shot empty"><em>No photo yet</em><span>A product photo can be added later.</span></figure>';
}
function listInnerSrv(arr: any): string {
  if (!Array.isArray(arr) || !arr.length) return "";
  return '<ul>' + arr.map((x) => '<li>' + esc(typeof x === "object" ? fmtValSrv(x) : x) + '</li>').join("") + '</ul>';
}
function pinGridSrv(pins: any): string {
  if (!Array.isArray(pins) || !pins.length) return "";
  const cells = pins.map((x) => {
    if (x && typeof x === "object") {
      const label = x.name || x.pin || x.signal || x.label || "";
      const fn = x.function || x.fn || x.desc || x.note || "";
      return '<span class=pin><b>' + esc(label) + '</b>' + (fn ? '<span>' + esc(fn) + '</span>' : '') + '</span>';
    }
    return '<span class=pin><b>' + esc(x) + '</b></span>';
  }).join("");
  return boxSrv("Pinout", '<div class=pins>' + cells + '</div>');
}
function linkRowSrv(label: string, ids: any): string {
  if (!Array.isArray(ids) || !ids.length) return "";
  const links = ids.map((id) => '<a class=xlink href="/part/' + encodeURIComponent(String(id)) + '">' + esc(id) + '</a>').join("");
  return boxSrv(label, '<div class=xlinks>' + links + '</div>');
}
const SHOWN_SRV = new Set(["id", "name", "vendor", "manufacturer", "kind", "type", "role", "summary", "good_for", "not_for", "notes", "spec", "pins", "chips", "photo", "datasheet", "mouser_query", "buy", "seen_in", "modules", "tags", "difficulty", "category", "projects", "image", "hash", "status", "source", "git"]);
function extraSrv(p: any): string {
  const rows = Object.keys(p || {}).filter((k) => !SHOWN_SRV.has(k)).map((k) => {
    const v = p[k];
    if (v == null || v === "" || (Array.isArray(v) && !v.length)) return "";
    return '<tr><th>' + esc(k) + '</th><td>' + esc(fmtValSrv(v)) + '</td></tr>';
  }).join("");
  return rows ? boxSrv("Also on file", '<table class=spec>' + rows + '</table>') : "";
}
function whenSrv(s: unknown): string {
  return String(s || "").replace("T", " ").replace("Z", "").slice(0, 16);
}
function activityBoxSrv(a: Activity): string {
  const meter = (n: number, label: string) => '<div class=meter><b>' + esc(n || 0) + '</b><span>' + esc(label) + '</span></div>';
  let h = '<div class=meters>' + meter(a.views, "Opened") + meter(a.mcp_reads, "MCP reads") + meter((a.research || []).length, "Reports") + meter((a.contributions || []).length, "Contributions") + '</div>';
  const row = (kind: string, detail: string, t: unknown) => '<div class=ev><span>' + esc(kind) + (detail ? " · " + esc(detail) : "") + '</span>' + (t ? '<time>' + esc(whenSrv(t)) + '</time>' : '') + '</div>';
  let rows = "";
  for (const e of a.events || []) {
    if (!e || e.kind === "view" || e.kind === "research" || e.kind === "cog_request" || e.kind === "contribution") continue;
    if (e.kind === "mcp" && (!e.tool || e.tool === "get_part")) continue;
    rows += row(e.tool || e.kind || "mcp", e.note || "", e.created);
  }
  for (const r of a.research || []) rows += row("report", String((r.status ? r.status + " " : "") + (r.note || "")).trim(), r.created);
  for (const r of a.cog_requests || []) rows += row("cog request", String((r.status ? r.status + " " : "") + (r.note || "")).trim(), r.created);
  for (const c of a.contributions || []) rows += row("contribution", String((c.status || "") + " " + (c.type || "") + (c.author ? " by " + c.author : "")).trim(), c.created);
  h += rows || '<p class=hint>No reports yet. Opens and MCP reads still count above, including zero.</p>';
  if (a.last_at) h += '<p class=hint>Last look ' + esc(whenSrv(a.last_at)) + '</p>';
  return boxSrv("Attention", h);
}

function overviewTab(type: string, hash: string, p: any, activity: Activity): string {
  const vend = vendorOfSrv(p);
  let h = '<div class=hero>' + photoSrv(p.photo || p.image, String(p.name || p.id || "Part")) + '<div>';
  h += '<div class=phead><span class="badge b-' + esc(type) + '">' + esc(type) + '</span>' + (vend ? '<span class=vend>' + esc(vend) + '</span>' : '') + '</div>';
  h += '<h1>' + esc(p.name || p.id) + '</h1>';
  h += '<p class=pid>' + esc(p.id || "") + '</p>';
  if (hash) h += '<p class=hash title="stable item hash">' + esc(hash) + '</p>';
  if (p.role) h += '<p class=role>' + esc(p.role) + '</p>';
  const meta = [p.kind, p.category, p.difficulty, p.manufacturer && p.manufacturer !== vend ? p.manufacturer : ""].filter(Boolean);
  if (meta.length) h += '<p class=role>' + meta.map((x: any) => esc(x)).join(" · ") + '</p>';
  if (p.summary) h += '<p class=psum>' + esc(p.summary) + '</p>';
  h += '</div></div><div class=stack>';
  h += activityBoxSrv(activity);
  if (Array.isArray(p.notes) && p.notes.length) h += boxSrv("Notes", p.notes.map((n: any) => '<div class=note>' + esc(n) + '</div>').join(""));
  const good = listInnerSrv(p.good_for);
  const bad = listInnerSrv(p.not_for);
  if (good || bad) {
    h += '<div class=pair>'
      + (good ? '<section class="box good"><h3>Good for</h3>' + good + '</section>' : '<section class=box><h3>Good for</h3><p class=hint>Nothing noted yet.</p></section>')
      + (bad ? '<section class="box bad"><h3>Not for</h3>' + bad + '</section>' : '<section class=box><h3>Not for</h3><p class=hint>Nothing noted yet.</p></section>')
      + '</div>';
  }
  const spec = specTableSrv(p.spec);
  if (spec) h += boxSrv("Spec", spec);
  h += pinGridSrv(p.pins);
  let supply = "";
  if (p.datasheet && isHttpSrv(p.datasheet)) supply += '<a class=ext href="' + esc(p.datasheet) + '" target=_blank rel=noopener>Datasheet</a>';
  else if (p.datasheet) supply += '<p class=hint>Datasheet on file: ' + esc(p.datasheet) + '</p>';
  if (p.mouser_query) supply += '<p class=hint>Mouser query: ' + esc(p.mouser_query) + '</p>';
  if (Array.isArray(p.buy) && p.buy.length) {
    supply += '<div class=buys>' + p.buy.map((b: any) => '<a class=buy href="' + esc(b.url || "#") + '" target=_blank rel=noopener>' + esc(b.vendor || "Buy") + (b.price ? ' <span class=price>' + esc(b.price) + '</span>' : '') + (b.ships_from ? ' <small>from ' + esc(b.ships_from) + '</small>' : '') + '</a>').join("") + '</div>';
  }
  if (supply) h += boxSrv("Supply", '<div class=links>' + supply + '</div>');
  if (type === "module") h += linkRowSrv("Chips on board", p.chips);
  if (type === "project" || (Array.isArray(p.modules) && p.modules.length)) h += linkRowSrv("Modules", p.modules);
  if (Array.isArray(p.seen_in) && p.seen_in.length) {
    h += boxSrv("Seen in", '<div class=pins>' + p.seen_in.map((s: any) => isHttpSrv(s) ? '<a class=xlink href="' + esc(s) + '" target=_blank rel=noopener>' + esc(s) + '</a>' : '<span class=pin><b>' + esc(s) + '</b></span>').join("") + '</div>');
  }
  if (Array.isArray(p.tags) && p.tags.length) h += boxSrv("Tags", '<div class=pins>' + p.tags.map((t: any) => '<span class=pin><b>' + esc(t) + '</b></span>').join("") + '</div>');
  h += extraSrv(p);
  h += '</div>';
  return h;
}

function cogTab(cogs: any[]): string {
  if (!cogs.length) {
    return '<div class=cta><p class=ctatext>No cog maps to this part yet. A cog is the small reader that brings this sensor onto the mesh.</p>' +
      '<button class=btn id=createcog>Create a cog for this part</button><p class=msg id=cogmsg></p></div>';
  }
  return cogs.map((c) => {
    const head = c.sensor_name || c.name || c.id;
    let h = '<div class=cogcard>';
    h += '<div class=phead><span class="badge b-cog">cog</span><span class=vend>' + esc(c.category || "") + (c.version ? ' · v' + esc(c.version) : '') + '</span></div>';
    h += '<h2>' + esc(head) + '</h2>';
    if (c.sensor_name) h += '<p class=role>' + esc(c.name || "") + '</p>';
    h += '<p class=pid>' + esc(c.id) + '</p>';
    if (c.description) h += '<p class=psum>' + esc(c.description) + '</p>';
    const git = cogGitUrl(c);
    if (git) {
      h += '<div class=links><a class=ext href="' + esc(git) + '" target=_blank rel=noopener>src/cogs/' + esc(c.id) + '</a>';
      if (guideFor(String(c.id || ""))) {
        h += '<a class=ext href="/?guide=' + encodeURIComponent(c.id) + '">Open the guide</a>';
      }
      h += '</div>';
    } else if (guideFor(String(c.id || ""))) {
      h += '<div class=links><a class=ext href="/?guide=' + encodeURIComponent(c.id) + '">Open the guide</a></div>';
    }
    const spec: any = {};
    if (c.bind_port) spec["API port"] = c.bind_port;
    if (c.store_id) spec["Base store id"] = c.store_id;
    if (Array.isArray(c.hardware_requirement) && c.hardware_requirement.length) spec["Hardware"] = c.hardware_requirement.join(", ");
    if (c.binary) spec["Binary"] = c.binary;
    h += specTableSrv(spec);
    if (c.sensor_id) h += '<div class=cogreads><b>Reads</b> <a href="/part/' + encodeURIComponent(c.sensor_id) + '">' + esc(c.sensor_name) + '</a></div>';
    h += cogDownloadsSrv(c);
    h += '</div>';
    return h;
  }).join("");
}

const TARGET_LABEL: Record<string, string> = {
  armv7: "ARM · armv7 (Pi Zero 2 W, 32-bit)",
  aarch64: "ARM64 · aarch64 (Pi 5, 64-bit)",
  x86_64: "Linux · x86_64 (glibc host)",
};

// Real per-target download links + signed facts + the signed-install command. Falls back to the
// MCP install hint when no artifact is published for the cog yet.
function cogDownloadsSrv(c: any): string {
  const arts: any[] = Array.isArray(c.artifacts) ? c.artifacts : [];
  if (!arts.length) {
    return '<div class=note>Install on a Seed over MCP: <code>cog install ' + esc(c.id) +
      '</code> (cog-dev / seed-mcp). The cog then serves its API on port ' + esc(c.bind_port || "?") + '.</div>';
  }
  const signer = arts[0].signer_pubkey || "";
  let h = '<div class=dlgrid>';
  for (const a of arts) {
    h += '<a class="ext dl" href="' + esc(a.url) + '" download>↓ ' + esc(TARGET_LABEL[a.target] || a.target) +
      ' <span class=dlmeta>' + esc(fmtBytes(a.size)) + '</span></a>';
  }
  h += '</div>';
  h += '<dl class=sigfacts><div><dt>Version</dt><dd>v' + esc(arts[0].version) + '</dd></div>';
  for (const a of arts) {
    h += '<div><dt>' + esc(a.target) + ' sha256</dt><dd class=mono>' + esc(a.sha256) + '</dd></div>';
    h += '<div><dt>' + esc(a.target) + ' sig</dt><dd class=mono>' + esc(shortHex(a.sig, 16, 16)) + '</dd></div>';
  }
  h += '<div><dt>Signed by</dt><dd class=mono title="' + esc(signer) + '">' + esc(shortHex(signer, 12, 8)) + '</dd></div></dl>';
  h += '<div class=note>Signed install on a Seed over MCP: <code>cog install ' + esc(c.id) +
    '</code> — the binary is verified against the WeaveLogic release key (<code>' + esc(shortHex(signer, 12, 8)) +
    '</code>) via the signed registry at <code>/registry.json</code>. The cog then serves its API on port ' +
    esc(c.bind_port || "?") + '.</div>';
  return h;
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
  return '<!doctype html><meta charset=utf-8><meta name=color-scheme content=dark><title>Not found</title><body style="font-family:system-ui;background:#08080A;color:#E0DEE8;padding:40px"><h1>Part not found</h1><p>No catalog part with id <code>' + esc(id) + '</code>.</p><p><a style="color:#C4A25C" href="/">← Back to the explorer</a></p></body>';
}

function PART_PAGE(name: string, type: string, hash: string, part: any, cogs: any[], firmware: any[], activity: Activity): string {
  const hasCog = cogs.length > 0;
  const hasFw = firmware.length > 0;
  const ov = overviewTab(type, hash, part, activity);
  const cg = cogTab(cogs);
  const fw = firmwareTab(firmware);
  const pid = String(part.id || "");
  return `<!doctype html><html lang=en><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<meta name=color-scheme content="dark">
<title>${esc(part.name || pid)} — ${esc(name)}</title>
<style>
:root{
  --bg:#08080A;--panel:#2A2A30;--card:#16161C;--hover:#202028;--active:#2C2C36;
  --ink:#E0DEE8;--ink-soft:#AAA8B4;--grey:#706E7A;
  --line:rgba(255,255,255,.094);--line-soft:rgba(255,255,255,.055);
  --wl:#C4A25C;--accent:#C4A25C;--green:#6EC896;--warn:#DCAF55;--crit:#DC5F5F;
  --focus:#C4A25C;--r:10px;--r-sm:8px;
}
*{box-sizing:border-box}html,body{margin:0}
body{background:var(--bg);color:var(--ink);font:16px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif}
a{color:var(--ink);text-decoration:none}a:hover{text-decoration:underline}
.wrap{max-width:880px;margin:0 auto;padding:28px 20px 80px}
.back{display:inline-flex;align-items:center;gap:8px;color:var(--grey);font-size:13px;margin:0 0 18px}
.back::before{content:"";width:8px;height:8px;background:var(--accent)}
.phead{display:flex;align-items:center;gap:8px;margin-bottom:8px}
.badge{font-size:10px;font-weight:700;text-transform:uppercase;letter-spacing:.06em;padding:3px 8px;border-radius:var(--r-sm);color:var(--ink-soft);border:1px solid var(--line);background:transparent}
.b-chip{color:var(--accent);border-color:rgba(196,162,92,.45)}.b-project{color:var(--green);border-color:rgba(110,200,150,.4)}
.vend{color:var(--grey);font-size:12px;margin-left:auto}
.cogreads{margin:0 0 12px;font-size:13px}.cogreads b{color:var(--grey);text-transform:uppercase;font-size:11px;letter-spacing:.06em;margin-right:6px}
h1{margin:2px 0 4px;font-size:28px;font-weight:700;letter-spacing:-.02em;line-height:1.15}
h2{margin:2px 0 6px;font-size:18px}
.pid,.hash{font-size:12px;color:var(--grey);font-family:ui-monospace,SFMono-Regular,Menlo,monospace;margin:0 0 4px}
.role{color:var(--grey);margin:0 0 8px;font-size:13px}
.psum{font-size:15px;margin:8px 0 0;line-height:1.5}
.hero{display:grid;grid-template-columns:200px 1fr;gap:20px;align-items:start;margin-bottom:8px}
.shot{margin:0;background:#0E0E12;border:1px solid var(--line);border-radius:var(--r);min-height:160px;display:flex;flex-direction:column;align-items:center;justify-content:center;overflow:hidden;text-align:center}
.shot img{width:100%;height:180px;object-fit:contain;background:#0E0E12}
.shot.empty{color:var(--grey);font-size:12px;padding:16px;gap:4px}
.shot.empty em{font-style:normal;color:var(--ink-soft)}
.stack{display:flex;flex-direction:column;gap:12px;margin-top:16px}
.box,.cogcard,.actions{background:var(--card);border:1px solid var(--line);border-radius:var(--r);padding:16px}
.box h3{margin:0 0 10px;font-size:11px;letter-spacing:.08em;text-transform:uppercase;color:var(--grey);font-weight:600}
.box.good h3{color:var(--green)}.box.bad h3{color:var(--crit)}
.box ul{margin:0;padding-left:18px}.box li{margin:4px 0}
.box table.spec{margin:0}
.box .note{background:transparent;border:0;border-top:1px solid var(--line-soft);border-radius:0;margin:0;padding:10px 0;font-size:14px}
.box .note:first-of-type{border-top:0;padding-top:0}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:12px}
.pins,.xlinks,.buys,.dlgrid{display:flex;flex-wrap:wrap;gap:8px}
.pin{display:inline-flex;flex-direction:column;gap:2px;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px;padding:8px 10px;border:1px solid var(--line);border-radius:6px;background:var(--bg)}
.pin b{font-weight:600}.pin span{color:var(--grey);font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;font-size:11px}
.meters{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;margin-bottom:8px}
.meter b{display:block;font-size:22px;font-weight:600;font-variant-numeric:tabular-nums}
.meter span,.hint{color:var(--grey);font-size:12px}
.meter span{font-size:10px;letter-spacing:.06em;text-transform:uppercase}
.ev{display:flex;justify-content:space-between;gap:12px;padding:7px 0;border-bottom:1px solid var(--line-soft);font-size:12px;color:var(--ink-soft)}
.ev time{color:var(--grey);font-variant-numeric:tabular-nums}
.hint{margin:8px 0 0}
table.spec{width:100%;border-collapse:collapse;font-size:13px}
table.spec th{text-align:left;color:var(--grey);font-weight:600;padding:6px 12px 6px 0;vertical-align:top;white-space:nowrap;width:1%}
table.spec td{padding:6px 0;border-bottom:1px solid var(--line-soft)}
.links{display:flex;flex-direction:column;gap:8px}
.ext,.buy,.xlink{display:inline-flex;align-items:center;gap:8px;padding:8px 12px;border:1px solid var(--line);border-radius:var(--r-sm);font-size:13px;font-weight:600;color:var(--ink);background:var(--bg)}
.ext:hover,.buy:hover,.xlink:hover{text-decoration:none;background:var(--hover)}
.buy .price{color:var(--green);margin:0}.buy small{color:var(--grey);font-weight:500}
.ext.dl{background:transparent;color:var(--ink)}
.dlmeta{color:var(--grey);font-weight:500;font-size:12px}
.sigfacts{margin:8px 0 0;display:grid;grid-template-columns:auto 1fr;gap:4px 12px;font-size:12px}
.sigfacts>div{display:contents}.sigfacts dt{color:var(--grey);text-transform:uppercase;font-size:10px;letter-spacing:.04em;align-self:center}
.sigfacts dd{margin:0}
.mono{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:11px;word-break:break-all}
.note code{background:var(--hover);padding:1px 6px;border-radius:5px;font-size:12px}
.tabs{display:flex;gap:6px;margin:0 0 16px}
.tab{background:var(--card);border:1px solid var(--line);border-radius:var(--r-sm);color:var(--ink-soft);font:600 14px inherit;padding:8px 14px;cursor:pointer}
.tab[aria-selected=true]{background:var(--active);color:var(--ink);box-shadow:inset 0 -2px 0 var(--accent)}
.tabpanel[hidden]{display:none}
.cogcard{margin-bottom:12px}
.cta{text-align:left}.ctatext{color:var(--ink-soft);font-size:14px;margin:0 0 14px}
.btn{background:var(--active);color:var(--ink);border:1px solid var(--line);box-shadow:inset 0 -2px 0 var(--accent);border-radius:var(--r-sm);padding:10px 16px;font:600 14px inherit;cursor:pointer}
.btn:disabled{opacity:.6;cursor:default}
.actions{display:flex;gap:12px;flex-wrap:wrap;align-items:center;margin-top:12px}
.btn.ghost{background:transparent;box-shadow:none;color:var(--ink-soft)}
.msg{font-size:13px;color:var(--green);min-height:1em}
:focus-visible{outline:2px solid var(--focus);outline-offset:2px}
@media(max-width:720px){.hero,.pair,.meters{grid-template-columns:1fr}.wrap{padding:20px 14px 64px}.shot img{height:140px}}
</style></head><body>
<div class=wrap>
<a class=back href="/">${esc(name)}</a>
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
