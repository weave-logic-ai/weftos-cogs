import assert from "node:assert/strict";
import test from "node:test";
import { clampLimit, tokenize } from "../src/search.ts";

test("tokenize lowercases and splits on non-alphanumeric", () => {
  assert.deepEqual(tokenize("QM33120W TR13"), ["qm33120w", "tr13"]);
  assert.deepEqual(tokenize("a--b"), ["a", "b"]);
  assert.deepEqual(tokenize(""), []);
});

test("clampLimit stays inside 1..max", () => {
  assert.equal(clampLimit(undefined, 30, 200), 30);
  assert.equal(clampLimit(0, 30, 200), 30);
  assert.equal(clampLimit(-5, 30, 200), 1);
  assert.equal(clampLimit("nope", 30, 200), 30);
  assert.equal(clampLimit(10, 30, 200), 10);
  assert.equal(clampLimit(999, 30, 200), 200);
});
