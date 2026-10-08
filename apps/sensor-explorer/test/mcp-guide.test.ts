import assert from "node:assert/strict";
import test from "node:test";
import { mcpGuidePage } from "../src/mcp-guide.ts";

test("the MCP guide tells people and agents how to add a pending item", () => {
  const html = mcpGuidePage("Sensor Explorer");
  assert.match(html, /<!doctype html>/);
  assert.match(html, /POST \/mcp/);
  assert.match(html, /request_new_item/);
  assert.match(html, /schema":true/);
  assert.match(html, /pending/);
  assert.match(html, /contribute/);
  assert.match(html, /prompt injection/);
  assert.match(html, /video links/);
  assert.match(html, /href="\/"/);
  assert.equal(html.includes("BOOTSTRAP_API_KEY") && html.includes("sk_"), false);
  assert.doesNotMatch(html, /Bearer [A-Za-z0-9]{16,}/);
});

test("the guide names the caller, and stays readable on a narrow viewport", () => {
  const html = mcpGuidePage("A & B <labs>");
  assert.match(html, /A &amp; B &lt;labs&gt;/);
  assert.match(html, /max-width:640px/);
  assert.match(html, /viewport/);
});
