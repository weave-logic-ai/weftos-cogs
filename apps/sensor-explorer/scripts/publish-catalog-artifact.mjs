// Publish the canonical catalog as static assets.
// public/catalog.json is a byte copy. public/catalog-release.json names its version and SHA-256.
// Both are generated. They are not a second source.
// Usage: node scripts/publish-catalog-artifact.mjs [catalog.json] [outDir]
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const defaultIn = resolve(here, "../../../crates/cog-market/catalog/catalog.json");
const defaultOut = resolve(here, "../public");
const [, , inPath = defaultIn, outPath = defaultOut] = process.argv;
const bytes = readFileSync(inPath);
const digest = createHash("sha256").update(bytes).digest("hex");
const cat = JSON.parse(bytes.toString("utf8"));
const version = String(cat.generated || "");
const schema = Number(cat.schema);
if (!version || !Number.isInteger(schema) || !/^[0-9a-f]{64}$/.test(digest)) {
  console.error("catalog identity missing: need generated, integer schema, and a sha256 digest");
  process.exit(1);
}
mkdirSync(outPath, { recursive: true });
const catalogFile = resolve(outPath, "catalog.json");
const releaseFile = resolve(outPath, "catalog-release.json");
writeFileSync(catalogFile, bytes);
writeFileSync(releaseFile, JSON.stringify({
  version,
  digest,
  generated: version,
  source_path: "crates/cog-market/catalog/catalog.json",
  schema,
}) + "\n");
console.log(`wrote ${catalogFile} and ${releaseFile} version ${version} digest ${digest}`);
