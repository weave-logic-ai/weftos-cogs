-- WeftOS Sensor Explorer — signed-download serving layer (COG-008).
-- Turns the cog registry from a browsable listing into a signed binary source a WeaveLogic
-- cog-source / the on-device cogrepo cog can install from, speaking the weft-cog-repo
-- signed-install contract (pinned WeaveLogic pubkey 6aae63e0…).
--
-- cog_versions  — one row per (cog, version); `channel` selects a release train and `yanked`
--                 soft-pulls a bad build without deleting its artifacts.
-- cog_artifacts — one row per (cog, version, target); carries the R2 object key, byte size,
--                 sha256, the Ed25519 signature over the binary, the signer pubkey and the
--                 per-target manifest key. `bytes` holds the binary INLINE and is served when
--                 the R2 bucket is not bound (account-level R2 is currently off — see
--                 wrangler.jsonc; the download endpoint streams from R2 the moment it is bound,
--                 and falls back to this column until then). The 711 KB ld1040c binaries sit
--                 well under D1's 2 MB per-row cap.

CREATE TABLE IF NOT EXISTS cog_versions (
  cog_id   TEXT NOT NULL,
  version  TEXT NOT NULL,
  channel  TEXT NOT NULL DEFAULT 'stable',
  yanked   INTEGER NOT NULL DEFAULT 0,
  created  TEXT,
  PRIMARY KEY (cog_id, version)
);
CREATE INDEX IF NOT EXISTS idx_cog_versions_cog ON cog_versions(cog_id);

CREATE TABLE IF NOT EXISTS cog_artifacts (
  cog_id        TEXT NOT NULL,
  version       TEXT NOT NULL,
  target        TEXT NOT NULL,          -- aarch64 | armv7
  r2_key        TEXT NOT NULL,          -- R2 object key == the COG-008 registry path
  size          INTEGER NOT NULL,       -- byte size of the binary
  sha256        TEXT NOT NULL,          -- lowercase hex
  sig           TEXT NOT NULL,          -- Ed25519 signature over the binary, lowercase hex
  signer_pubkey TEXT NOT NULL,          -- pinned WeaveLogic release pubkey, lowercase hex
  manifest_key  TEXT,                   -- R2 object key of the per-target manifest JSON
  bytes         BLOB,                   -- inline binary, served while R2 is unbound (fallback)
  created       TEXT,
  PRIMARY KEY (cog_id, version, target)
);
CREATE INDEX IF NOT EXISTS idx_cog_artifacts_cog ON cog_artifacts(cog_id);
