//! Per-window aggregation of decoded LD2450 frames into one output line.
//!
//! The radar sends one report frame about every 100 ms holding its current tracks (up to
//! three). A line carries the latest frame's tracks, how many frames arrived, and the
//! link counters. A value the radar did not report in the window stays `null`.

use serde::Serialize;

use crate::frame::{Ack, FramerStats, Slot};

pub const SCHEMA: &str = "weavelogic.cog-output.v0";
/// Coordinate frame of `targets`: metres, x lateral, y forward along the radar's
/// boresight. Placing targets in a room needs the mount pose, which this cog does not
/// know.
pub const TARGET_FRAME: &str = "radar_local";
/// Targets farther than this from the radar are implausible for a 6 m sensor and are
/// dropped as corrupt (the protocol carries no checksum).
pub const MAX_PLAUSIBLE_MM: i32 = 10_000;

/// One tracked target from the latest frame. Positions only; nothing identifies a person.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Target {
    /// Slot 1-3 in the radar's frame. A slot number is not a stable person id.
    pub slot: u8,
    pub x_m: f64,
    pub y_m: f64,
    pub speed_mps: f64,
    pub resolution_mm: u16,
}

/// Why a non-empty slot was not reported as a target.
fn implausible(s: &Slot) -> bool {
    // The protocol says y is always positive (in front of the radar).
    s.y_mm <= 0 || s.x_mm.abs() > MAX_PLAUSIBLE_MM || s.y_mm > MAX_PLAUSIBLE_MM
}

/// Decode one report frame into targets, plus the count of implausible slots.
pub fn targets(slots: &[Slot; 3]) -> (Vec<Target>, u64) {
    let mut out = Vec::new();
    let mut bad = 0;
    for (i, s) in slots.iter().enumerate() {
        if s.is_empty() {
            continue;
        }
        if implausible(s) {
            bad += 1;
            continue;
        }
        out.push(Target {
            slot: i as u8 + 1,
            x_m: f64::from(s.x_mm) / 1000.0,
            y_m: f64::from(s.y_mm) / 1000.0,
            speed_mps: f64::from(s.speed_cm_s) / 100.0,
            resolution_mm: s.resolution_mm,
        });
    }
    (out, bad)
}

/// Frames decoded within one window.
#[derive(Debug, Default)]
pub struct Window {
    reports: u64,
    latest: Option<(u64, Vec<Target>)>,
    max_target_count: Option<u8>,
    implausible_targets: u64,
    acks: u64,
    failed_acks: u64,
}

impl Window {
    /// `t_ms` is the wall-clock time the frame finished parsing.
    pub fn ingest_report(&mut self, slots: &[Slot; 3], t_ms: u64) -> Vec<Target> {
        let (ts, bad) = targets(slots);
        self.reports += 1;
        self.implausible_targets += bad;
        let n = ts.len() as u8;
        self.max_target_count = Some(self.max_target_count.map_or(n, |m| m.max(n)));
        self.latest = Some((t_ms, ts.clone()));
        ts
    }

    pub fn ingest_ack(&mut self, a: &Ack) {
        self.acks += 1;
        if a.status != 0 {
            self.failed_acks += 1;
        }
    }

