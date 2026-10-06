-- WeftOS Sensor Explorer — D1 schema.
-- One `parts` table holds modules, chips and projects (type column); the full record is JSON in
-- `data`, with a few promoted columns for fast facet/search queries. Links (module.chips,
-- project.modules) live inside the JSON. Contributions land in `contributions` (pending) until
-- approved; `api_keys` gates the MCP/write surface.

CREATE TABLE IF NOT EXISTS parts (
  id       TEXT NOT NULL,
  type     TEXT NOT NULL,                -- 'module' | 'chip' | 'project'
  name     TEXT,
  vendor   TEXT,                         -- vendor (module) / manufacturer (chip)
  kind     TEXT,                         -- module kind: board|sensor|display|actuator
  category TEXT,                         -- project category / derived sensing category
  search   TEXT,                         -- lowercased haystack for LIKE search
  tags     TEXT,                         -- space-joined tags/interfaces for facets
  data     TEXT NOT NULL,                -- full JSON record
  status   TEXT NOT NULL DEFAULT 'published',  -- published | pending
  updated  TEXT,
  PRIMARY KEY (type, id)                 -- ids are unique within a type; a module and a chip may share one
);
CREATE INDEX IF NOT EXISTS idx_parts_type ON parts(type);
CREATE INDEX IF NOT EXISTS idx_parts_kind ON parts(kind);
CREATE INDEX IF NOT EXISTS idx_parts_vendor ON parts(vendor);

CREATE TABLE IF NOT EXISTS api_keys (
  key     TEXT PRIMARY KEY,
  owner   TEXT,
  scope   TEXT NOT NULL DEFAULT 'read',  -- read | contribute | admin
  created TEXT
);

CREATE TABLE IF NOT EXISTS contributions (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  part_id  TEXT,
  type     TEXT,
  data     TEXT NOT NULL,
  author   TEXT,
  created  TEXT,
  status   TEXT NOT NULL DEFAULT 'pending'  -- pending | approved | rejected
);
