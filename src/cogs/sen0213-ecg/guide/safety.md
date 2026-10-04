# Safety

> Short rules. Follow all of them whenever pads are on a person.

**This is not a medical device.** It is a hobby and research sensor. Every report carries `medical: false`.

## Rules

1. **Battery power only while pads are on a person.** The AD8232 is not isolated. Power the Seed from a battery or a USB power bank, not from a mains-connected laptop or charger.
2. **Don't use it on anyone with an implanted electronic device**, such as a pacemaker or defibrillator.
3. **Don't use it for diagnosis or treatment.** The numbers are not validated. Don't act on them for health decisions, and see a clinician about any health concern.
4. **Don't treat a low `quality` as a finding.** It means "don't trust the numbers". It does not mean arrhythmia.
5. **Keep the signal private.** The export is ECG data. It listens on all interfaces by default. On a network you don't trust, set `api_bind` to `127.0.0.1:8046`.

## Good practice

- Check the wiring with the pads **off** first. Connect the pads last.
- Use only 3.3 V. Never connect 5 V to the ADS1115 for this build.
- Use fresh pads. Stop if the skin becomes red or sore.
- Unplug the leads from the Seed before you touch or move the wiring.
- Don't run it on a wet person or in a wet place.

## What the cog won't do

- It won't give a heart rate unless the leads are on and beats were found.
- It flags `leads_off`, `flat` and `no_source` instead of guessing.
- It reports one lead. One lead can't tell you about the whole heart.

## In an emergency

Take the pads off and call your local emergency number. Don't rely on this device.
