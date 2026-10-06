# Host licence how-to still describes the in-process check

- **State:** blocked
- **Observed:** 2026-10-06, Doc, during the public COG successor write
- **Rank:** 1 — a builder following the how-to will treat the in-process licence directory as the product contract, which COG-106 replaced
- **Waiting on:** the lane that adds the Cog Host `cog.check_run` client on the WeftOS protocol branch. Do not rewrite the how-to until that client exists. The extraction instruction said the client is not in this checkout.

## What is wrong

[docs/cogs/weft-licence.md](../cogs/weft-licence.md), section "Running cogs on the Seed", still says Cog Host reads its own licence directory and enforces the start check itself (WeftOS ADR-106's in-process gate).

[COG-106](../decisions/COG-106-host-licence.md) says Cog Host asks the local daemon for `cog.check_run` on the Unix socket `weftos.cog.v1` and does not decide the grant in-process.

## Evidence

- COG-106, decision section, recorded this session.
- At `056029567eb8`, a search for `weftos.cog.v1`, `cog.check_run`, `daemon_unavailable`, `malformed_reply`, and `version_mismatch` in `*.rs`, `*.md`, and `*.toml` returned no matches. The socket client was not in the tree.
- `crates/weftos-cog-host/src/licence.rs` (lines 1–8 and 44–48 at that commit) still calls `check_run` through an in-process gate. Not quoted further here: COG-106 describes the decision, not that path.

## Not done

The how-to was not rewritten. Rewriting it now would document a client that is not in this checkout.
