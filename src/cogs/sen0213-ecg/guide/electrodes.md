# Electrodes

> Where to put the three pads, how to prepare the skin, and what an upside-down signal means.

The sensor has three leads: `RA` (right arm), `LA` (left arm) and `RL` (right leg, the reference). Read the labels on the snaps. The AD8232 measures the difference between RA and LA, and RL steadies the reading.

## Placements

```diagram
placements
```

The picture is a front view: the subject's right is on the viewer's left.

| Placement | Pads | Use it when |
|---|---|---|
| **Chest, lead I** (default) | RA below the right collarbone, LA below the left collarbone, RL on the right lower ribs | The best starting point, seated or lying. Least muscle noise. |
| **Chest, lead II** | RA below the right collarbone, LA on the left lower ribs (below the heart), RL on the right lower ribs | R-waves are small, or T-waves look as tall as R-waves. Lead II usually gives the biggest R-wave. |
| **Wrists** | RA inside the right wrist, LA inside the left wrist, RL on the right ankle or right forearm | A quick check with no undressing. Expect more noise. Rest your arms and don't grip anything. |

Keep the pads close to the heart and on the correct sides of it.

## Prepare the skin

1. Pick the placement and gather three fresh pads.
2. Clean each spot with alcohol and let it dry. Shave a little hair if the pad won't stick.
3. Peel the pads, stick them down and press firmly for several seconds.
4. Clip the leads on, matching the `RA`, `LA` and `RL` labels.
5. Wait a minute or two for the gel to settle before you judge the signal.
6. Sit or lie still and relaxed with your arms resting. Don't talk.

## Tips for a clean signal

- Use fresh gel pads. Dry or reused pads are the usual cause of noise.
- Twist or bundle the three lead wires together. This cuts the pickup of mains hum.
- Keep the leads away from mains cables and chargers.
- A good `RL` contact reduces hum. Don't skip the reference pad.
- Tape or hold the lead snaps so they don't tug on the pads.

## Upside-down R-waves

If the R-waves point down, `RA` and `LA` are swapped. Swap the two pads and check again. The app's **Lead polarity** step turns green when the median R amplitude is positive.

Fix it before you judge R amplitude or SNR, because both read negative or misleading with the pads swapped.

## Gotchas

- `leads_off` means more than 5% of samples sit near 0 V or 3.3 V. The pad has lifted or lost contact.
- `flat` means the signal barely moves. Check the sensor power and the `A` wire. → see **Troubleshoot**
- Walking, talking and gripping add muscle noise (EMG). Rest first.
- Run the cog from a battery or power bank while pads are on a person. → see **Safety**
