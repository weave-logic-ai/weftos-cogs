//! Per-window aggregation of decoded LD6002 frames into one output line.
//!
//! Only the frame types the reference host decoder defines are decoded. Everything
//! else is counted by type and never given a meaning. A value the radar did not
//! report in the window stays `None` and serializes as `null`.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::frame::{Frame, FramerStats};
use crate::targets::{parse_nearest, parse_points, Nearest, Target};

pub const T_PHASE: u16 = 0x0A13; // total / breath-phase / heart-phase, f32 LE x3
pub const T_BREATH_RATE: u16 = 0x0A14; // f32 LE
pub const T_HEART_RATE: u16 = 0x0A15; // f32 LE
pub const T_DISTANCE: u16 = 0x0A16; // flag (byte 0) + distance cm f32 LE at [4..8]
pub const T_TARGET_COUNT: u16 = 0x0A38; // u8
pub const T_PRESENCE: u16 = 0x0F09; // people-exist u16 LE, 1 = present
pub const T_TEXT: u16 = 0x0100; // ASCII debug channel, counted only
pub const T_POINTS: u16 = 0x0A04; // target positions, see targets.rs
pub const T_NEAREST: u16 = 0x0A17; // nearest target x/y, see targets.rs

const DECODED: [u16; 9] = [
    T_PHASE,
    T_BREATH_RATE,
    T_HEART_RATE,
    T_DISTANCE,
    T_TARGET_COUNT,
    T_PRESENCE,
    T_TEXT,
    T_POINTS,
    T_NEAREST,
];

pub const SCHEMA: &str = "weavelogic.cog-output.v0";
pub const SOURCE_KIND: &str = "ld6002-uart";
/// Coordinate frame of `targets` and `nearest`: metres, x left/right, y away from the
/// radar. Placing points in a room needs the mount pose, which this cog does not know.
pub const TARGET_FRAME: &str = "radar_local";
/// Most points kept per window. At ~9.5 Hz single-target frames a 1 s window holds about
/// 10; the cap bounds a long window or a crowded scene under the console output limit.
pub const MAX_TARGETS: usize = 64;

