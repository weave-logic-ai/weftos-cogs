# Changelog

Product tags of this repository. Crate versions are not required to match the tag.
The import marker remains `weftos-0.8.3-import.1`. Wire protocol remains `weftos.cog.v1`.

## v0.1.1

- Sensor Explorer serves `GET /mcp`, a guide for people and agents: how to connect, the key scopes, the tool list, and how a proposal stays pending until a person approves it.
- New cog `bme280` for the catalog module `bme280` (Bosch BME280, I2C 0x76/0x77). Compensation follows Bosch driver v3.5.1 integer formulas. The catalog file is unchanged. Artifacts from the cross-build are unsigned.

## v0.1.0

Sensor Explorer MCP:

- `request_new_item` returns the Project, Module, and Chip schema (including BuyLink, DocLink, and Firmware) when asked, and stores a filled record as a pending contribution.
- A deterministic intake filter screens MCP tool arguments before they are stored or queried. It refuses prompt injection, sexual content, video links, spam, and oversized input, and the refusal does not repeat the submitted text.
- `add_part` and `promote_from_pool` stay pending and pass through the same filter.

Signed release artifacts are not part of this tag. The cog signing key is not in this repository.
