//! Decoders for the LD6002 target-position frames (`0x0A04`, `0x0A17`).
//!
//! Layouts were measured on this radar (see ADR-157), not taken from a vendor document:
//!
//! ```text
//! 0x0A04: n:u32le, then n records of 4 x f32le [x_m, y_m, f3, f4]
//! 0x0A17: 2 x f32le [x_m, y_m] of the nearest target
//! ```
//!
//! `x_m`/`y_m` are metres in the radar's local frame: x left/right, y away from the
//! radar. `f3` and `f4` have no known meaning; they are passed through, never read as
//! height or velocity.

use serde::Serialize;

/// One decoded `0x0A04` record, stamped with the wall-clock time its frame was parsed.
/// Floats serialize as their shortest f32 form; a non-finite value becomes `null`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Target {
    pub t_ms: u64,
    pub x_m: f32,
    pub y_m: f32,
    /// Record word 3. Meaning unknown; always NaN (`null`) in the measured capture.
    pub f3_unknown: f32,
    /// Record word 4. Meaning unknown; always 0.0 in the measured capture.
    pub f4_unknown: f32,
}

/// The `0x0A17` nearest-target position.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Nearest {
    pub x_m: f32,
    pub y_m: f32,
}

const COUNT_LEN: usize = 4;
const RECORD_LEN: usize = 16;

