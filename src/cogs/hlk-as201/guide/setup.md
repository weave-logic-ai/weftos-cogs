# Set up the Seed

> Install the cog, start it, confirm a live attitude, then set the port and motion threshold if needed.

## Install the cog

The easiest way is the **WeftOS console marketplace**: install `hlk-as201` (it is Ed25519-signed; the host verifies it before it lands). Or sideload it directly:

```
/var/lib/cognitum/apps/hlk-as201/cog-hlk-as201-arm
/var/lib/cognitum/apps/hlk-as201/manifest.json
```

## Start it

From the console (the Start button), from the Seed's app API, or by hand:

```
cog-hlk-as201 --interval 1                      # continuous, one report every second
cog-hlk-as201 --once --simulate                 # no hardware: synthetic AS201 frames
```

The cog brings up its export on `:8051` first, so a companion app can connect and watch `no_source` turn into a live attitude while you finish wiring.

## Confirm /status

Once it is running, check the live report:

```
curl http://<seed>:8051/status
```

`status` should read `moving` or `still` and `euler_deg` should change as you tilt the sensor. If it reads `no_source` or `no_frames`, go to **Troubleshoot**.

## Set the port and baud

- Default serial device is `/dev/ttyUSB0`. On the Pi UART header use `--port /dev/serial0`.
- The AS201 default speed is 115200. If your unit was configured differently, set `--baud` (4800–921600).

```
cog-hlk-as201 --port /dev/serial0 --baud 115200
```

## Motion threshold

The `--motion` option is the angular-rate magnitude (deg/s) above which the sensor counts as **moving**. Raise it if a still sensor reports `moving` from noise; lower it to catch gentler motion.

```
cog-hlk-as201 --motion 25
```

## How the signal flows

```diagram
flow
```
