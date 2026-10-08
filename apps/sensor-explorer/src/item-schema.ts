// Catalog item schema for MCP `request_new_item`.
// Field names follow crates/cog-market/src/hw.rs (Project, Module, Chip, BuyLink, DocLink, Firmware).
// A submission is a pending contribution. This module does not publish and does not stamp a digest.

const ID = { type: "string", minLength: 1, maxLength: 80, pattern: "^[a-z0-9]+(-[a-z0-9]+)*$" };
const NAME = { type: "string", minLength: 1, maxLength: 200 };
const TEXT = { type: "string", maxLength: 4000 };
const STRING_LIST = { type: "array", items: TEXT };
const SPEC = { type: "object", additionalProperties: { type: "string" } };

const buyLink = {
  type: "object",
  additionalProperties: false,
  required: ["vendor"],
  properties: {
    vendor: NAME,
    price: TEXT,
    url: TEXT,
    ships_from: TEXT,
  },
};

const docLink = {
  type: "object",
  additionalProperties: false,
  required: ["label", "url"],
  properties: { label: NAME, url: TEXT },
};

const firmware = {
  type: "object",
  additionalProperties: false,
  properties: {
    version: TEXT,
    read_with: TEXT,
    read_config_key: TEXT,
    update: TEXT,
    url: TEXT,
    notes: TEXT,
  },
};

const project = {
  type: "object",
  additionalProperties: false,
  required: ["id", "name"],
  properties: {
    id: ID,
    name: NAME,
    category: TEXT,
    difficulty: TEXT,
    summary: TEXT,
    store_id: { type: "integer", minimum: 0 },
    export_port: { type: "integer", minimum: 0, maximum: 65535 },
    simulate: { type: "boolean" },
    guide: TEXT,
    modules: STRING_LIST,
    hash: TEXT,
  },
};

const moduleItem = {
  type: "object",
  additionalProperties: false,
  required: ["id", "name"],
  properties: {
    id: ID,
    name: NAME,
    vendor: TEXT,
    kind: { type: "string", enum: ["board", "sensor", "display", "actuator"] },
    summary: TEXT,
    good_for: STRING_LIST,
    not_for: STRING_LIST,
    notes: STRING_LIST,
    spec: SPEC,
    pins: STRING_LIST,
    chips: STRING_LIST,
    photo: TEXT,
    datasheet: TEXT,
    mouser_query: TEXT,
    buy: { type: "array", items: buyLink },
    seen_in: STRING_LIST,
    cogs: STRING_LIST,
    firmware,
    docs: { type: "array", items: docLink },
    hash: TEXT,
  },
};

const chip = {
  type: "object",
  additionalProperties: false,
  required: ["id", "name"],
  properties: {
    id: ID,
    name: NAME,
    manufacturer: TEXT,
    role: TEXT,
    tags: STRING_LIST,
    summary: TEXT,
    spec: SPEC,
    mouser_query: TEXT,
    datasheet: TEXT,
    hash: TEXT,
  },
};

/** The full item schema an agent can request. Hash is left empty; review assigns it. */
export const CATALOG_ITEM_SCHEMA = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  $id: "weftos.catalog.item",
  title: "WeftOS hardware catalog item",
  description: "One Project, Module, or Chip as defined by crates/cog-market/src/hw.rs. Leave hash empty. A reviewer publishes the row and sets hash to wh_ plus the first 16 hex chars of sha256(type:id). Video links are refused by the intake filter. A datasheet URL is a document link.",
  type: "object",
  additionalProperties: false,
  required: ["type", "item"],
  properties: {
    type: { type: "string", enum: ["project", "module", "chip"] },
    item: { oneOf: [project, moduleItem, chip] },
  },
  $defs: {
    Project: project,
    Module: moduleItem,
    Chip: chip,
    BuyLink: buyLink,
    DocLink: docLink,
    Firmware: firmware,
  },
};

const ID_RE = /^[a-z0-9]+(-[a-z0-9]+)*$/;

export function wantsSchema(args: any): boolean {
  return args?.schema === true || args?.request === "schema";
}

export function itemRecord(args: any): { type: string; item: Record<string, unknown> } | { error: string } | null {
  const item = args?.item ?? args?.part;
  if (item == null) return null;
  if (!item || typeof item !== "object" || Array.isArray(item)) return { error: "item must be an object" };
  const type = String(args?.type || "");
  if (type !== "project" && type !== "module" && type !== "chip") return { error: "type must be project, module, or chip" };
  const id = typeof item.id === "string" ? item.id.trim() : "";
  const name = typeof item.name === "string" ? item.name.trim() : "";
  if (!ID_RE.test(id) || id.length > 80) return { error: "item.id must be a lowercase hyphenated id" };
  if (!name || name.length > 200) return { error: "item.name is required" };
  return { type, item: item as Record<string, unknown> };
}
