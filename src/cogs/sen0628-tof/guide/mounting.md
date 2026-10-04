# Mounting

> Where to put the sensor, how it sees, and what to avoid. Placement matters more than any setting.

## Which way is which

```diagram
grid
```

The map is how the sensor sees it when you look out of the lens toward the scene. Zones are numbered row by row: X runs left to right and Y runs top to bottom. Index = `y * 8 + x` in 8x8 mode, and `y * 4 + x` in 4x4 mode.

- Zone 0 is the top-left corner. Zone 63 is the bottom-right corner (zone 15 in 4x4 mode).
- Which edge of the board is "top" is not something we have verified. Check any arrow or marking on the board, and then confirm it: hold your hand in at one side and see which column the app lights up.
- Left, middle and right sectors use columns. In 8x8 mode: columns 0 to 2, 3 to 4, and 5 to 7. In 4x4 mode: column 0, columns 1 to 2, and column 3.

## How much it sees

The view is 60 x 60 degrees, so the zones grow with distance. The width covered at distance `d` is about `2 x d x tan(30 degrees)`, roughly `1.15 x d`.

| Distance | Width covered | 8x8 zone size |
|---|---|---|
| 0.5 m | about 0.58 m | about 7 cm |
| 1 m | about 1.15 m | about 14 cm |
| 2 m | about 2.3 m | about 29 cm |
| 3.5 m | about 4.0 m | about 50 cm |

A 4x4 zone is twice as wide and twice as tall. Keep the area you care about within 3.5 m of the sensor.

## Where to put it

| Placement | Looks | Good for |
|---|---|---|
| Ceiling | Straight down | Occupancy and counting people |
| Wall, chest or head height | Level, across the room | Approach and presence |

- **Mount rigidly.** A sensor that sways looks like motion. Screw it down and add strain relief to the cable.
- **Keep a clear view.** Nothing in front of the lens, no cable or bracket in the corners of the view.
- **Level it for wall mounts.** Use the flat-wall check below.
- **Keep the target area in range.** Anything beyond 3.5 m reads as no target.

## What to avoid

- **Glass or windows in front.** A cover glass over the sensor causes crosstalk and false near readings. Don't put it behind a window.
- **Direct sunlight on the scene.** Infrared from the sun adds noise and shortens the range.
- **Very dark or absorbent surfaces.** They return little light and drop out.
- **Very shiny surfaces and mirrors.** They bounce the beam away, or add false readings from reflections.

Watch the valid share in the app. A low valid share usually means one of these.

## Check it is level: the flat wall

Point the sensor square-on at a flat wall about 1 m away. The app's levelling check compares the zones. Distances should be nearly equal across the grid.

- If one side reads clearly longer, the sensor is turned toward that side.
- The outer zones, and most of all the corners, read a little longer. That is expected, by about `1/cos(angle)` for the zone's angle off the axis, about 11 % for the middle of an edge column (about 26 degrees off axis) and about 22 % at the corners (about 35 degrees), if the sensor reports distance along each zone's direction (not verified on hardware). It is not a tilt.

## Learn the background with the scene empty

Presence is measured against a background learned from the first `learn_seconds` of frames after the cog starts. Each zone's median becomes the background.

1. Place the sensor.
2. Make sure nobody is in view, and nothing is moving.
3. Start or restart the cog and wait for `learn_seconds` (default 5).

If you move the sensor or rearrange the room, restart the cog to learn again. → see **Calibrate**
