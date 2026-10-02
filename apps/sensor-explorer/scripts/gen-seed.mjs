// Generate seed.sql for D1 from a catalog.json. One INSERT per module/chip/project into `parts`,
// with promoted columns (name/vendor/kind/category/search/tags) and the full record in `data`.
// Usage: node scripts/gen-seed.mjs catalog.seed.json seed.sql
import { readFileSync, writeFileSync } from "node:fs";

const [, , inPath = "catalog.seed.json", outPath = "seed.sql"] = process.argv;
const cat = JSON.parse(readFileSync(inPath, "utf8"));
const q = (s) => "'" + String(s ?? "").replaceAll("'", "''") + "'";
const hay = (...xs) => xs.flat().filter(Boolean).join(" ").toLowerCase();

const rows = [];
function row(id, type, name, vendor, kind, category, tags, search, obj) {
  rows.push(
    `INSERT OR REPLACE INTO parts (id,type,name,vendor,kind,category,tags,search,data,status,updated) VALUES (` +
      [q(id), q(type), q(name), q(vendor), q(kind), q(category), q(tags.join(" ")), q(search), q(JSON.stringify(obj)), q("published"), q(cat.generated || "")].join(",") +
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

const header = `-- Generated from ${inPath} (${cat.modules?.length || 0} modules, ${cat.chips?.length || 0} chips, ${cat.projects?.length || 0} projects)\n`;
writeFileSync(outPath, header + rows.join("\n") + "\n");
console.log(`wrote ${outPath}: ${rows.length} rows`);
