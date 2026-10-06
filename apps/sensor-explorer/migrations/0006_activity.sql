-- Opens, MCP reads, and other catalog attention. Missing tables must not break the site:
-- writers catch "no such table" until this migration is applied.

CREATE TABLE IF NOT EXISTS events (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  kind     TEXT NOT NULL,             -- view | mcp | research | cog_request | contribution
  tool     TEXT,                      -- MCP tool name when kind = mcp
  ref_type TEXT,                      -- part | pool | cog | catalog
  ref_id   TEXT,
  note     TEXT,
  created  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_ref ON events(ref_id, id);
CREATE INDEX IF NOT EXISTS idx_events_kind ON events(kind, id);

CREATE TABLE IF NOT EXISTS part_stats (
  ref_id    TEXT PRIMARY KEY,
  views     INTEGER NOT NULL DEFAULT 0,
  mcp_reads INTEGER NOT NULL DEFAULT 0,
  last_at   TEXT
);
