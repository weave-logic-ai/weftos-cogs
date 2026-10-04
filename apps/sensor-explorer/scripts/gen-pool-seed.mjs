// pool JSON ({parts:{mpn:record}}) -> pool-seed.sql for D1.
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
const [, , inPath = "pool.seed.json", outPath = "pool-seed.sql"] = process.argv;
const pool = JSON.parse(readFileSync(inPath, "utf8"));
const parts = pool.parts || {};
const q = (s) => "'" + String(s ?? "").replaceAll("'", "''") + "'";
const hay = (p) => [p.mpn, p.manufacturer, p.name, p.category, p.summary].filter(Boolean).join(" ").toLowerCase();
// Stable item hash: wh_ + hex(SHA-256('pool:<mpn>'))[:16].
const itemHash = (mpn) => "wh_" + createHash("sha256").update(`pool:${mpn}`).digest("hex").slice(0, 16);
const rows = [];
for (const p of Object.values(parts)) {
  const buy = (p.buy || [])[0] || {};
  rows.push(
    `INSERT OR REPLACE INTO pool (mpn,manufacturer,name,category,search,datasheet,price,url,source,data,status,hash) VALUES (` +
      [q(p.mpn), q(p.manufacturer), q(p.name), q(p.category), q(hay(p)), q(p.datasheet), q(buy.price), q(buy.url), q(p.source), q(JSON.stringify(p)), q(p.status || "imported"), q(itemHash(p.mpn))].join(",") +
      `);`
  );
}
writeFileSync(outPath, `-- ${rows.length} pool parts from ${inPath}\n` + rows.join("\n") + "\n");
console.log(`wrote ${outPath}: ${rows.length} rows`);
