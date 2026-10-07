import assert from "node:assert/strict";
import test from "node:test";
import { catalogRelease, clampLimit, tokenize } from "../src/search.ts";

test("tokenize lowercases and splits on non-alphanumeric", () => {
  assert.deepEqual(tokenize("QM33120W TR13"), ["qm33120w", "tr13"]);
  assert.deepEqual(tokenize("a--b"), ["a", "b"]);
  assert.deepEqual(tokenize(""), []);
});

test("catalogRelease returns null when the release table is missing", async () => {
  const missing = {
    prepare() {
      return { async first() { throw new Error("D1_ERROR: no such table: catalog_release: SQLITE_ERROR"); } };
    },
  } as unknown as D1Database;
  assert.equal(await catalogRelease(missing), null);
});

test("catalogRelease returns the row and rethrows other database errors", async () => {
  const row = { version: "2026-10-05", digest: "abc", generated: "2026-10-05", source_path: "crates/cog-market/catalog/catalog.json", schema: 1 };
  const present = { prepare() { return { async first() { return row; } }; } } as unknown as D1Database;
  assert.equal(await catalogRelease(present), row);
  const down = { prepare() { return { async first() { throw new Error("D1_ERROR: network"); } }; } } as unknown as D1Database;
  await assert.rejects(() => catalogRelease(down), /network/);
});

test("clampLimit stays inside 1..max", () => {
  assert.equal(clampLimit(undefined, 30, 200), 30);
  assert.equal(clampLimit(0, 30, 200), 30);
  assert.equal(clampLimit(-5, 30, 200), 1);
  assert.equal(clampLimit("nope", 30, 200), 30);
  assert.equal(clampLimit(10, 30, 200), 10);
  assert.equal(clampLimit(999, 30, 200), 200);
});
