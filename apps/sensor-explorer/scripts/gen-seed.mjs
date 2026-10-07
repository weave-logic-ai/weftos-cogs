// Generate seed.sql for D1 from the canonical catalog.
// One INSERT per module/chip/project into `parts`, plus one `catalog_release` row whose
// digest is SHA-256 of the raw catalog file. The default input is
// crates/cog-market/catalog/catalog.json. Do not keep a second snapshot.
// Usage: node scripts/gen-seed.mjs [catalog.json] [seed.sql]
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const defaultIn = resolve(here, "../../../crates/cog-market/catalog/catalog.json");
const [, , inPath = defaultIn, outPath = "seed.sql"] = process.argv;
const bytes = readFileSync(inPath);
const digest = createHash("sha256").update(bytes).digest("hex");
const cat = JSON.parse(bytes.toString("utf8"));
const version = String(cat.generated || "");
const schema = Number(cat.schema);
if (!version || !Number.isInteger(schema) || !/^[0-9a-f]{64}$/.test(digest)) {
  console.error("catalog identity missing: need generated, integer schema, and a sha256 digest");
  process.exit(1);
}
const q = (s) => "'" + String(s ?? "").replaceAll("'", "''") + "'";
const hay = (...xs) => xs.flat().filter(Boolean).join(" ").toLowerCase();
// Stable item hash: wh_ + hex(SHA-256('<type>:<id>'))[:16].
const itemHash = (type, id) => "wh_" + createHash("sha256").update(`${type}:${id}`).digest("hex").slice(0, 16);

const rows = [];
function row(id, type, name, vendor, kind, category, tags, search, obj) {
  rows.push(
    `INSERT OR REPLACE INTO parts (id,type,name,vendor,kind,category,tags,search,data,status,updated,hash) VALUES (` +
      [q(id), q(type), q(name), q(vendor), q(kind), q(category), q(tags.join(" ")), q(search), q(JSON.stringify(obj)), q("published"), q(cat.generated || ""), q(itemHash(type, id))].join(",") +
      `);`
  );
}

for (const m of cat.modules || []) {
  const iface = (m.spec && (m.spec.uart ? "uart" : m.spec.i2c ? "i2c" : "")) || "";
  const tags = [m.kind, iface, ...(m.chips || [])].filter(Boolean);
  const search = hay(m.name, m.vendor, m.kind, m.summary, Object.values(m.spec || {}), m.good_for, m.not_for, m.chips, m.notes);
  row(m.id, "module", m.name, m.vendor || "", m.kind || "sensor", "", tags, search, m);
}
for (const c of cat.chips || []) {
  const tags = [...(c.tags || [])];
  const search = hay(c.name, c.manufacturer, c.role, c.summary, Object.values(c.spec || {}), c.tags);
  row(c.id, "chip", c.name, c.manufacturer || "", "", c.role || "", tags, search, c);
}
for (const p of cat.projects || []) {
  const search = hay(p.name, p.category, p.difficulty, p.summary, p.modules);
  row(p.id, "project", p.name, "", "", p.category || "", p.modules || [], search, p);
}

const sourcePath = "crates/cog-market/catalog/catalog.json";
const header =
  `-- Generated from ${sourcePath} (${cat.modules?.length || 0} modules, ${cat.chips?.length || 0} chips, ${cat.projects?.length || 0} projects)\n` +
  `-- catalog version ${version} sha256 ${digest}\n` +
  `INSERT OR REPLACE INTO catalog_release (id, version, digest, generated, source_path, schema) VALUES (` +
  [1, q(version), q(digest), q(version), q(sourcePath), schema].join(",") +
  `);\n`;
writeFileSync(outPath, header + rows.join("\n") + "\n");
console.log(`wrote ${outPath}: ${rows.length} rows version ${version} digest ${digest}`);
