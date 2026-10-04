# Set up the Seed

> Install the signed cog, start it, confirm `/status`. Set `--port` only if it isn't autodetected.

## Install the cog

The easiest way is the **WeftOS console marketplace**: install `rd-03e` (it is Ed25519-signed; the host verifies it before it lands). Or sideload it directly:

```
/var/lib/cognitum/apps/rd-03e/cog-rd-03e-arm
/var/lib/cognitum/apps/rd-03e/manifest.json
```

If you are using the Pi UART (pins 8/10) rather than a USB-serial adapter, enable it first — see **Wiring → Enable the Pi UART**.

## Start it

From the console (the Start button), from the Seed's app API, or by hand:

```
cog-rd-03e --interval 1                 # continuous, one report per second
cog-rd-03e --once --simulate            # no hardware: synthetic frames
```

The cog brings up its export on `:8050` first, so the app can connect and watch `no_source` turn into a live report while you finish wiring.

## How it flows

```diagram
flow
```

## Confirm it's reading

Hit the status endpoint on the Seed:

```
curl http://<seed>:8050/status
```

You want `"status":"present"` or `"status":"clear"` with a `distance_cm`. If you see `"status":"no_frames"` the port opened but nothing decoded yet; `"status":"no_source"` means the port could not be opened at all — see **Troubleshoot**.

## Set the port

The cog autodetects a USB-serial adapter (it tries `/dev/ttyUSB0`, `/dev/ttyACM0`, and the common `tty.usbserial*` names). If yours lands elsewhere — or you wired the Pi UART and want `/dev/serial0` — name it:

```
cog-rd-03e --interval 1 --port /dev/serial0
```
