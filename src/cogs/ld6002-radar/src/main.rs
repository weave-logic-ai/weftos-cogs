//! cog-ld6002-radar: HLK-LD6002 60 GHz radar reader.
//!
//! Usage:
//!   cog-ld6002-radar [--once] [--interval SECS] [--device /dev/ttyUSB0] [--baud 115200]
//!   (feature `spatial-evidence` adds `--spatial-out FILE --radar-pose ... --spatial-region ...`)
//!
//! Reads the radar's UART continuously and prints one JSON line per `--interval`
//! window to stdout (diagnostics go to stderr). `--once` reads one window and exits.
//! A missing or silent device yields a `health:"no_source"` line with null values,
//! never a fabricated reading, and `--once` still exits 0. Target positions are in the
//! radar's local frame; placing them in a room is the fusion host's job.

mod frame;
#[cfg(feature = "spatial-evidence")]
mod spatial;
mod targets;
mod window;

use std::io::{ErrorKind, Write};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use frame::Framer;
use window::{Window, WindowContext};

const DEFAULT_DEVICE: &str = "/dev/ttyUSB0";
const DEFAULT_BAUD: u32 = 115_200;
const DEFAULT_INTERVAL: u64 = 1;
const READ_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug, PartialEq)]
struct Config {
    once: bool,
    interval: u64,
    device: String,
    baud: u32,
    /// `spatial.evidence.v1` output; `None` unless `--spatial-out` is set.
    #[cfg(feature = "spatial-evidence")]
    spatial: Option<spatial::Settings>,
}

fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut cfg = Config {
        once: false,
        interval: DEFAULT_INTERVAL,
        device: DEFAULT_DEVICE.into(),
        baud: DEFAULT_BAUD,
        #[cfg(feature = "spatial-evidence")]
        spatial: None,
    };
    #[cfg(feature = "spatial-evidence")]
    let mut sp = spatial::Args::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--once" => cfg.once = true,
            "--interval" => {
                let v = value()?;
                cfg.interval = v
                    .parse()
                    .ok()
                    .filter(|n| (1..=3600).contains(n))
                    .ok_or(format!("--interval must be an integer 1..=3600, got {v:?}"))?;
            }
            "--baud" => {
                let v = value()?;
                cfg.baud = v
                    .parse()
                    .ok()
                    .filter(|n| (9600..=3_000_000).contains(n))
                    .ok_or(format!(
                        "--baud must be an integer 9600..=3000000, got {v:?}"
                    ))?;
            }
            "--device" => {
                let v = value()?;
                validate_device(&v)?;
                cfg.device = v;
            }
            #[cfg(feature = "spatial-evidence")]
            f if spatial::Args::FLAGS.contains(&f) => sp.set(f, &value()?)?,
            other => return Err(format!("unknown argument {other:?} (see --help)")),
        }
    }
    #[cfg(feature = "spatial-evidence")]
    {
        cfg.spatial = sp.finish()?;
    }
    Ok(cfg)
}

/// Least authority: only character devices under /dev/, no traversal, bounded length.
fn validate_device(path: &str) -> Result<(), String> {
    let ok = path.starts_with("/dev/")
        && path.len() <= 128
        && !path.split('/').any(|c| c == "..")
        && path.bytes().all(|b| b.is_ascii_graphic());
    if ok {
        Ok(())
    } else {
        Err(format!("--device must be a path under /dev/, got {path:?}"))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn open(cfg: &Config) -> Result<Box<dyn serialport::SerialPort>, String> {
    serialport::new(&cfg.device, cfg.baud)
        .data_bits(serialport::DataBits::Eight)
        .stop_bits(serialport::StopBits::One)
        .parity(serialport::Parity::None)
        .flow_control(serialport::FlowControl::None)
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|e| e.to_string())
}

fn emit(report: &window::Report) {
    match serde_json::to_string(report) {
        Ok(line) => {
            let mut out = std::io::stdout().lock();
            // A closed stdout (runner gone) is not recoverable; stop cleanly.
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                std::process::exit(0);
            }
        }
        Err(e) => eprintln!("ld6002-radar: serialize failed: {e}"),
    }
}

/// The spatial-evidence emitter and its last write error since the previous window.
#[cfg(feature = "spatial-evidence")]
#[derive(Default)]
struct SpatialState {
    emitter: Option<spatial::Emitter>,
    error: Option<String>,
}

