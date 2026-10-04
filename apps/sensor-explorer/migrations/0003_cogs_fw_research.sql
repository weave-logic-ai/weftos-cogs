-- WeftOS Sensor Explorer — findability migration.
-- Adds the cog registry, a curated firmware registry, the "create a cog" request queue and the
-- research queue, plus a stable per-item `hash` on the curated `parts` and the imported `pool`.
--
-- hash = 'wh_' + hex(SHA-256('<type>:<id>'))[:16]  (for pool, type='pool', id=mpn).
-- Computed during seeding (gen scripts, Node crypto); the Worker can recompute via Web Crypto.

ALTER TABLE parts ADD COLUMN hash TEXT;
ALTER TABLE pool  ADD COLUMN hash TEXT;
CREATE INDEX IF NOT EXISTS idx_parts_hash ON parts(hash);
CREATE INDEX IF NOT EXISTS idx_pool_hash  ON pool(hash);

-- Cog registry: one row per cog scanned from the cogs-bridge source tree. `maps_to` is a JSON
-- array of the catalog part ids the cog works with (a cog with no clean part still appears here).
CREATE TABLE IF NOT EXISTS cogs (
  id          TEXT PRIMARY KEY,
  name        TEXT,
  category    TEXT,
  version     TEXT,
  description TEXT,
  store_id    TEXT,                 -- base store id when the cog declares one, else null
  hardware    TEXT,                 -- JSON array: hardware_requirement
  bind_port   INTEGER,              -- [api] bind_port, else null
  maps_to     TEXT,                 -- JSON array of catalog part ids
  search      TEXT,                 -- lowercased haystack for LIKE search
  hash        TEXT,                 -- wh_… over 'cog:<id>'
  data        TEXT NOT NULL         -- full JSON record
);
CREATE INDEX IF NOT EXISTS idx_cogs_category ON cogs(category);

-- Curated firmware registry: device → firmware image we know about, mapped to catalog parts.
CREATE TABLE IF NOT EXISTS firmware (
  id          TEXT PRIMARY KEY,
  name        TEXT,
  device      TEXT,
  version     TEXT,
  description TEXT,
  repo        TEXT,
  maps_to     TEXT,                 -- JSON array of catalog part ids
  search      TEXT,
  data        TEXT NOT NULL
);

-- "Create a cog" requests raised from a part with no mapped cog.
CREATE TABLE IF NOT EXISTS cog_requests (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  part_id   TEXT,
  part_hash TEXT,
  note      TEXT,
  status    TEXT NOT NULL DEFAULT 'requested',   -- requested | triaged | built | declined
  created   TEXT
);

-- Research queue — the backlog a future research agent will consume.
CREATE TABLE IF NOT EXISTS research (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  ref_hash  TEXT,
  ref_type  TEXT,                   -- part | pool | cog
  ref_id    TEXT,
  note      TEXT,
  status    TEXT NOT NULL DEFAULT 'queued',       -- queued | claimed | done
  created   TEXT
);
CREATE INDEX IF NOT EXISTS idx_research_status ON research(status);
