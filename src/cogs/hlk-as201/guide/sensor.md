# The sensor

> Hi-Link HLK-AS201 — a 9/10-axis attitude sensor (accelerometer + gyroscope + magnetometer). An IMU, not a radar.

## What it is

An **inertial measurement unit**: 3-axis accelerometer, 3-axis gyroscope and
3-axis magnetometer (some variants add a barometer). It reports orientation
(roll / pitch / yaw), acceleration, turn rate and magnetic heading. The X/Y/Z
silkscreen is the giveaway that it's an IMU — a radar would label range/antenna.

## What it's good for

- **Orientation / tilt** — roll and pitch of whatever it's mounted to.
- **Motion & vibration** — turn rate spikes, bumps, handling, fall/impact events.
- **Heading** — compass bearing from the magnetometer (after calibration).
- **"Is it moving"** — the `moving` flag trips above the `--motion` threshold.

## What it's *not* for

- **Absolute position** — integrating acceleration drifts; it is not a GPS.
- **Trustworthy heading without a magnetometer calibration** (a figure-8 cal).
- **Trustworthy readings before the accel zero-bias calibration** (place flat + still).

## Datasheet spec (vendor, not our measurement)

| | |
|---|---|
| Axes | 3× accel, 3× gyro, 3× mag + barometer (10-axis); 32-bit DSP @ up to 240 MHz |
| UART | 115200 8N1 (adjustable 4800–921600), 3.3 V; also BLE 5.1 |
| Report | 20 Hz default; `FA FB · len · cmd · data · SUM · FC FD`, 42-byte ten-axis payload |
| Scaling | accel ×16/32768 g · gyro ×0.0625 °/s · euler ×180/32768° · mag ×0.00610 µT · quat ×1/32768 · temp ×0.01°C |

> **Protocol confirmed** against the Hi-Link **HLK-AS201 datasheet V1.1 (2025-08-01)** —
> <https://hlktech.net/index.php?id=1380>. The earlier WIT-fallback caveat no longer applies.

## What to expect from the output

- **Flat and still:** `accel_g` z ≈ **+1.0** (gravity), x/y ≈ 0; `gyro_dps` ≈ 0;
  `euler_deg` roll/pitch ≈ 0; `status` `still`.
- **Tilt it:** roll/pitch track the angle. **Spin it:** `gyro_mag_dps` jumps and
  `moving` flips true past `--motion`.
- `mag_ut` is magnetic field in µT; `mag_accuracy` 0 = uncalibrated/interference, 3 = calibrated.
- `pressure_hpa` and `height_m` come from the onboard barometer (10-axis parts).

## Calibration

1. **Accel zero-bias:** place it flat and still and send the datasheet's `0x1C` command. After
   success, the modulus of acceleration is ≈ 1.0 g in any static orientation.
2. **Magnetic heading:** `0x1D` to begin, rotate the module more than one full turn about each of
   X/Y/Z (finish within ~1 min), then `0x1E` to save. `mag_accuracy` reaching 3 means calibrated.
3. Note the small nonzero `gyro_mag_dps` at rest — that's the gyro bias and a good `--motion` floor.

## Spatial evidence: not applicable

This cog has no `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107). The only IMU record there, `imu_event` (`moved`), means "this sensing node's mount moved": the engine then marks evidence from that node's `source_id` stale. The AS201 reports the attitude of whatever it is attached to, usually a worn or handheld object. Its motion is not a node moving, and if it is worn it is a person's movement, which ADR-107 keeps out of the map. Its readings also carry no position. Emitting them as `imu_event` would mark the wrong evidence stale.

What might fit later: an AS201 fixed to a sensing node's bracket (a radar or ToF mount), configured with that node's id, could send `imu_event` `moved` when its attitude changes past a threshold. Its tilt and magnetometer heading could also seed that node's `pose` record. The heading is a compass bearing (clockwise from north), so it needs converting to ADR-107's yaw (counter-clockwise from east) and correcting for declination. Neither exists yet.

## Measured

> Pending: wire this IMU and run
> `scripts/measure-cog.py http://<seed>:8051 --seconds 30 --label "AS201 flat & still"`.
> The resting gravity vector, gyro bias (noise floor), and `--motion` threshold
> that cleanly separates still from moving land here.