    pub fn finish(self, ctx: WindowContext) -> Report {
        let s = ctx.stats;
        let verified = s.reports > 0 && ctx.kind == SourceKind::Uart;
        let mut reasons = Vec::new();
        let (health, quality) = match (&ctx.device_error, s.reports > 0) {
            (Some(e), _) => {
                reasons.push(format!("device_error: {e}"));
                ("no_source", None)
            }
            (None, false) => {
                reasons.push(if ctx.bytes_read == 0 {
                    "no_bytes".into()
                } else {
                    "no_valid_frames".into()
                });
                ("no_source", None)
            }
            (None, true) => {
                // Link integrity: share of consumed bytes that belonged to valid frames.
                let q = s.frame_bytes as f64 / (s.frame_bytes + s.resync_bytes) as f64;
                let q = (q * 1000.0).round() / 1000.0;
                let bad = s.parse_errors() > 0 || self.implausible_targets > 0;
                (if bad { "degraded" } else { "ok" }, Some(q))
            }
        };
        if s.parse_errors() > 0 {
            reasons.push("parse_errors".into());
        }
        if self.implausible_targets > 0 {
            reasons.push("implausible_targets".into());
        }
        if self.failed_acks > 0 {
            reasons.push("command_failed".into());
        }
        if let Some(e) = &ctx.config_error {
            reasons.push(format!("config_error: {e}"));
        }
        let hint = hint(
            ctx.kind,
            &ctx.device_error,
            s.reports,
            ctx.bytes_read,
            s.parse_errors(),
        );
        match ctx.kind {
            SourceKind::Simulated => reasons.push("simulated".into()),
            SourceKind::Replay => reasons.push("replay".into()),
            SourceKind::Uart => {}
        }
        let elapsed_s = ctx.end_ms.saturating_sub(ctx.start_ms) as f64 / 1000.0;
        // Null when there is no timing to measure: a replay, or a device that never opened.
        let timed = ctx.kind != SourceKind::Replay && ctx.device_error.is_none();
        let frame_rate_hz = (timed && elapsed_s > 0.0)
            .then(|| (s.reports as f64 / elapsed_s * 10.0).round() / 10.0);
        let (latest_frame_ms, targets) = match self.latest {
            Some((t, ts)) => (Some(t), ts),
            None => (None, Vec::new()),
        };
        Report {
            schema: SCHEMA,
            cog: "ld2450-radar",
            version: env!("CARGO_PKG_VERSION"),
            timestamp: ctx.end_ms / 1000,
            timestamp_ms: ctx.end_ms,
            window_start_ms: ctx.start_ms,
            sensor_class: "radar",
            source: Source {
                kind: ctx.kind.name(),
                device: ctx.device,
                verified,
                simulated: ctx.kind == SourceKind::Simulated,
            },
            health,
            quality,
            reasons,
            hint,
            frame: TARGET_FRAME,
            target_count: latest_frame_ms.map(|_| targets.len() as u8),
            max_target_count: self.max_target_count,
            latest_frame_ms,
            targets,
            firmware: ctx.firmware,
            tracking_mode: ctx.tracking_mode,
            frames: s.reports,
            frame_rate_hz,
            parse_errors: s.parse_errors(),
            bad_tail: s.bad_tail,
            bad_ack_len: s.bad_ack_len,
            implausible_targets: self.implausible_targets,
            resync_bytes: s.resync_bytes,
            bytes_read: ctx.bytes_read,
            acks: self.acks,
            #[cfg(feature = "spatial-evidence")]
            spatial: None,
        }
    }
}

/// What to check next, for a person at the bench.
fn hint(
    kind: SourceKind,
    device_error: &Option<String>,
    reports: u64,
    bytes_read: u64,
    parse_errors: u64,
) -> Option<&'static str> {
    if kind == SourceKind::Replay {
        return (device_error.is_some() || reports == 0)
            .then_some("check the capture path and that it holds raw LD2450 UART bytes");
    }
    if device_error.is_some() {
        return Some(
            "cannot open the device: check --device (ls -l /dev/serial0 or /dev/ttyUSB0), that the \
             serial console no longer owns the UART, and that the cog's user may open it (dialout)",
        );
    }
    if reports == 0 && bytes_read == 0 {
        return Some(
            "port open but silent: check 5 V on header pin 2 and GND on pin 6, radar UT to Pi RXD \
             (pin 10), and that the console was removed and the Seed rebooted",
        );
    }
    if reports == 0 {
        return Some(
            "bytes but no frames: check --baud (factory 256000), that no login console shares the \
             UART, and power-cycle the radar in case it is stuck in config mode",
        );
    }
    (parse_errors > 0).then_some(
        "frames with bad tails mean lost bytes: on the mini UART (ttyS0) move the PL011 to pins 8/10 \
         or use a USB-serial adapter; keep the wires short",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Uart,
    Simulated,
    Replay,
}

impl SourceKind {
    pub fn name(self) -> &'static str {
        match self {
            SourceKind::Uart => "ld2450-uart",
            SourceKind::Simulated => "ld2450-simulated",
            SourceKind::Replay => "ld2450-replay",
        }
    }
}

