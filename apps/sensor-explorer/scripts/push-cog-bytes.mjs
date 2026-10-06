// Push cog binary bytes into the serving layer, PUT /api/cogs/:id/artifact?target=…
// The Worker re-hashes the body and refuses it unless the sha256 matches the cog_artifacts row,
// so a mismatched or truncated upload can never land. Bytes are stored in R2 when that bucket is
// bound, else inline in D1 (the fallback used while account-level R2 is off).
//
// Reads the bearer key from $BOOTSTRAP_API_KEY, else from .dev.vars — the key is NEVER printed.
// Usage: node scripts/push-cog-bytes.mjs <registry-dir> [base-url]
//   registry-dir : a COG-008 repo dir containing registry.json + cogs/<arch>/<binary>
//   base-url     : default https://sensor-explorer.wfscifi.workers.dev
import { readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";

const TARGET = { arm: "armv7", arm64: "aarch64", x86_64: "x86_64", amd64: "x86_64" };
const [, , dir, base = "https://sensor-explorer.wfscifi.workers.dev"] = process.argv;
if (!dir) { console.error("usage: node scripts/push-cog-bytes.mjs <registry-dir> [base-url]"); process.exit(1); }

function bearer() {
  if (process.env.BOOTSTRAP_API_KEY) return process.env.BOOTSTRAP_API_KEY.trim();
  const dv = join(process.cwd(), ".dev.vars");
  if (existsSync(dv)) {
    const m = readFileSync(dv, "utf8").match(/^\s*BOOTSTRAP_API_KEY\s*=\s*(.+)\s*$/m);
    if (m) return m[1].trim().replace(/^["']|["']$/g, "");
  }
  console.error("no key: set $BOOTSTRAP_API_KEY or put BOOTSTRAP_API_KEY in .dev.vars");
  process.exit(1);
}

const key = bearer();
const reg = JSON.parse(readFileSync(join(dir, "registry.json"), "utf8"));
let ok = 0, fail = 0;

for (const cog of reg.cogs || []) {
  for (const [arch, a] of Object.entries(cog.artifacts || {})) {
    const target = TARGET[arch] || arch;
    const body = readFileSync(join(dir, a.path));
    const url = `${base}/api/cogs/${encodeURIComponent(cog.id)}/artifact?target=${target}`;
    const res = await fetch(url, {
      method: "PUT",
      headers: { authorization: `Bearer ${key}`, "content-type": "application/octet-stream" },
      body,
    });
    const txt = await res.text();
    if (res.ok) { console.log(`✓ ${cog.id} ${target} (${body.length} B): ${txt}`); ok++; }
    else { console.error(`✗ ${cog.id} ${target}: HTTP ${res.status} ${txt}`); fail++; }
  }
}
console.log(`done: ${ok} uploaded, ${fail} failed`);
process.exit(fail ? 1 : 0);
