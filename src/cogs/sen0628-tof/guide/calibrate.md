# Calibrate

> Check the readings are good, then tune presence. Use the app's calibration view.

Work in this order. Each step builds on the one before it.

## 1. Valid zones

A zone is valid when its reading is from 20 mm up to `max_range_mm` (default 3500). A reading of 0 means no target. The report's `valid_pct` is the share of zones that are valid in the latest frame.

- In a normal room a good setup has most zones valid.
- A low share usually means glass, sunlight, a very dark or shiny surface, or a scene beyond range. → see **Mounting**
- A `valid_pct` of 0 gives the status `no_targets`.
- Lower `max_range_mm` to ignore a far wall. Raise it only up to the sensor's 3500 mm rating, or the extra readings are noise.

## 2. Levelling check

Point the sensor square-on at a flat wall and look at the app's levelling check. Distances should be near equal. Edge and corner zones read a little longer, which is expected. A one-sided slope means the sensor is turned. → see **Mounting**

## 3. Per-zone noise

`zone_noise_mm` is the standard deviation of each zone over the report window, in mm. It is `null` where a zone had fewer than 2 valid readings. `mean_zone_noise_mm` is the average.

- Point at a static scene and look for zones that are much noisier than the rest.
- Noisy zones are usually dark or shiny surfaces, sunlight, or an edge where two objects meet inside one zone.
- Rock-steady is not the goal. The sensor is accurate to about 11 mm up close and 5 % further out.

## 4. Background and presence

The cog learns a background from the first `learn_seconds` of frames (default 5): for each zone, the median reading. A zone is occupied when it is more than `presence_mm` closer than the background (default 150). Presence is on when at least 2 zones are occupied. A zone with a valid reading where the background had none also counts as occupied.

1. Empty the scene and start or restart the cog.
2. Wait for `background_learned` to turn `true`.
3. Walk into view. `presence` should turn on, and `occupied_zones` lists the zone indices.

**The background is learned once.** It does not update while the cog runs. Restart the cog (or save the config, which restarts it) to learn again.

Tune the margin:

| Symptom | Change |
|---|---|
| Presence on with nobody in view | Raise `presence_mm`, or relearn with the scene empty |
| Presence misses a person or a small object | Lower `presence_mm` toward 50 to 100 |
| Presence never turns on | Check `background_learned`. Relearn empty. |

The `presence_mm` range is 30 to 1000.

## 5. Sector distances

`sectors` holds the nearest valid distance in the left, middle and right columns, or `null` when a sector has none. In 8x8 mode: columns 0 to 2, 3 to 4 and 5 to 7. In 4x4 mode: column 0, columns 1 to 2 and column 3. Use them for steering or for telling which side something is on.

## 6. Frame rate

`rate_hz` is how many frames the cog reads per second (1 to 15, default 10). The sensor itself ranges at 15 to 60 Hz. Compare `frame_rate_hz` in the report with the setting. If it is well below, the Seed is busy. → see **Troubleshoot**

Motion (`motion_mm`) is the mean absolute change between consecutive frames over zones valid in both. A still scene is near the noise level.

## 7. Record

Use the app's recording to save frames as CSV for later analysis. The same data is at `http://<seed>:8047/raw.csv?seconds=N` (N up to 30).

## Change settings

Edit the cog's settings in the app, which uses the agent's config API. **A config write replaces the whole config and restarts the cog**, so the background is learned again. → see **API reference**