#[cfg(feature = "spatial-evidence")]
impl SpatialState {
    /// Turn one target frame into `spatial.evidence.v1` lines and append them to the file.
    /// The frame is decoded again here so the window's own decoding stays untouched.
    fn frame(&mut self, f: &frame::Frame, t_ms: u64) {
        let Some(e) = self.emitter.as_mut() else {
            return;
        };
        let records = match f.ty {
            window::T_POINTS => targets::parse_points(&f.payload, t_ms)
                .map(|p| e.points(t_ms, &p))
                .unwrap_or_default(),
            window::T_NEAREST => targets::parse_nearest(&f.payload)
                .and_then(|n| e.nearest(t_ms, n))
                .into_iter()
                .collect(),
            _ => return,
        };
        let lines: Vec<String> = records
            .iter()
            .filter_map(|r| serde_json::to_string(r).ok())
            .collect();
        if lines.is_empty() {
            return;
        }
        if let Err(err) = spatial::append(e.path(), &lines) {
            self.error = Some(format!("{}: {err}", e.path()));
        }
    }
}

/// Read one window from `port`, feeding the long-lived framer so frames that
/// straddle a window boundary are not lost. Returns the device error, if any.
fn read_window(
    port: &mut dyn serialport::SerialPort,
    framer: &mut Framer,
    win: &mut Window,
    deadline: Instant,
    bytes_read: &mut u64,
    #[cfg(feature = "spatial-evidence")] spatial: &mut SpatialState,
) -> Option<String> {
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                *bytes_read += n as u64;
                let frames = framer.feed(&buf[..n]);
                let t_ms = now_ms(); // every frame in this read finished parsing now
                for f in frames {
                    #[cfg(feature = "spatial-evidence")]
                    spatial.frame(&f, t_ms);
                    win.ingest(&f, t_ms);
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(e) => return Some(e.to_string()),
        }
    }
    None
}

