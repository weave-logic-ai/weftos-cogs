// Scan the cogs-bridge source tree for cog.toml manifests and emit cogs.seed.json — the cog
// registry that makes cogs discoverable in the Sensor Explorer. Each record carries the manifest
// facts {id,name,category,version,description,store_id,hardware_requirement,bind_port} plus
// `maps_to`: the catalog part ids the cog works with (verified against the canonical catalog).
//
// Usage: node scripts/scan-cogs.mjs <cogsDir> [catalog.json] [cogs.seed.json]
import { readFileSync, writeFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const defaultCatalog = resolve(dirname(fileURLToPath(import.meta.url)), "../../../crates/cog-market/catalog/catalog.json");
const [, , cogsDirArg, catPath = defaultCatalog, outPath = "cogs.seed.json"] = process.argv;
if (!cogsDirArg) {
  console.error("usage: node scripts/scan-cogs.mjs <cogsDir> [catalog.json] [cogs.seed.json]");
  process.exit(1);
}
const cogsDir = cogsDirArg;

// Which catalog part(s) each cog works with. Verified against the catalog below; a cog may map to
// nothing (network/app cogs) and still appear in the registry. The FIRST mapped entry that is a
// catalog module becomes the cog's primary sensor (its real product name), so order matters:
// put the module the cog actually reads first.
const MAPS_TO = {
  "rd-03e": ["rd-03e-module"],
  "hlk-as201": ["hlk-as201-module", "as201-imu"],
  "sen0628-tof": ["sen0628-tof", "dfrobot-sen0628-vl53l7cx", "vl53l7cx"],
  "sen0213-ecg": ["sen0213-ecg", "ad8232-board", "ad8232"],
  "sound-detect": ["ky-038", "inmp441-mic"],
  "mentra-live": ["mentra-live-glasses", "mentra-live-display", "mentra-live-microphone"],
  "ld2450-radar": ["hlk-ld2450"],
  "ld1040c-motion": ["hlk-ld1040c"],
  "ld6002-radar": ["hlk-ld6002", "hlk-ld6002b"],
  "sensor-ota-push": [],
  "bridge": [],
  "catalog": [],
};

// --- tiny TOML reader: top-level [sections] with `key = value` lines (string | int | string[]) ---
function parseToml(src) {
  const out = {};
  let section = "";
  for (let raw of src.split(/\r?\n/)) {
    const line = raw.replace(/\s+#.*$/, "").trim();
    if (!line || line.startsWith("#")) continue;
    const sec = line.match(/^\[([^\]]+)\]$/);
    if (sec) { section = sec[1]; out[section] ||= {}; continue; }
    const kv = line.match(/^([A-Za-z0-9_]+)\s*=\s*(.+)$/);
    if (!kv) continue;
    const key = kv[1];
    let v = kv[2].trim();
    let val;
    if (v.startsWith("[")) {
      val = [...v.matchAll(/"([^"]*)"/g)].map((m) => m[1]);
    } else if (v.startsWith('"')) {
      val = v.slice(1, v.lastIndexOf('"'));
    } else if (/^(true|false)$/.test(v)) {
      val = v === "true";
    } else if (/^-?\d+$/.test(v)) {
      val = Number(v);
    } else {
      val = v.replace(/^"|"$/g, "");
    }
    (section ? (out[section] ||= {}) : out)[key] = val;
  }
  return out;
}

// catalog ids, to verify maps_to; plus a module id -> real product name map so each cog can carry
// the real sensor name as its headline (modules are preferred over chips as the primary sensor).
const cat = JSON.parse(readFileSync(catPath, "utf8"));
const catIds = new Set([...(cat.modules || []), ...(cat.chips || []), ...(cat.projects || [])].map((r) => r.id));
const moduleName = new Map((cat.modules || []).map((r) => [r.id, r.name || r.id]));

const cogs = [];
const dirs = readdirSync(cogsDir).filter((d) => {
  try { return statSync(join(cogsDir, d)).isDirectory() && existsSync(join(cogsDir, d, "cog.toml")); }
  catch { return false; }
}).sort();

for (const d of dirs) {
  const toml = parseToml(readFileSync(join(cogsDir, d, "cog.toml"), "utf8"));
  const cog = toml.cog || {};
  const id = cog.id || d;
  const mapsRaw = MAPS_TO[id] || [];
  const maps_to = mapsRaw.filter((pid) => {
    const ok = catIds.has(pid);
    if (!ok) console.warn(`  ! ${id}: maps_to '${pid}' not in catalog — dropped`);
    return ok;
  });
  const storeId = toml["config.base_store_id"] && toml["config.base_store_id"].default;
  // Primary sensor: the first verified mapped part that is a catalog module (module > chip). Its
  // real product name becomes the cog's headline everywhere. Sensorless cogs get null/null.
  const sensor_id = maps_to.find((pid) => moduleName.has(pid)) || null;
  const sensor_name = sensor_id ? moduleName.get(sensor_id) : null;
  cogs.push({
    id,
    name: cog.name || id,
    category: cog.category || "",
    version: cog.version || "",
    description: cog.description || "",
    store_id: storeId != null ? String(storeId) : null,
    hardware_requirement: cog.hardware_requirement || [],
    bind_port: toml.api && typeof toml.api.bind_port === "number" ? toml.api.bind_port : null,
    binary: cog.binary || "",
    maps_to,
    sensor_id,
    sensor_name,
  });
}

writeFileSync(outPath, JSON.stringify({ schema: 1, source: basename(cogsDir), generated: new Date().toISOString().slice(0, 10), cogs }, null, 2) + "\n");
console.log(`wrote ${outPath}: ${cogs.length} cogs (${cogs.filter((c) => c.maps_to.length).length} mapped to catalog parts)`);
