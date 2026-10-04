# Glossary

> Plain-word definitions of the terms used in this guide.

| Term | Meaning |
|---|---|
| **AD8232** | The single-lead ECG amplifier chip on the SEN0213. It amplifies and filters the signal to 0-3.3 V, idling near mid-supply. |
| **ADS1115** | A 16-bit I2C analog-to-digital converter. It reads the sensor's voltage for the Seed. |
| **PGA** | Programmable gain amplifier. The cog sets the ADS1115 to ±4.096 V, which is 125 µV per step (LSB). |
| **SPS** | Samples per second. The ADS1115 runs at 860 SPS; the cog reads it at 250 Hz by default (100-500). |
| **Lead I** | The voltage difference between the right and left arms (or collarbones). The default placement. |
| **Lead II** | The difference between the right arm and the left leg (or left lower ribs). Usually gives a taller R-wave. |
| **RA / LA / RL** | Right arm, left arm, and right leg (the reference). The three electrode leads. |
| **R-peak** | The tall spike in each heartbeat. The cog finds its time. |
| **QRS** | The sharp group of waves caused by the ventricles contracting. The R-peak is its tallest part. |
| **RR interval** | The time between two R-peaks, in ms. Intervals outside 300-2000 ms are dropped. |
| **Heart rate** | 60000 divided by the median RR over the last 10 s, in beats per minute. |
| **SDNN** | Standard deviation of the RR intervals, in ms. A measure of heart rate variability, over the last 60 s. |
| **RMSSD** | Root mean square of successive RR differences, in ms. Another variability measure. |
| **Notch filter** | A filter that removes one narrow frequency. The cog uses it at 50 or 60 Hz. |
| **Mains hum** | 50 or 60 Hz interference from power wiring. It shows up as a regular ripple. |
| **Baseline wander** | Slow drift of the signal from breathing, movement or poor contact. |
| **EMG** | Electrical noise from muscles. It shows as fuzzy spikes. |
| **SNR** | Signal-to-noise ratio, in dB. Here: R amplitude against the noise floor. Higher is better. |
| **Pan-Tompkins** | The classic 1985 R-peak detection method: band-pass, derivative, squaring, moving-window integration, adaptive thresholds. |
| **Leads-off** | Electrodes not making contact. The signal pins to 0 V or 3.3 V. |
| **I2C** | A two-wire bus. The Seed talks to the ADS1115 on bus 1. |
| **SDA / SCL** | The I2C data and clock wires. SDA is pin 3, SCL is pin 5. |
| **Cog** | A small app that runs on a Cognitum Seed. This one is `sen0213-ecg`. |
| **Seed** | The Cognitum appliance. Ours is a Raspberry Pi Zero 2 W. |
| **Sideload** | Installing a cog by copying it to the Seed yourself, instead of from the store. |
