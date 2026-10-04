# Calibrate

> What each measurement in the app means, what good looks like, and what to do about it.

The app measures the last 4 seconds of signal. The targets below are rules of thumb, not standards.

## Metrics

| Metric | What it is | Good looks like | If it's off |
|---|---|---|---|
| Baseline (V) | Mean of the raw signal. The AD8232 idles near mid-supply. | 1.0 to 2.3 V (about 1.65 V ideal) | Check the sensor's 3.3 V supply and the `A` to `A0` wire. |
| At the rails (%) | Share of samples within 0.05 V of 0 V or 3.3 V | 0% | Pads off or loose. Press firmly on clean skin. |
| Hum at 50 Hz and 60 Hz (mV) | Mains pickup on the raw signal, before the notch | Under 0.02 mV at both | Set the notch to the stronger frequency. Twist the leads, move away from cables. |
| R amplitude (mV) | Median height of the R-peaks | Positive, clearly above the noise | A negative value means RA and LA are swapped. Size depends on placement; lead II is usually taller. |
| Noise floor (mV) | Wide-band noise away from the QRS (muscle, electrode, ADC) | As low as you can get | Rest, reseat the pads, use fresh gel. |
| SNR (dB) | R amplitude against the noise floor | Higher is better; no hard target | Improve contact or placement. |
| SDNN, RMSSD (ms) | Beat-to-beat variability | Needs 3 or more intervals | Keep recording; hold still. |
| Sampling health | Late samples, I2C errors | `late_samples` near 0, no errors | → see **Troubleshoot** |

## Pick the mains notch

The cog applies a notch at `mains_hz` (60 Hz in the Americas, 50 Hz in Europe and most of Asia, 0 to turn it off).

1. Look at the hum readings for 50 Hz and 60 Hz in the app.
2. If both are under 0.02 mV, the hum is negligible and any notch is fine.
3. Otherwise the stronger one is your mains frequency. Press **Apply recommended notch**.
4. The cog restarts with the new setting. The **Mains notch** step turns green when the notch matches the hum.

The hum is measured before the notch, so the reading stays valid after you apply it.

## Check polarity

The **Lead polarity** step reads the median R amplitude. If it is negative, swap RA and LA. → see **Electrodes**

## Check quality

`quality` runs from 0 to 1. The app wants 0.8 or more.

- It is 0 unless the leads are on and at least two beats were found.
- It is lowered by irregular beat intervals. So a low score means "don't trust the numbers". It does not mean an arrhythmia, and it does not directly measure noise.
- If it stays low: keep still, check pad contact, and try lead II.

## Tap-along pulse check

1. Find your pulse at the wrist.
2. Tap the button (or press Space) with each beat. Four or more taps are needed. A pause over 3 s starts a new count.
3. Compare the tap rate with the cog's heart rate. They should agree within **5 bpm**: the app shows green inside that range, amber outside.

If the cog reads about double, it is probably counting T-waves. → see **Troubleshoot**

## Sampling health

- `late_samples` counts reads that missed their deadline, and `max_late_ms` is the worst lateness. Both should stay near zero.
- `read_errors` counts I2C errors. A rising count points at a loose wire.

## Record

- **CSV:** the last 60 s as `t_ms,raw_v,filtered_mv,r_peak`.
- **RuView reference:** beat-to-beat bpm as `timestamp_ms,value`, in RuView ADR-293's reference-series format. It is used to grade `wifi-densepose-vitals` heart rate as MEASURED.

Natively both go to files. In the browser they are copied to the clipboard.

## Change settings

Changes made in the app are written to the cog's config and restart it. The app merges your edit into the full config, because the agent's `PUT` replaces everything. → see **API reference**
