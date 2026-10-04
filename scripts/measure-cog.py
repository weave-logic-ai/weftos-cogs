#!/usr/bin/env python3
"""Measure a running sensor cog from its HTTP export — REAL numbers, no guessing.

Polls a cog's /raw ring and /status over a window, de-dups samples by timestamp, and reports the
actual update rate, value range, noise floor (stddev at rest), and event counters. Emits a Markdown
block to paste into the guide's "Measured" page, stamped with the time + source so it is never
confused with a vendor datasheet claim.

Usage:
    scripts/measure-cog.py http://203.0.113.10:8050 --seconds 30 --label "RD-03E @ 1 m, still room"
    scripts/measure-cog.py http://127.0.0.1:8049 --seconds 20          # local simulate, to prove it

It records exactly what the cog emits; if the cog is in no_source it says so and exits non-zero.
"""
import argparse, json, math, sys, time, urllib.request


def get(url, timeout=4):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.loads(r.read().decode())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base", help="cog export base, e.g. http://203.0.113.10:8050")
    ap.add_argument("--seconds", type=float, default=30.0)
    ap.add_argument("--label", default="", help="rig/context note, e.g. 'RD-03E @ 1 m, still room'")
    ap.add_argument("--poll-ms", type=int, default=250)
    a = ap.parse_args()
    base = a.base.rstrip("/")

    try:
        st0 = get(base + "/status")
    except Exception as e:
        print(f"cannot reach {base}/status: {e}", file=sys.stderr)
        return 2
    if st0.get("status") in ("no_source", "no_frames", "starting", None):
        print(f"cog is '{st0.get('status')}' — connect the sensor and start it before measuring.", file=sys.stderr)
        return 1

    samples = {}  # t_ms -> v, de-duped across polls of the decimated ring
    t_end = time.time() + a.seconds
    first = None
    last_status = st0
    while time.time() < t_end:
        try:
            raw = get(base + "/raw")
            for s in raw.get("samples", []):
                samples[int(s["t_ms"])] = float(s["v"])
            last_status = get(base + "/status")
            if first is None:
                first = last_status
        except Exception:
            pass
        time.sleep(a.poll_ms / 1000.0)

    if len(samples) < 2:
        print("not enough samples captured (is /raw serving a trace?)", file=sys.stderr)
        return 1

    ts = sorted(samples)
    vs = [samples[t] for t in ts]
    span_s = (ts[-1] - ts[0]) / 1000.0
    rate = (len(ts) - 1) / span_s if span_s > 0 else float("nan")
    n = len(vs)
    mean = sum(vs) / n
    stddev = math.sqrt(sum((v - mean) ** 2 for v in vs) / n)
    vmin, vmax = min(vs), max(vs)

    # Event counters (present both the first and last status so drift over the window is visible).
    def ev(s):
        for k in ("total_detections", "total_events", "total_motion_events"):
            if k in s:
                return k, s[k]
        return None, None
    ek, e0 = ev(first or st0)
    _, e1 = ev(last_status)
    events = (e1 - e0) if (e0 is not None and e1 is not None) else None

    stamp = time.strftime("%Y-%m-%d %H:%M %Z")
    print(f"## Measured\n")
    print(f"> Captured {stamp} from `{base}` over {span_s:.1f} s"
          + (f" — {a.label}" if a.label else "") + ". Real readings from this cog, not a datasheet.\n")
    print("| Quantity | Value |")
    print("|---|---|")
    print(f"| Update rate (trace) | **{rate:.1f} Hz** ({n} samples in {span_s:.1f} s) |")
    print(f"| Value range | {vmin:.3f} … {vmax:.3f} |")
    print(f"| Mean | {mean:.3f} |")
    print(f"| Noise floor (stddev) | {stddev:.4f} |")
    if events is not None:
        print(f"| Events during window | {events} ({ek}) |")
    print(f"| Reported status | {last_status.get('status')} |")
    # Carry through any cog-specific scalar fields for context (distance_cm, gyro_mag_dps, etc.).
    for k in ("distance_cm", "state_name", "gyro_mag_dps", "temp_c", "events_per_min",
              "detections_per_min", "motion_events_per_min", "source"):
        if k in last_status:
            print(f"| {k} (last) | {last_status[k]} |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
