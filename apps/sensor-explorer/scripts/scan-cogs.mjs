// Scan the cogs-bridge source tree for cog.toml manifests and emit cogs.seed.json — the cog
// registry that makes cogs discoverable in the Sensor Explorer. Each record carries the manifest
// facts {id,name,category,version,description,store_id,hardware_requirement,bind_port} plus
// `maps_to`: the catalog part ids the cog works with (verified against catalog.seed.json).
//
// Usage: node scripts/scan-cogs.mjs [cogsDir] [catalog.seed.json] [cogs.seed.json]
//   cogsDir defaults to a local cog directory
import { readFileSync, writeFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";

const [, , cogsDirArg, catPath = "catalog.seed.json", outPath = "cogs.seed.json"] = process.argv;
const cogsDir = cogsDirArg || join(homedir(), "local-cogs");

// Which catalog part(s) each cog works with. Verified against the catalog below; a cog may map to
// nothing (network/app cogs) and still appear in the registry.
const MAPS_TO = {
  "rd-03e": ["rd-03e-module"],
  "hlk-as201": ["hlk-as201-module", "as201-imu"],
  "sen0628-tof": ["sen0628-tof", "dfrobot-sen0628-vl53l7cx", "vl53l7cx"],
  "sen0213-ecg": ["sen0213-ecg", "ad8232-board", "ad8232"],
  "sound-detect": ["ky-038", "inmp441-mic"],
  "mentra-live": ["mentra-live-glasses", "mentra-live-display", "mentra-live-microphone"],
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

// catalog ids, to verify maps_to
const cat = JSON.parse(readFileSync(catPath, "utf8"));
const catIds = new Set([...(cat.modules || []), ...(cat.chips || []), ...(cat.projects || [])].map((r) => r.id));

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
  });
}

writeFileSync(outPath, JSON.stringify({ schema: 1, source: cogsDir, generated: new Date().toISOString().slice(0, 10), cogs }, null, 2) + "\n");
console.log(`wrote ${outPath}: ${cogs.length} cogs (${cogs.filter((c) => c.maps_to.length).length} mapped to catalog parts)`);
