-- WeftOS Sensor Explorer — tie cogs to their real sensor.
-- A cog's headline should be the REAL PRODUCT NAME of the catalog sensor it reads (its primary
-- mapped module), not the internal cog slug. These columns carry that mapped module's id + name
-- so /api/cogs, the MCP cog tools and the UI can lead with the real name without re-parsing `data`.
-- Populated during seeding (scan-cogs.mjs -> gen-cogs-seed.mjs). Null for sensorless cogs.

ALTER TABLE cogs ADD COLUMN sensor_id   TEXT;
ALTER TABLE cogs ADD COLUMN sensor_name TEXT;
