# Set up the Seed

> Install and start the bridge cog, then point a sender at it.

## 1. Install the bridge on the Seed

```bash
scripts/cross-build.sh bridge
scripts/seed-sideload.sh bridge genesis@<seed>
```

## 2. Start it

```bash
curl -X POST http://<seed>/api/v1/apps/bridge/start
```

It listens on `127.0.0.1:8048` by default (this Seed only). To take readings from other nodes, give it an allowlist and set `api_bind` to `0.0.0.0:8048` (see **Security**); without an allowlist or token it refuses to start on a non-loopback address. Confirm (on the Seed, since the default bind is loopback):

```bash
ssh genesis@<seed> curl -s http://127.0.0.1:8048/status
# once api_bind is 0.0.0.0:8048, from anywhere:  curl http://<seed>:8048/status
```

A fresh bridge reports zero sources. The Seed agent serves plain HTTP on port 80 with permissive CORS; the bridge's own port (8048) does too.

## 3. Run a cog on the other board

Any of our cogs works as a source, because they all export a `/status` that now includes the 8-number store vector. For example, on a Pi 5 running the ToF cog:

```bash
weft-tof-scope  # or run cog-sen0628-tof directly with --api-bind 0.0.0.0:8047
```

## 4. Forward with weft-bridge-send

On the same board, run the sender (it lives in the WeftOS repo; build with `scripts/build.sh scope`-style tooling or `cargo build -p weftos-bridge-send`):

```bash
weft-bridge-send \
  --from http://127.0.0.1:8047 \
  --to   http://<seed>:8048 \
  --source pi5 \
  --interval 1
```

`--from` is the source cog's export, `--to` is the bridge, `--source` names this node. The sender polls the cog's `/status`, pulls the vector and a few metrics, and posts them to the bridge.

## 5. Watch it arrive

```bash
curl http://<seed>:8048/status    # the source appears with a store id and a rising count
curl http://<seed>:8048/sources   # the (node, cog) -> store id table
```

## Test without a second board

`--once --simulate` injects one synthetic reading, so you can prove the bridge writes to the store with nothing else running:

```bash
curl -X POST http://<seed>/api/v1/apps/bridge/console -d '{"command":"--once --simulate"}'
```

## Upgrading from 0.1

0.1 listened on `0.0.0.0:8048` with no authentication by default. 0.2 listens on `127.0.0.1:8048`, so after an upgrade **remote nodes stop reaching the bridge** unless the config names a network address. Whether your install keeps its old address depends on whether `api_bind` was stored in the cog's config (a config `PUT` stores every key you send) or was left to the default; the 0.2 default applies only in the second case. The bridge logs a `NOTICE` at start when it is loopback-only but has auth configured, and `/status` shows `bind_scope`.

0.2 does not refuse to start and does not guess the old address: a cog that exits is restarted by the agent in a loop, and the bridge cannot tell an explicit `api_bind` from a default, so a migration could re-open the network by guessing. The safe default (loopback) stays, and the network is an explicit opt-in.

To keep taking remote readings, send the whole config with:

1. `api_bind = 0.0.0.0:8048`, and **either**
2. `allowlist = <path>` with each sender's key (see **Security**), the recommended way; **or**, for senders that cannot sign yet (the current `weft-bridge-send`),
3. `token = <secret>` and `allow_shared_token = true` (deprecated; set it with the `BRIDGE_TOKEN` environment variable if you can, since `--token` shows in `ps`).

Without one of 2 or 3 a network bind refuses to start. Then check the first remote reading with `curl http://<seed>:8048/status` (counts only unless you are on the Seed or sign the request).
