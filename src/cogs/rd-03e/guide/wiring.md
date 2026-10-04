# Wiring

> Four wires: 3.3 V, GND, and the radar's TX into the Pi's RX. The config line is optional. Trust the labels, not the colours.

The RD-03E speaks **3.3 V TTL UART at 256000 8N1**. Bring it into the Seed one of two ways:

- **Pi UART header** — the four pins below, straight onto the 40-pin header. This is the tidy option.
- **A 3.3 V USB-serial adapter** — plug the module into an FTDI/CP2102-style 3.3 V adapter and it shows up as `/dev/ttyUSB0`. Use this if the Pi UART is busy or you are on a laptop. **Must be a 3.3 V adapter** — a 5 V one can damage the module.

Power the module from **3.3 V (pin 1)**. Never use the 5 V pins (2 and 4): the RD-03E is a 3.3 V part and the Pi's GPIO is not 5 V tolerant.

## Find pin 1

```diagram
header
```

![Pi Zero 2 W / Pi 5 header pinout](pizero2w-pinout.jpg)

Hold the Seed with the **40-pin header along the top edge** and the ports along the **bottom**, micro-SD at the **left**. Pin 1 (3.3 V) is the **left pin of the inner row** (the row nearer the chip); the outer row (nearer the top edge) carries the even pins 2-40.

> **See your exact board.** Open the zoomable, interactive pinout: <https://pinout.xyz/>.

## The wiring diagram

```diagram
wiring
```

## Pin by pin

| Module pin | Seed pin | Typical wire | Why |
|---|---|---|---|
| `VCC` | pin 1 (3V3) | red | power, 3.3 V only |
| `GND` | pin 6 (GND) | black | ground |
| `TX` | pin 10 (GPIO15 RXD) | green | **radar data** — the module talks, the Pi listens |
| `RX` | pin 8 (GPIO14 TXD) | yellow | config only — the Pi talks, the module listens |

**Data = module TX → Pi RXD.** **Config = module RX → Pi TXD.** The data direction is the one that matters: the module streams its report frames out of its `TX`, so that wire goes to the Pi's **receive** pin (pin 10). The cog only reads, so the config wire (module `RX` → pin 8) is optional — leave it off and the cog still works.

Colours are a convention only.

## Enable the Pi UART

The Pi's serial port is used by the login console out of the box. Free it for the radar:

```
sudo raspi-config nonint do_serial_cons 1   # disable the serial login console
sudo raspi-config nonint do_serial_hw 0     # enable the serial hardware
sudo reboot
```

After reboot, `/dev/serial0` points at the UART on pins 8/10. If you used a USB-serial adapter instead, skip this — set the cog's `--port` to `/dev/ttyUSB0`.

**Final check.** Start the cog and open the app. The step **radar streaming** goes green once the first `AA .. .. .. 55` frame decodes.
