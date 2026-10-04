# Wiring

> Four wires: 3.3 V power, ground, and the sensor's TX into the Pi's RX. Trust the labels, not the colours.

The AS201 talks **3.3 V TTL UART at 115200 8N1** (adjustable 4800–921600). Bring it in either through the **Seed's own UART header pins** or through a **3.3 V USB-to-serial adapter** (so it appears as `/dev/ttyUSB0`). Power it from **3.3 V on pin 1** — never the 5 V pins (2 and 4): the AS201 is a 3.3 V part and the Pi's GPIO is not 5 V tolerant.

## Find pin 1

```diagram
header
```

![Pi Zero 2 W / Pi 5 header pinout](pizero2w-pinout.jpg)

Hold the Seed with the **40-pin header along the top edge** and the ports along the bottom, micro-SD at the **left**. Pin 1 (3.3 V) is the **left pin of the inner row** (the row nearer the chip); the outer row (nearer the top edge) carries the even pins 2–40.

> **See your exact board.** Open the interactive, zoomable pinout: <https://pinout.xyz/>.

## The wiring diagram

```diagram
wiring
```

## Pin by pin

| From | To | Typical wire |
|---|---|---|
| Sensor `VCC` | Seed pin 1 (3V3) — **not** 5 V | red |
| Sensor `GND` | Seed pin 6 (GND) | black |
| Sensor `TX` | Seed pin 10 (GPIO15 RXD) | green |
| Sensor `RX` | Seed pin 8 (GPIO14 TXD) | yellow |

**Direction matters:** **data = sensor TX → Pi RXD**, and **config = sensor RX → Pi TXD**. Only the sensor's TX → Pi RXD is needed to read the stream; the sensor RX wire is only used when you want to reconfigure the sensor.

Over a USB-serial adapter instead, the two data wires cross the same way: sensor TX → adapter RX, sensor RX → adapter TX, and the device appears as `/dev/ttyUSB0`.

## Enable the Pi UART

The Pi's primary UART is used by the serial login console on the stock image. Free it for the sensor:

```
sudo raspi-config nonint do_serial_hw 0     # enable the serial port hardware
sudo raspi-config nonint do_serial_cons 1   # disable the serial login console
sudo reboot
```

After reboot, `/dev/serial0` points at the UART on pins 8/10. (If you use a USB-serial adapter you can skip this — use `--port /dev/ttyUSB0`.)

## Mounting

Mount the sensor **flat and still** for a clean zero reference. The gyro is sensitive to vibration, so a rigid, decoupled mount gives the quietest readings.