fn f32_at(p: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

/// Decode a whole `0x0A04` payload, or nothing. The payload length must be exactly
/// `4 + 16n` for the declared `n`, and every x/y must be finite; any other payload is a
/// parse error and yields no partial points.
pub fn parse_points(p: &[u8], t_ms: u64) -> Option<Vec<Target>> {
    let n = u32::from_le_bytes(p.get(..COUNT_LEN)?.try_into().ok()?) as usize;
    let body = &p[COUNT_LEN..];
    if n.checked_mul(RECORD_LEN)? != body.len() {
        return None;
    }
    body.chunks_exact(RECORD_LEN)
        .map(|r| {
            let (x_m, y_m) = (f32_at(r, 0), f32_at(r, 4));
            (x_m.is_finite() && y_m.is_finite()).then(|| Target {
                t_ms,
                x_m,
                y_m,
                f3_unknown: f32_at(r, 8),
                f4_unknown: f32_at(r, 12),
            })
        })
        .collect()
}

/// Decode a `0x0A17` payload: exactly 8 bytes, both coordinates finite.
pub fn parse_nearest(p: &[u8]) -> Option<Nearest> {
    if p.len() != 8 {
        return None;
    }
    let (x_m, y_m) = (f32_at(p, 0), f32_at(p, 4));
    (x_m.is_finite() && y_m.is_finite()).then_some(Nearest { x_m, y_m })
}

/// Build a `0x0A04` payload from `[x, y, f3, f4]` records. Used by tests.
#[cfg(test)]
pub fn encode_points(records: &[[f32; 4]]) -> Vec<u8> {
    let mut p = (records.len() as u32).to_le_bytes().to_vec();
    for r in records {
        for v in r {
            p.extend(v.to_le_bytes());
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_target() {
        let p = encode_points(&[[0.25, 1.5, f32::NAN, 0.0]]);
        assert_eq!(p.len(), 20);
        let got = parse_points(&p, 42).unwrap();
        assert_eq!(got.len(), 1);
        let t = got[0];
        assert_eq!((t.t_ms, t.x_m, t.y_m, t.f4_unknown), (42, 0.25, 1.5, 0.0));
        assert!(t.f3_unknown.is_nan());
    }

    #[test]
    fn three_targets_keep_order() {
        let p = encode_points(&[
            [-1.0, 0.5, 0.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [1.25, 3.75, 0.0, 0.0],
        ]);
        let xs: Vec<f32> = parse_points(&p, 0).unwrap().iter().map(|t| t.x_m).collect();
        assert_eq!(xs, vec![-1.0, 0.0, 1.25]);
    }

    #[test]
    fn zero_targets_is_valid_and_empty() {
        assert_eq!(parse_points(&0u32.to_le_bytes(), 0), Some(vec![]));
    }

    #[test]
    fn length_must_match_declared_count() {
        let good = encode_points(&[[1.0, 1.0, 0.0, 0.0], [2.0, 2.0, 0.0, 0.0]]);
        // One byte short, one byte long, a count that claims more or fewer records.
        assert_eq!(parse_points(&good[..good.len() - 1], 0), None);
        assert_eq!(parse_points(&[good.clone(), vec![0]].concat(), 0), None);
        let mut more = good.clone();
        more[0] = 3;
        assert_eq!(parse_points(&more, 0), None);
        let mut fewer = good;
        fewer[0] = 1;
        assert_eq!(parse_points(&fewer, 0), None);
        assert_eq!(parse_points(&[1, 0], 0), None);
        assert_eq!(parse_points(&[], 0), None);
        // A huge count must not overflow the length check.
        assert_eq!(parse_points(&u32::MAX.to_le_bytes(), 0), None);
    }

    #[test]
    fn non_finite_position_rejects_the_whole_frame() {
        let p = encode_points(&[[1.0, 1.0, 0.0, 0.0], [f32::NAN, 1.0, 0.0, 0.0]]);
        assert_eq!(parse_points(&p, 0), None);
        let p = encode_points(&[[1.0, f32::INFINITY, 0.0, 0.0]]);
        assert_eq!(parse_points(&p, 0), None);
    }

    #[test]
    fn unknown_words_serialize_raw_with_nan_as_null() {
        let t = parse_points(&encode_points(&[[0.12, -0.5, f32::NAN, 0.0]]), 7).unwrap()[0];
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"t_ms":7,"x_m":0.12,"y_m":-0.5,"f3_unknown":null,"f4_unknown":0.0}"#
        );
    }

    // Through the framer and the window, as the binary sees them.

    use crate::frame::{encode, Framer, FramerStats};
    use crate::window::{Report, Window, WindowContext, MAX_TARGETS, T_NEAREST, T_POINTS};

    fn points(id: u16, records: &[[f32; 4]]) -> Vec<u8> {
        encode(id, T_POINTS, &encode_points(records))
    }

    fn nearest(x: f32, y: f32) -> Vec<u8> {
        encode(9, T_NEAREST, &[x.to_le_bytes(), y.to_le_bytes()].concat())
    }

    /// Feed `reads` as separate serial reads, each stamped with its own parse time.
    fn window(reads: &[&[u8]]) -> Report {
        let mut f = Framer::new();
        let mut w = Window::default();
        for (i, chunk) in reads.iter().enumerate() {
            for fr in f.feed(chunk) {
                w.ingest(&fr, 1000 + i as u64);
            }
        }
        let bytes_read = reads.iter().map(|r| r.len() as u64).sum();
        w.finish(WindowContext {
            start_ms: 0,
            end_ms: 2000,
            device: "/dev/ttyUSB0".into(),
            stats: f.stats(),
            bytes_read,
            device_error: None,
        })
    }

    fn xy(r: &Report) -> Vec<(f32, f32)> {
        r.targets.iter().map(|t| (t.x_m, t.y_m)).collect()
    }

    #[test]
    fn window_collects_points_in_arrival_order() {
        let bytes = [
            points(1, &[[0.1, 1.0, f32::NAN, 0.0]]),
            points(
                2,
                &[
                    [-0.5, 2.0, f32::NAN, 0.0],
                    [0.5, 2.5, f32::NAN, 0.0],
                    [1.5, 3.0, f32::NAN, 0.0],
                ],
            ),
        ]
        .concat();
        let r = window(&[&bytes]);
        assert_eq!(
            xy(&r),
            vec![(0.1, 1.0), (-0.5, 2.0), (0.5, 2.5), (1.5, 3.0)]
        );
        assert!(r.targets.iter().all(|t| t.t_ms == 1000));
        assert_eq!((r.targets_dropped, r.target_parse_errors), (0, 0));
        assert_eq!((r.health, r.frame), ("ok", "radar_local"));
        assert!(r.reasons.is_empty());
    }

    #[test]
    fn wrong_length_frame_is_counted_and_adds_no_points() {
        let mut short = encode_points(&[[1.0, 1.0, 0.0, 0.0], [2.0, 2.0, 0.0, 0.0]]);
        short.truncate(short.len() - 4);
        let bytes = [
            encode(1, T_POINTS, &short),
            points(2, &[[3.0, 3.0, 0.0, 0.0]]),
        ]
        .concat();
        let r = window(&[&bytes]);
        assert_eq!(xy(&r), vec![(3.0, 3.0)]);
        assert_eq!(r.target_parse_errors, 1);
        assert_eq!(r.reasons, vec!["target_parse_errors"]);
        assert_eq!(r.frame_counts.get("0x0a04"), Some(&2));
    }

    #[test]
    fn nan_f3_is_kept_and_serialized_as_null() {
        let r = window(&[&points(1, &[[0.25, 0.75, f32::NAN, 0.0]])]);
        assert_eq!(r.targets.len(), 1);
        assert!(r.targets[0].f3_unknown.is_nan());
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(v["targets"][0]["f3_unknown"].is_null());
        assert_eq!(v["targets"][0]["x_m"], 0.25);
        assert_eq!(v["targets"][0]["t_ms"], 1000);
    }

    #[test]
    fn frames_split_across_reads_decode_with_the_later_read_time() {
        let bytes = [
            points(1, &[[0.5, 1.0, 0.0, 0.0]]),
            nearest(0.5, 1.0),
            points(2, &[[0.6, 1.1, 0.0, 0.0], [-0.6, 2.2, 0.0, 0.0]]),
        ]
        .concat();
        let first_end = points(1, &[[0.0; 4]]).len();
        for cut in 0..=bytes.len() {
            let r = window(&[&bytes[..cut], &bytes[cut..]]);
            assert_eq!(
                xy(&r),
                vec![(0.5, 1.0), (0.6, 1.1), (-0.6, 2.2)],
                "split at {cut}"
            );
            assert_eq!(r.resync_bytes, 0, "split at {cut}");
            assert_eq!(r.nearest, Some(Nearest { x_m: 0.5, y_m: 1.0 }));
            // A frame completed by the second read is stamped with the second read's time.
            let want = if cut >= first_end { 1000 } else { 1001 };
            assert_eq!(r.targets[0].t_ms, want, "split at {cut}");
            assert_eq!(r.targets[2].t_ms, 1000 + u64::from(cut < bytes.len()));
        }
    }

    #[test]
    fn cap_keeps_the_first_points_and_counts_the_rest() {
        // 22 frames x 3 points = 66 points: 64 kept, 2 dropped.
        let bytes: Vec<u8> = (0..22u16)
            .flat_map(|i| {
                let x = f32::from(i);
                points(
                    i,
                    &[[x, 1.0, 0.0, 0.0], [x, 2.0, 0.0, 0.0], [x, 3.0, 0.0, 0.0]],
                )
            })
            .collect();
        let r = window(&[&bytes]);
        assert_eq!(r.targets.len(), MAX_TARGETS);
        assert_eq!(r.targets_dropped, 66 - MAX_TARGETS as u64);
        assert_eq!(xy(&r)[MAX_TARGETS - 1], (21.0, 1.0));
        // Each frame is still decoded and counted; the cap is not an error.
        assert_eq!((r.target_parse_errors, r.health), (0, "ok"));
        let line = serde_json::to_string(&r).unwrap();
        assert!(line.len() < 65_536 / 4, "{} bytes", line.len());
    }

    #[test]
    fn nearest_is_the_last_valid_report() {
        let bytes = [
            nearest(0.1, 0.9),
            nearest(-0.2, 1.4),
            encode(9, T_NEAREST, &[0; 7]), // short: malformed, does not replace
        ]
        .concat();
        let r = window(&[&bytes]);
        assert_eq!(
            r.nearest,
            Some(Nearest {
                x_m: -0.2,
                y_m: 1.4
            })
        );
        assert_eq!(r.reasons, vec!["short_payload"]);
        assert!(r.targets.is_empty());
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(v["nearest"], serde_json::json!({"x_m": -0.2, "y_m": 1.4}));
    }

    #[test]
    fn no_source_window_has_empty_targets() {
        let r = Window::default().finish(WindowContext {
            start_ms: 0,
            end_ms: 1,
            device: "/dev/x".into(),
            stats: FramerStats::default(),
            bytes_read: 0,
            device_error: Some("gone".into()),
        });
        assert_eq!(r.health, "no_source");
        assert!(r.targets.is_empty() && r.nearest.is_none());
    }

    #[test]
    fn nearest_needs_exactly_two_finite_floats() {
        let mut p = 0.5f32.to_le_bytes().to_vec();
        p.extend(1.75f32.to_le_bytes());
        assert_eq!(
            parse_nearest(&p),
            Some(Nearest {
                x_m: 0.5,
                y_m: 1.75
            })
        );
        assert_eq!(parse_nearest(&p[..7]), None);
        assert_eq!(parse_nearest(&[p.clone(), vec![0]].concat()), None);
        let mut nan = p;
        nan[4..].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(parse_nearest(&nan), None);
    }
}