fn run(cfg: &Config) {
    let window_len = Duration::from_secs(cfg.interval);
    let mut port: Option<Box<dyn serialport::SerialPort>> = None;
    let mut framer = Framer::new();
    #[cfg(feature = "spatial-evidence")]
    let (mut spatial_state, spatial_refusal, spatial_status) = match cfg.spatial.clone() {
        None => (SpatialState::default(), None, None),
        Some(s) => {
            let status = s.refusal_status();
            // The cog has no simulator: every line comes from the UART and is MEASURED.
            match spatial::Emitter::new(s, false) {
                Ok(e) => (
                    SpatialState {
                        emitter: Some(e),
                        error: None,
                    },
                    None,
                    Some("emitting"),
                ),
                Err(why) => {
                    eprintln!("ld6002-radar: not emitting spatial.evidence.v1: {why}");
                    (SpatialState::default(), Some(why), status)
                }
            }
        }
    };
    loop {
        let start_ms = now_ms();
        let deadline = Instant::now() + window_len;
        let mut win = Window::default();
        let mut bytes_read = 0;
        let mut device_error = None;

        if port.is_none() {
            match open(cfg) {
                Ok(p) => {
                    eprintln!("ld6002-radar: opened {} at {} baud", cfg.device, cfg.baud);
                    framer = Framer::new(); // never splice bytes across a reopen
                    port = Some(p);
                }
                Err(e) => device_error = Some(e),
            }
        }
        let before = framer.stats();
        if let Some(p) = port.as_mut() {
            if let Some(e) = read_window(
                p.as_mut(),
                &mut framer,
                &mut win,
                deadline,
                &mut bytes_read,
                #[cfg(feature = "spatial-evidence")]
                &mut spatial_state,
            ) {
                eprintln!("ld6002-radar: read error on {}: {e}", cfg.device);
                device_error = Some(e);
                port = None;
            }
        }
        let report = win.finish(WindowContext {
            start_ms,
            end_ms: now_ms(),
            device: cfg.device.clone(),
            stats: framer.stats().since(&before),
            bytes_read,
            device_error: device_error.clone(),
        });
        #[cfg(feature = "spatial-evidence")]
        let report = {
            let mut r = report;
            r.spatial = spatial_status;
            if let Some(why) = &spatial_refusal {
                r.reasons.push(format!("spatial_evidence_refused: {why}"));
            }
            if let Some(e) = spatial_state.error.take() {
                r.reasons.push(format!("spatial_evidence_error: {e}"));
                r.spatial = Some("write_error");
            }
            r
        };
        emit(&report);
        if cfg.once {
            return;
        }
        if device_error.is_some() {
            // Wait out the rest of the window before retrying the open.
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    cog_sensor_sources::handle_help(
        &args,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        include_str!("../cog.toml"),
    );
    match parse_args(&args) {
        Ok(cfg) => run(&cfg),
        Err(e) => {
            eprintln!("ld6002-radar: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn defaults() {
        let c = parse_args(&[]).unwrap();
        assert_eq!(
            c,
            Config {
                once: false,
                interval: 1,
                device: "/dev/ttyUSB0".into(),
                baud: 115_200,
                #[cfg(feature = "spatial-evidence")]
                spatial: None,
            }
        );
    }

    #[test]
    fn every_console_command_parses() {
        let toml = include_str!("../cog.toml");
        let line = toml
            .lines()
            .find(|l| l.starts_with("allowed_commands"))
            .unwrap();
        let cmds: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        assert!(cmds.len() >= 4);
        for c in cmds.iter().filter(|c| **c != "--help") {
            parse_args(&args(c)).unwrap_or_else(|e| panic!("{c}: {e}"));
        }
    }

    #[test]
    fn flags() {
        let c = parse_args(&args(
            "--once --interval 5 --device /dev/cu.usbserial-X --baud 9600",
        ))
        .unwrap();
        assert_eq!(
            c,
            Config {
                once: true,
                interval: 5,
                device: "/dev/cu.usbserial-X".into(),
                baud: 9600,
                #[cfg(feature = "spatial-evidence")]
                spatial: None,
            }
        );
    }

    /// Encoded `0x0A04` and `0x0A17` frames through the framer and the spatial sink, as
    /// `read_window` drives them. Set `LD6002_SPATIAL_KEEP=<file>` to keep the lines.
    #[cfg(feature = "spatial-evidence")]
    #[test]
    fn frames_become_spatial_lines_in_the_file() {
        use crate::frame::encode;
        use crate::window::{T_NEAREST, T_POINTS};
        let path = std::env::temp_dir().join(format!("ld6002-e2e-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut a = spatial::Args::default();
        for (f, v) in [
            ("--spatial-out", path.to_str().unwrap()),
            ("--radar-pose", "0.5,0.5,1.8,26,-10,1"),
            ("--spatial-region", "region/urth/meso/test-room"),
            ("--spatial-source-id", "ld6002-1"),
        ] {
            a.set(f, v).unwrap();
        }
        let mut st = SpatialState {
            // Synthetic frames, so SYNTHETIC lines.
            emitter: Some(spatial::Emitter::new(a.finish().unwrap().unwrap(), true).unwrap()),
            error: None,
        };
        let mut framer = Framer::new();
        let pts = |recs: &[[f32; 4]]| {
            let mut p = (recs.len() as u32).to_le_bytes().to_vec();
            recs.iter()
                .flatten()
                .for_each(|v| p.extend(v.to_le_bytes()));
            encode(1, T_POINTS, &p)
        };
        let near =
            |x: f32, y: f32| encode(2, T_NEAREST, &[x.to_le_bytes(), y.to_le_bytes()].concat());
        let t0 = 1_759_500_001_000;
        for k in 0..10u64 {
            let (x, y) = (-0.2 + 0.05 * k as f32, 1.2 + 0.1 * k as f32);
            let mut bytes = Vec::new();
            if k < 6 {
                let mut recs = vec![[x, y, f32::NAN, 0.0]];
                if k % 2 == 1 {
                    recs.push([0.6, 2.4, 0.0, 0.0]);
                }
                bytes.extend(pts(&recs));
            }
            bytes.extend(near(x, y));
            // 0x0A04 at about 10 Hz, then 0x0A17 alone every 600 ms.
            let t = t0 + if k < 6 { k * 100 } else { 500 + (k - 5) * 600 };
            for f in framer.feed(&bytes) {
                st.frame(&f, t);
            }
        }
        assert!(st.error.is_none());
        let text = std::fs::read_to_string(&path).unwrap();
        if let Ok(keep) = std::env::var("LD6002_SPATIAL_KEEP") {
            std::fs::copy(&path, keep).unwrap();
        }
        std::fs::remove_file(&path).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        // 6 point frames with 9 records, then 4 nearest-only frames after the gap.
        assert_eq!(lines.len(), 13, "{text}");
        for l in &lines {
            assert_eq!(l["type"], "radar_track_point");
            assert_eq!(l["provenance"]["proof"], "SYNTHETIC");
            assert_eq!(l["sensor"]["elev_half_deg"], 60.0);
        }
        assert_eq!(lines[12]["provenance"]["receipt"], "ld6002-1:000013");
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "--interval 0",
            "--interval 3601",
            "--interval x",
            "--interval",
            "--baud 100",
            "--device /etc/passwd",
            "--device /dev/../etc/passwd",
            "--device relative",
            "--verbose",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad} should be rejected");
        }
    }
}
