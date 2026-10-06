-- The imported parts pool (JLCPCB/LCSC bulk, Mouser category pulls). Separate from `parts` (curated).
-- Promote a pool part into the curated catalog via a contribution.
CREATE TABLE IF NOT EXISTS pool (
  mpn          TEXT PRIMARY KEY,
  manufacturer TEXT,
  name         TEXT,
  category     TEXT,
  search       TEXT,
  datasheet    TEXT,
  price        TEXT,
  url          TEXT,
  source       TEXT,
  data         TEXT NOT NULL,
  status       TEXT NOT NULL DEFAULT 'imported'
);
CREATE INDEX IF NOT EXISTS idx_pool_cat ON pool(category);
CREATE INDEX IF NOT EXISTS idx_pool_mfr ON pool(manufacturer);
