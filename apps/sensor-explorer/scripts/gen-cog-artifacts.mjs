// Generate cog-artifacts.sql from a COG-008 signed registry.json (migration 0005).
// Emits one cog_versions row and one cog_artifacts row per (cog, target). Metadata only —
// the binary bytes are pushed separately with scripts/push-cog-bytes.mjs (or streamed from R2
// once that bucket is bound). The signer pubkey defaults to the pinned WeaveLogic release key.
// Usage: node scripts/gen-cog-artifacts.mjs [registry.json] [cog-artifacts.sql] [signer_pubkey]
import { readFileSync, writeFileSync } from "node:fs";

const PINNED_PUBKEY = "6aae63e067488f1e5414ad4a6b9536bef0407db210fb33a3b378e8d6d12eca15";
const [, , regPath = "registry.json", outPath = "cog-artifacts.sql", signer = PINNED_PUBKEY] = process.argv;

// COG-008 registry artifact keys → canonical cog targets.
const TARGET = { arm: "armv7", arm64: "aarch64", x86_64: "x86_64", amd64: "x86_64" };

const reg = JSON.parse(readFileSync(regPath, "utf8"));
const q = (s) => "'" + String(s ?? "").replaceAll("'", "''") + "'";
const now = new Date().toISOString();
const rows = [];
let nVer = 0, nArt = 0;

for (const cog of reg.cogs || []) {
  rows.push(
    "INSERT OR REPLACE INTO cog_versions (cog_id,version,channel,yanked,created) VALUES (" +
      [q(cog.id), q(cog.version), q("stable"), "0", q(now)].join(",") + ");"
  );
  nVer++;
  for (const [key, a] of Object.entries(cog.artifacts || {})) {
    const target = TARGET[key] || key;
    rows.push(
      "INSERT OR REPLACE INTO cog_artifacts (cog_id,version,target,r2_key,size,sha256,sig,signer_pubkey,manifest_key,created) VALUES (" +
        [
          q(cog.id), q(cog.version), q(target), q(a.path),
          String(a.size | 0), q(a.sha256), q(a.sig), q(signer),
          a.manifest_path == null ? "NULL" : q(a.manifest_path),
          q(now),
        ].join(",") + ");"
    );
    nArt++;
  }
}

writeFileSync(outPath, `-- ${nVer} cog versions + ${nArt} artifacts from ${regPath}\n` + rows.join("\n") + "\n");
console.log(`wrote ${outPath}: ${nVer} versions, ${nArt} artifacts`);
