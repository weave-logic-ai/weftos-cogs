import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

const script = join(import.meta.dirname, "../scripts/publish-catalog-artifact.mjs");

function publish(raw: string, out: string) {
  const dir = mkdtempSync(join(tmpdir(), "catalog-artifact-"));
  const src = join(dir, "in.json");
  writeFileSync(src, raw);
  const run = spawnSync(process.execPath, [script, src, out], { encoding: "utf8" });
  return { dir, run };
}

test("publish copies the raw catalog bytes and records their sha256", () => {
  const out = mkdtempSync(join(tmpdir(), "catalog-out-"));
  // The space after the first colon must survive. Re-serializing would drop it.
  const raw = '{ "schema": 1, "generated": "2026-10-05", "projects": [], "modules": [], "chips": [] }\n';
  const { dir, run } = publish(raw, out);
  assert.equal(run.status, 0, run.stderr);
  const copied = readFileSync(join(out, "catalog.json"), "utf8");
  assert.equal(copied, raw);
  const digest = createHash("sha256").update(copied).digest("hex");
  const rel = JSON.parse(readFileSync(join(out, "catalog-release.json"), "utf8"));
  assert.equal(rel.digest, digest);
  assert.equal(rel.version, "2026-10-05");
  assert.equal(rel.generated, "2026-10-05");
  assert.equal(rel.schema, 1);
  assert.equal(rel.source_path, "crates/cog-market/catalog/catalog.json");
  rmSync(dir, { recursive: true });
  rmSync(out, { recursive: true });
});

test("publish rejects a catalog with no generated version", () => {
  const out = mkdtempSync(join(tmpdir(), "catalog-out-"));
  const { dir, run } = publish('{ "schema": 1 }\n', out);
  assert.notEqual(run.status, 0);
  rmSync(dir, { recursive: true });
  rmSync(out, { recursive: true });
});
