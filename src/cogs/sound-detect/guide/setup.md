# Set up the Seed

> Enable I2C once, install the cog, start it. Then the app's checklist turns green top to bottom.

## Enable I2C

I2C is off on the stock image. On the Seed:

```
sudo raspi-config nonint do_i2c 0   # or: add dtparam=i2c_arm=on to /boot/firmware/config.txt
sudo reboot
```

After reboot, `/dev/i2c-1` exists. The cog's default bus is 1.

## Install the cog

The easiest way is the **WeftOS console marketplace**: install `sound-detect` (it is Ed25519-signed; the host verifies it before it lands). Or sideload it directly:

```
/var/lib/cognitum/apps/sound-detect/cog-sound-detect-arm
/var/lib/cognitum/apps/sound-detect/manifest.json
```

## Start it

From the console (the Start button), from the Seed's app API, or by hand:

```
cog-sound-detect --interval 5                 # continuous, one report every 5 s
cog-sound-detect --once --simulate            # no hardware: synthetic bursts
```

The cog brings up its export on `:8049` first, so the app can connect and watch `no_source` turn into a live level while you finish wiring.