fn f32_le(p: &[u8], at: usize) -> Option<f32> {
    let b = p.get(at..at + 4)?;
    Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Round to 0.01 so f32 artefacts (72.30000305...) do not leak into the JSON.
fn r2(v: f32) -> f64 {
    (f64::from(v) * 100.0).round() / 100.0
}

/// A rate the radar reports as 0 or non-finite is "no reading" (the reference
/// decoder drops `v <= 0` the same way).
fn positive(v: f32) -> Option<f32> {
    (v.is_finite() && v > 0.0).then_some(v)
}

/// Frames decoded within one window. The last report of each value wins.
#[derive(Debug, Default)]
pub struct Window {
    heart_rate: Option<f32>,
    breath_rate: Option<f32>,
    distance_cm: Option<f32>,
    presence: Option<bool>,
    target_count: Option<u8>,
    targets: Vec<Target>,
    targets_dropped: u64,
    target_parse_errors: u64,
    nearest: Option<Nearest>,
    type_counts: BTreeMap<u16, u64>,
    malformed: u64,
}

impl Window {
    /// `t_ms` is the wall-clock time the frame finished parsing; it stamps targets.
    pub fn ingest(&mut self, f: &Frame, t_ms: u64) {
        *self.type_counts.entry(f.ty).or_default() += 1;
        let p = &f.payload;
        let ok = match f.ty {
            T_HEART_RATE => f32_le(p, 0).map(|v| {
                if let Some(v) = positive(v) {
                    self.heart_rate = Some(v);
                }
            }),
            T_BREATH_RATE => f32_le(p, 0).map(|v| {
                if let Some(v) = positive(v) {
                    self.breath_rate = Some(v);
                }
            }),
            T_DISTANCE => f32_le(p, 4).map(|v| {
                if p[0] != 0 && v.is_finite() && v >= 0.0 {
                    self.distance_cm = Some(v);
                }
            }),
            T_PRESENCE => (p.len() >= 2).then(|| self.presence = Some(p[0] == 1)),
            T_TARGET_COUNT => p.first().map(|&n| self.target_count = Some(n)),
            T_PHASE => (p.len() >= 12).then_some(()),
            T_POINTS => {
                match parse_points(p, t_ms) {
                    Some(pts) => {
                        let room = MAX_TARGETS - self.targets.len();
                        self.targets_dropped += pts.len().saturating_sub(room) as u64;
                        self.targets.extend(pts.into_iter().take(room));
                    }
                    None => self.target_parse_errors += 1,
                }
                Some(())
            }
            T_NEAREST => parse_nearest(p).map(|n| self.nearest = Some(n)),
            _ => Some(()),
        };
        if ok.is_none() {
            self.malformed += 1;
        }
    }

    pub fn finish(self, ctx: WindowContext) -> Report {
        let s = ctx.stats;
        let verified = s.frames > 0;
        let (health, quality, mut reasons) = match (&ctx.device_error, verified) {
            (Some(e), _) => ("no_source", None, vec![format!("device_error: {e}")]),
            (None, false) if ctx.bytes_read == 0 => ("no_source", None, vec!["no_bytes".into()]),
            (None, false) => ("no_source", None, vec!["no_valid_frames".into()]),
            (None, true) => {
                // Link integrity: share of consumed bytes that belonged to valid frames.
                // Resync bytes (garbage, failed checksums, a mid-frame join) lower it.
                let q = s.frame_bytes as f64 / (s.frame_bytes + s.resync_bytes) as f64;
                let q = (q * 1000.0).round() / 1000.0;
                let health = if s.checksum_errors() > 0 {
                    "degraded"
                } else {
                    "ok"
                };
                (health, Some(q), Vec::new())
            }
        };
        if verified && s.checksum_errors() > 0 {
            reasons.push("checksum_errors".into());
        }
        if self.malformed > 0 {
            reasons.push("short_payload".into());
        }
        if self.target_parse_errors > 0 {
            reasons.push("target_parse_errors".into());
        }
        let hex = |t: &u16| format!("0x{t:04x}");
        let frame_counts: BTreeMap<String, u64> =
            self.type_counts.iter().map(|(t, n)| (hex(t), *n)).collect();
        let undecoded_frame_counts: BTreeMap<String, u64> = self
            .type_counts
            .iter()
            .filter(|(t, _)| !DECODED.contains(t))
            .map(|(t, n)| (hex(t), *n))
            .collect();
        Report {
            schema: SCHEMA,
            cog: "ld6002-radar",
            version: env!("CARGO_PKG_VERSION"),
            timestamp: ctx.end_ms / 1000,
            timestamp_ms: ctx.end_ms,
            window_start_ms: ctx.start_ms,
            sensor_class: "radar",
            source: Source {
                kind: SOURCE_KIND,
                device: ctx.device,
                verified,
            },
            health,
            quality,
            reasons,
            presence: self.presence,
            distance_cm: self.distance_cm.map(r2),
            heart_rate_bpm: self.heart_rate.map(r2),
            breathing_rate_bpm: self.breath_rate.map(r2),
            target_count: self.target_count,
            frame: TARGET_FRAME,
            targets: self.targets,
            targets_dropped: self.targets_dropped,
            target_parse_errors: self.target_parse_errors,
            nearest: self.nearest,
            phase_frames: self.type_counts.get(&T_PHASE).copied().unwrap_or(0),
            frames: s.frames,
            checksum_errors: s.checksum_errors(),
            oversize_frames: s.oversize_frames,
            resync_bytes: s.resync_bytes,
            bytes_read: ctx.bytes_read,
            frame_counts,
            undecoded_frame_counts,
            #[cfg(feature = "spatial-evidence")]
            spatial: None,
        }
    }
}

/// Everything about a window that is not a decoded frame.
pub struct WindowContext {
    pub start_ms: u64,
    pub end_ms: u64,
    pub device: String,
    pub stats: FramerStats,
    pub bytes_read: u64,
    pub device_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Source {
    pub kind: &'static str,
    pub device: String,
    /// True only when checksum-valid LD6002 frames arrived in this window.
    pub verified: bool,
}

/// One output line. `timestamp` is catalog seconds; `timestamp_ms` and
/// `window_start_ms` are Unix-epoch UTC milliseconds bounding the window.
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub cog: &'static str,
    pub version: &'static str,
    pub timestamp: u64,
    pub timestamp_ms: u64,
    pub window_start_ms: u64,
    pub sensor_class: &'static str,
    pub source: Source,
    pub health: &'static str,
    pub quality: Option<f64>,
    pub reasons: Vec<String>,
    pub presence: Option<bool>,
    pub distance_cm: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    pub breathing_rate_bpm: Option<f64>,
    pub target_count: Option<u8>,
    /// Coordinate frame of `targets` and `nearest`; always [`TARGET_FRAME`].
    pub frame: &'static str,
    /// `0x0A04` points in arrival order, at most [`MAX_TARGETS`].
    pub targets: Vec<Target>,
    /// Points beyond [`MAX_TARGETS`] in this window, not emitted.
    pub targets_dropped: u64,
    /// `0x0A04` frames whose payload did not decode; they contribute no points.
    pub target_parse_errors: u64,
    /// The last valid `0x0A17` in the window.
    pub nearest: Option<Nearest>,
    pub phase_frames: u64,
    pub frames: u64,
    pub checksum_errors: u64,
    pub oversize_frames: u64,
    pub resync_bytes: u64,
    pub bytes_read: u64,
    pub frame_counts: BTreeMap<String, u64>,
    pub undecoded_frame_counts: BTreeMap<String, u64>,
    /// Spatial-evidence adapter state: `null` (off), `emitting`, `no_pose`, `no_region`
    /// or `write_error`. Only in builds with the `spatial-evidence` feature.
    #[cfg(feature = "spatial-evidence")]
    pub spatial: Option<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{encode, Framer};

    fn ctx(stats: FramerStats, bytes_read: u64) -> WindowContext {
        WindowContext {
            start_ms: 1_790_000_000_000,
            end_ms: 1_790_000_001_234,
            device: "/dev/ttyUSB0".into(),
            stats,
            bytes_read,
            device_error: None,
        }
    }

    fn run(bytes: &[u8]) -> Report {
        let mut f = Framer::new();
        let mut w = Window::default();
        for (i, fr) in f.feed(bytes).iter().enumerate() {
            w.ingest(fr, 1_790_000_000_100 + i as u64);
        }
        w.finish(ctx(f.stats(), bytes.len() as u64))
    }

    fn dist(flag: u8, cm: f32) -> Vec<u8> {
        let mut p = vec![flag, 0, 0, 0];
        p.extend(cm.to_le_bytes());
        encode(1, T_DISTANCE, &p)
    }

    #[test]
    fn decodes_every_defined_type() {
        let bytes = [
            encode(1, T_HEART_RATE, &72.3f32.to_le_bytes()),
            encode(2, T_BREATH_RATE, &14.0f32.to_le_bytes()),
            dist(1, 85.25),
            encode(4, T_PRESENCE, &[1, 0]),
            encode(5, T_TARGET_COUNT, &[2]),
            encode(6, T_PHASE, &[0u8; 12]),
        ]
        .concat();
        let r = run(&bytes);
        assert_eq!(r.heart_rate_bpm, Some(72.3));
        assert_eq!(r.breathing_rate_bpm, Some(14.0));
        assert_eq!(r.distance_cm, Some(85.25));
        assert_eq!(r.presence, Some(true));
        assert_eq!(r.target_count, Some(2));
        assert_eq!(r.phase_frames, 1);
        assert_eq!((r.health, r.quality), ("ok", Some(1.0)));
        assert!(r.source.verified);
        assert!(r.undecoded_frame_counts.is_empty());
    }

    #[test]
    fn missing_values_stay_null() {
        let r = run(&encode(1, T_PHASE, &[0u8; 12]));
        assert_eq!(r.health, "ok");
        assert_eq!(r.heart_rate_bpm, None);
        assert_eq!(r.breathing_rate_bpm, None);
        assert_eq!(r.distance_cm, None);
        assert_eq!(r.presence, None);
        assert_eq!(r.target_count, None);
        assert!(r.targets.is_empty());
        assert_eq!((r.targets_dropped, r.target_parse_errors), (0, 0));
        assert_eq!(r.nearest, None);
    }

    #[test]
    fn zero_rates_and_invalid_distance_are_not_readings() {
        let bytes = [
            encode(1, T_HEART_RATE, &0.0f32.to_le_bytes()),
            encode(2, T_BREATH_RATE, &f32::NAN.to_le_bytes()),
            dist(0, 120.0),
        ]
        .concat();
        let r = run(&bytes);
        assert_eq!(
            (r.heart_rate_bpm, r.breathing_rate_bpm, r.distance_cm),
            (None, None, None)
        );
        assert_eq!(r.frames, 3);
    }

    #[test]
    fn presence_zero_is_reported_false() {
        let r = run(&encode(1, T_PRESENCE, &[0, 0]));
        assert_eq!(r.presence, Some(false));
    }

    #[test]
    fn last_report_in_window_wins() {
        let bytes = [
            encode(1, T_HEART_RATE, &60.0f32.to_le_bytes()),
            encode(2, T_HEART_RATE, &61.5f32.to_le_bytes()),
        ]
        .concat();
        assert_eq!(run(&bytes).heart_rate_bpm, Some(61.5));
    }

    #[test]
    fn unknown_types_are_counted_not_decoded() {
        let bytes = [
            encode(1, 0x0A18, &[1, 0]),
            encode(2, 0xBEEF, &[9; 16]),
            encode(3, 0xBEEF, &[9; 16]),
            encode(4, 0x0A04, &0u32.to_le_bytes()),
        ]
        .concat();
        let r = run(&bytes);
        // 0x0A18 is still undecoded: its two bytes have no known meaning.
        assert_eq!(r.undecoded_frame_counts.get("0x0a18"), Some(&1));
        assert_eq!(r.undecoded_frame_counts.get("0xbeef"), Some(&2));
        assert_eq!(r.undecoded_frame_counts.get("0x0a04"), None);
        assert_eq!(r.frame_counts.len(), 3);
        assert_eq!(r.target_count, None);
    }

    #[test]
    fn short_payload_is_flagged_not_decoded() {
        let r = run(&encode(1, T_HEART_RATE, &[1, 2]));
        assert_eq!(r.heart_rate_bpm, None);
        assert_eq!(r.reasons, vec!["short_payload"]);
    }

    #[test]
    fn checksum_errors_degrade_quality() {
        let mut bad = encode(1, T_HEART_RATE, &70.0f32.to_le_bytes());
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        let bytes = [bad, encode(2, T_HEART_RATE, &71.0f32.to_le_bytes())].concat();
        let r = run(&bytes);
        assert_eq!(r.health, "degraded");
        // The data-checksum failure, plus any stray SOF met while resyncing through it.
        assert!(r.checksum_errors >= 1);
        assert_eq!(r.resync_bytes, 13);
        assert_eq!(r.quality, Some(0.5));
        assert!(r.reasons.contains(&"checksum_errors".to_string()));
    }

    #[test]
    fn no_frames_means_no_source_and_null_quality() {
        let r = run(&[0xFF; 32]);
        assert_eq!(
            (r.health, r.quality, r.source.verified),
            ("no_source", None, false)
        );
        assert_eq!(r.reasons, vec!["no_valid_frames"]);
        let r = run(&[]);
        assert_eq!(r.reasons, vec!["no_bytes"]);
    }

    #[test]
    fn device_error_reports_no_source() {
        let mut c = ctx(FramerStats::default(), 0);
        c.device_error = Some("not found".into());
        let r = Window::default().finish(c);
        assert_eq!((r.health, r.quality), ("no_source", None));
        assert_eq!(r.reasons, vec!["device_error: not found"]);
    }

    #[test]
    fn serializes_one_line_with_seconds_timestamp() {
        let line = serde_json::to_string(&run(&encode(1, T_PRESENCE, &[1, 0]))).unwrap();
        assert!(!line.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["timestamp"], 1_790_000_001u64);
        assert_eq!(v["timestamp_ms"], 1_790_000_001_234u64);
        assert!(v["heart_rate_bpm"].is_null());
        assert!(v["nearest"].is_null());
        assert_eq!(v["targets"], serde_json::json!([]));
        assert_eq!(v["frame"], "radar_local");
        for banned in ["status", "aabb", "pose", "keypoints"] {
            assert!(v.get(banned).is_none(), "{banned} must never be emitted");
        }
    }
}
