// Generate cogs-seed.sql from cogs.seed.json + firmware.seed.json for D1.
// Rows go into `cogs` and `firmware` (migration 0003). Cogs get a stable item hash
// (wh_ + hex(SHA-256('cog:<id>'))[:16]) so the research queue can reference them.
// Usage: node scripts/gen-cogs-seed.mjs [cogs.seed.json] [firmware.seed.json] [cogs-seed.sql]
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";

const [, , cogsPath = "cogs.seed.json", fwPath = "firmware.seed.json", outPath = "cogs-seed.sql"] = process.argv;
const q = (s) => "'" + String(s ?? "").replaceAll("'", "''") + "'";
const hay = (...xs) => xs.flat().filter(Boolean).join(" ").toLowerCase();
const itemHash = (type, id) => "wh_" + createHash("sha256").update(`${type}:${id}`).digest("hex").slice(0, 16);

const cogs = JSON.parse(readFileSync(cogsPath, "utf8")).cogs || [];
const fw = JSON.parse(readFileSync(fwPath, "utf8")).firmware || [];
const rows = [];

for (const c of cogs) {
  // The real sensor name is part of the haystack so searching the product name finds the cog.
  const search = hay(c.id, c.name, c.sensor_name, c.category, c.description, c.hardware_requirement, c.maps_to, c.binary);
  rows.push(
    "INSERT OR REPLACE INTO cogs (id,name,category,version,description,store_id,hardware,bind_port,maps_to,sensor_id,sensor_name,search,hash,data) VALUES (" +
      [
        q(c.id), q(c.name), q(c.category), q(c.version), q(c.description),
        c.store_id == null ? "NULL" : q(c.store_id),
        q(JSON.stringify(c.hardware_requirement || [])),
        c.bind_port == null ? "NULL" : String(c.bind_port),
        q(JSON.stringify(c.maps_to || [])),
        c.sensor_id == null ? "NULL" : q(c.sensor_id),
        c.sensor_name == null ? "NULL" : q(c.sensor_name),
        q(search), q(itemHash("cog", c.id)), q(JSON.stringify(c)),
      ].join(",") + ");"
  );
}

for (const f of fw) {
  const search = hay(f.id, f.name, f.device, f.description, f.maps_to);
  rows.push(
    "INSERT OR REPLACE INTO firmware (id,name,device,version,description,repo,maps_to,search,data) VALUES (" +
      [
        q(f.id), q(f.name), q(f.device), q(f.version), q(f.description), q(f.repo),
        q(JSON.stringify(f.maps_to || [])), q(search), q(JSON.stringify(f)),
      ].join(",") + ");"
  );
}

writeFileSync(outPath, `-- ${cogs.length} cogs + ${fw.length} firmware images\n` + rows.join("\n") + "\n");
console.log(`wrote ${outPath}: ${cogs.length} cogs, ${fw.length} firmware`);
