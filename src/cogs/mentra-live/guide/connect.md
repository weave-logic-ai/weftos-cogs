# Connect the glasses

The cog talks to the glasses with `adb`. You need `adb` installed on the companion and the glasses reachable by it.

## 1. Make sure adb can see the glasses

**On USB** — plug the glasses in and check:

```
adb devices
```

You should see a line ending in `device` (not `unauthorized` or `offline`). If it says `unauthorized`, accept the debugging prompt on the glasses.

**Over Wi-Fi (ADB-over-TCP)** — with the glasses and companion on the same network:

```
adb connect 192.168.1.50:5555      # the glasses' ip:port
adb devices
```

The cog will run `adb connect` for you when you pass `--addr`, but doing it once by hand confirms the address first.

> `adb` lookup order: the `--adb` flag, then the `$ADB` env var, then the Android SDK platform-tools location, then `adb` on `$PATH`. Pass `--adb /path/to/adb` if yours is somewhere unusual.

## 2. Run the cog

Point it at the glasses and at the weft-cog-host that owns the fleet:

```
# USB, autodetect the single attached device:
cog-mentra-live --interval 5 --host http://203.0.113.10:9480

# a specific USB device:
cog-mentra-live --serial ABCD1234 --host http://203.0.113.10:9480

# over Wi-Fi:
cog-mentra-live --addr 192.168.1.50:5555 --host http://203.0.113.10:9480
```

`--host` defaults to the `WEFTOS_HOST` environment variable, then `http://127.0.0.1:9480` (use that when the cog runs on cog0 next to the host). `--node-id` sets the fleet id the glasses join as (default `mentra-01`).

## 3. Confirm it joined

The cog prints a JSON snapshot each interval with `"joined": true` once a heartbeat succeeds. On the host, the fleet roster should list the node as online with its battery. When the glasses go away, the node ages out to offline on its own.
