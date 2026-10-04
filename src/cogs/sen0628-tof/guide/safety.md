# Safety

> Short rules. The sensor is a laser device and is not a safety system.

This is a hobby and research sensor. Don't rely on it where a missed detection could hurt someone.

## The laser

The SEN0628 uses a VL53L7CX, which emits invisible infrared laser light. The VL53L7CX is classified as a Class 1 laser product, which is eye-safe in normal use. That classification comes from the chip's documentation. Check the label and documents that came with your board.

- Don't stare into the emitter, and don't look at it through lenses or other optics.
- Don't take the board apart or modify the optics.

## Not a safety-rated detector

- **Don't use it for machine guarding, light curtains or interlocks.**
- **Don't use it for life safety**: fall detection that others depend on, fire or gas protection, or monitoring people who need help.
- It can miss a target: glass, mirrors, sunlight, and very dark or shiny surfaces all cause missing or wrong readings. → see **Mounting**
- The 8x8 grid is coarse. A small or thin object can sit between zones.
- Presence is an inference against a learned background. It can be wrong if the background was learned with people in view.

## Mounting

- Fix it so it cannot fall. Use screws or a bracket rather than tape, especially overhead.
- Don't mount it where a falling sensor could hurt someone, and keep the cable short enough not to hang.
- Add strain relief so a pulled cable does not drag the sensor off its mount.
- Keep the Seed and wiring dry and out of reach of small children.

## Handling

- Power it from 3.3 V, and avoid the 5 V pins for this build. → see **Wiring**
- Power off before you change wiring.
- Handle the board by its edges. Static can damage the chip.
- Keep the lens clean and dry. A smudge looks like a close object.

## Privacy

The export carries depth maps, which show where people are. It listens on all interfaces by default. On a network you don't trust, set `api_bind` to `127.0.0.1:8047`.

## What the cog won't do

- It won't invent a reading: it reports `no_source`, `no_frames` and `no_targets` instead of guessing.
- It doesn't identify people or objects. It reports distances only.
