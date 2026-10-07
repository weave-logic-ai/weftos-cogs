-- One row naming the canonical catalog this database was seeded from.
-- version is the catalog `generated` field. digest is SHA-256 of the raw catalog.json bytes.
-- The production database is not the canonical catalog. Reseed from the repository file.

CREATE TABLE IF NOT EXISTS catalog_release (
  id          INTEGER PRIMARY KEY CHECK (id = 1),
  version     TEXT NOT NULL,
  digest      TEXT NOT NULL,
  generated   TEXT NOT NULL,
  source_path TEXT NOT NULL,
  schema      INTEGER NOT NULL
);