/// Everything about a window that is not a decoded frame.
pub struct WindowContext {
    pub kind: SourceKind,
    pub start_ms: u64,
    pub end_ms: u64,
    pub device: String,
    pub stats: FramerStats,
    pub bytes_read: u64,
    pub device_error: Option<String>,
    pub config_error: Option<String>,
    pub firmware: Option<String>,
    pub tracking_mode: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Source {
    pub kind: &'static str,
    pub device: String,
    /// True only when valid report frames arrived from a real UART in this window.
    pub verified: bool,
    pub simulated: bool,
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
    /// The next physical check when something is wrong; `null` when healthy.
    pub hint: Option<&'static str>,
    /// Coordinate frame of `targets`; always [`TARGET_FRAME`].
    pub frame: &'static str,
    /// Valid targets in the latest frame; `null` when no frame arrived.
    pub target_count: Option<u8>,
    /// Most valid targets in any frame of the window.
    pub max_target_count: Option<u8>,
    /// Parse time of the frame `targets` came from.
    pub latest_frame_ms: Option<u64>,
    pub targets: Vec<Target>,
    pub firmware: Option<String>,
    pub tracking_mode: Option<&'static str>,
    pub frames: u64,
    /// Report frames per second over the window; `null` for a replay.
    pub frame_rate_hz: Option<f64>,
    pub parse_errors: u64,
    pub bad_tail: u64,
    pub bad_ack_len: u64,
    pub implausible_targets: u64,
    pub resync_bytes: u64,
    pub bytes_read: u64,
    pub acks: u64,
    /// Spatial-evidence adapter state: `null` (off), `emitting`, `no_pose`, `no_region`
    /// or `write_error`. Only in builds with the `spatial-evidence` feature.
    #[cfg(feature = "spatial-evidence")]
    pub spatial: Option<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{encode_report, Frame, Framer};

    fn slot(x: i32, y: i32, v: i32) -> Slot {
        Slot {
            x_mm: x,
            y_mm: y,
            speed_cm_s: v,
            resolution_mm: 360,
        }
    }

    fn ctx(stats: FramerStats, kind: SourceKind) -> WindowContext {
        WindowContext {
            kind,
            start_ms: 1_790_000_000_000,
            end_ms: 1_790_000_001_000,
            device: "/dev/serial0".into(),
            stats,
            bytes_read: stats.frame_bytes + stats.resync_bytes,
            device_error: None,
            config_error: None,
            firmware: None,
            tracking_mode: None,
        }
    }

    fn run(frames: &[[Slot; 3]], garbage: &[u8]) -> Report {
        let mut f = Framer::new();
        let mut w = Window::default();
        let mut bytes = garbage.to_vec();
        for s in frames {
            bytes.extend(encode_report(s));
        }
        for (i, fr) in f.feed(&bytes).into_iter().enumerate() {
            if let Frame::Report(s) = fr {
                w.ingest_report(&s, 1_790_000_000_100 + i as u64);
            }
        }
        w.finish(ctx(f.stats(), SourceKind::Uart))
    }

    #[test]
    fn latest_frame_wins_and_units_convert() {
        let e = Slot::default();
        let r = run(
            &[
                [slot(-782, 1713, -16), e, e],
                [slot(-700, 1650, 25), slot(1200, 3000, 0), e],
            ],
            &[],
        );
        assert_eq!(r.health, "ok");
        assert_eq!(r.quality, Some(1.0));
        assert_eq!(r.frames, 2);
        assert_eq!(r.target_count, Some(2));
        assert_eq!(r.max_target_count, Some(2));
        assert_eq!(r.frame_rate_hz, Some(2.0));
        let t = r.targets[0];
        assert_eq!((t.slot, t.x_m, t.y_m, t.speed_mps), (1, -0.7, 1.65, 0.25));
        assert_eq!(r.targets[1].slot, 2);
        assert!(r.source.verified);
    }

    #[test]
    fn empty_frames_are_zero_targets_not_null() {
        let r = run(&[[Slot::default(); 3]; 10], &[]);
        assert_eq!(r.health, "ok");
        assert_eq!(r.target_count, Some(0));
        assert!(r.targets.is_empty());
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["targets"], serde_json::json!([]));
    }

    #[test]
    fn implausible_slots_are_dropped_and_counted() {
        let r = run(
            &[[slot(100, -500, 0), slot(20_000, 1000, 0), slot(1, 900, 0)]],
            &[],
        );
        assert_eq!(r.implausible_targets, 2);
        assert_eq!(r.target_count, Some(1));
        assert_eq!(r.targets[0].slot, 3);
        assert_eq!(r.health, "degraded");
        assert!(r.reasons.contains(&"implausible_targets".to_string()));
    }

    #[test]
    fn garbage_lowers_quality_but_frames_still_decode() {
        let r = run(&[[slot(0, 2000, 0); 3]], &[0x13; 30]);
        assert_eq!(r.frames, 1);
        assert_eq!(r.resync_bytes, 30);
        assert_eq!(r.quality, Some(0.5));
        assert_eq!(r.health, "ok"); // resync alone is not a parse error
    }

    #[test]
    fn no_frames_is_no_source_with_nulls() {
        let r = Window::default().finish(ctx(FramerStats::default(), SourceKind::Uart));
        assert_eq!(r.health, "no_source");
        assert_eq!(r.quality, None);
        assert_eq!(r.target_count, None);
        assert_eq!(r.reasons, vec!["no_bytes".to_string()]);
        assert!(!r.source.verified);
    }

    #[test]
    fn simulated_output_is_never_verified() {
        let mut w = Window::default();
        w.ingest_report(&[slot(0, 1000, 0), Slot::default(), Slot::default()], 1);
        let stats = FramerStats {
            reports: 1,
            frame_bytes: 30,
            ..Default::default()
        };
        let r = w.finish(ctx(stats, SourceKind::Simulated));
        assert!(!r.source.verified);
        assert!(r.source.simulated);
        assert!(r.reasons.contains(&"simulated".to_string()));
    }
}
