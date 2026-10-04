# Placement

> Wall-mount the radar at 1.5 to 2 m facing the area, check which way its x axis points, then map its targets into the room with the mount position and heading.

## Mounting

Hi-Link's instruction manual (V1.00, §7):

- **Wall mounting** is the typical installation.
- **Height 1.5 to 2 m.** The manual's coverage drawing is for a 1.5 m mount.
- **Range up to 6 m**, **±60°** either side of the line straight out from the antenna, **±35°** up and down.
- **Face the antenna at the area**, with nothing in front of it.
- **Mount it rigidly.** A radar that shakes sees motion everywhere.
- **Watch what is behind it.** Radar passes through walls, so movement behind the radar can show up. A metal plate behind it reduces this.
- **Avoid moving clutter:** fans, swinging curtains, plants in a draught, and large shiny surfaces.
- **Keep other 24 GHz radars from facing it** directly.

The manual gives no tilt angle. Mount the board vertical, with the antenna facing straight out and level, unless you have a reason to tilt it. **Not yet verified on our radar:** how coverage changes with a downward tilt.

Whatever you choose, **measure the tilt** (a phone inclinometer app on the board's face is enough) and record it as `pitch_deg`: 0 when level, negative when tilted down. The beam is ±35° tall, so near the radar it covers a thin slice of height and far away it spans floor to ceiling; a downward tilt also means the bottom of the beam reaches the floor some distance out, and anything the radar reports past where the top of the beam meets the floor is floor clutter. The spatial engine handles all of this once it knows the height and tilt.

A cover is fine if it has no metal or conductive coating. The manual's guidance is a flat cover of even thickness, ideally 12.4 or 18.6 mm in front of the antenna.

## The radar's frame

The cog reports targets in `radar_local`, in metres:

- **y** points straight out from the antenna face (the boresight). Hi-Link's user guide says y is always positive. The cog drops a target with y at or below 0 as corrupt.
- **x** is sideways, along the wall.
- **Speed** is in m/s, signed.

**Unverified, check on the first run:** which side x is positive, and whether positive speed means moving toward or away from the radar. Hi-Link's documents show the signs but not their direction. Stand 2 m out, step to your right as seen from the radar looking out, and see whether `x_m` goes up. Then walk straight away and see the sign of `speed_mps`. Record both with the mount.

## Mapping into the room

The spatial engine (WeftOS ADR-107) uses `room_enu`: metres east (x) and north (y) from the room's south-west floor corner. Yaw is measured the mathematical way: degrees **counter-clockwise from east (+x)**, so 0 faces east, 90 faces north, 180 west and 270 south. To place a radar there, record:

- **(px, py):** where the radar is mounted, in metres.
- **yaw:** the direction its antenna faces, in degrees counter-clockwise from east.
- **height and tilt:** the antenna height above the floor (z) and `pitch_deg` from above.
- **side:** +1 if `x_m` grew when you stepped to your right in the check above, −1 if it shrank.

Then, for each target, with `x' = side × x_m` (metres to the radar's right) and `f = y_m × cos(pitch)` (the radar measures along its tilted beam; this is the distance along the floor):

```
east  = px + f × cos(yaw) + x' × sin(yaw)
north = py + f × sin(yaw) − x' × cos(yaw)
```

The cog does this for you with `--radar-pose px,py,z,yaw,pitch,side`.

Check: a radar on the west wall facing east has yaw 0, so a target 2 m out and 0.5 m to its right is at `east = px + 2`, `north = py − 0.5`. A radar on the south wall facing north has yaw 90, so the same target is at `east = px + 0.5`, `north = py + 2`.

The radar's position is 2-D. If you need it, use the mount height as z; the radar does not report target height.

## Placement checks

```
cog-ld2450-radar --once            # nobody in view: target_count 0
```

- **Empty room, 0 targets.** If targets appear with nobody there, look for moving clutter or something moving behind the wall.
- **One person, 1 target.** Walk the edges of the area; `targets` should follow you out to about 6 m and ±60°.
- **Distance check.** Stand at a taped spot and compare `hypot(x_m, y_m)` with a tape measure. Record what you find; we have no measured accuracy figure yet.
