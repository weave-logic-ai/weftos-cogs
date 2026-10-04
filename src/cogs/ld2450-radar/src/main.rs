//! cog-ld2450-radar: HLK-LD2450 24 GHz multi-target tracking radar reader. ADR-161.
//!
//! Usage:
//!   cog-ld2450-radar [--once] [--interval SECS] [--device /dev/serial0] [--baud 256000]
//!                    [--query-firmware] [--tracking-mode keep|single|multi]
//!                    [--api-bind 127.0.0.1:8052] [--simulate] [--replay FILE]
//!
//! Reads the radar's UART continuously and prints one JSON line per `--interval` window to
//! stdout (diagnostics go to stderr). `--once` reads one window and exits. A missing or
//! silent device yields a `health:"no_source"` line with null values, never a fabricated
//! reading, and `--once` still exits 0. Targets are in the radar's local frame; placing
//! them in a room is the fusion host's job. By default the cog never writes to the radar.

mod export;
mod frame;
mod guide;
mod source;
#[cfg(feature = "spatial-evidence")]
mod spatial;
mod window;

use std::io::Write;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use export::{Ring, Sample, Shared};
use frame::{cmd, Ack, Frame, Framer};
use source::{ByteSource, Simulator, READ_TIMEOUT};
use window::{SourceKind, Window, WindowContext};

const DEFAULT_DEVICE: &str = "/dev/serial0";
const DEFAULT_BAUD: u32 = 256_000;
const DEFAULT_INTERVAL: u64 = 1;
const DEFAULT_API_BIND: &str = "127.0.0.1:8052";
/// Baud rates the radar can be set to (protocol V1.03 Table 6).
const BAUDS: [u32; 8] = [
    9600, 19_200, 38_400, 57_600, 115_200, 230_400, 256_000, 460_800,
];
/// How long to wait for each command's ACK.
const ACK_TIMEOUT: Duration = Duration::from_millis(1000);
/// Largest `--replay` capture accepted.
const MAX_REPLAY_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackingMode {
    Keep,
    Single,
    Multi,
}

#[derive(Debug, PartialEq)]
struct Config {
    once: bool,
    interval: u64,
    device: String,
    baud: u32,
    query_firmware: bool,
    tracking_mode: TrackingMode,
    api_bind: String,
    simulate: bool,
    replay: Option<String>,
    /// `spatial.evidence.v1` output; `None` unless `--spatial-out` is set.
    #[cfg(feature = "spatial-evidence")]
    spatial: Option<spatial::Settings>,
}

impl Config {
    /// True when the cog will send any command to the radar.
    fn writes(&self) -> bool {
        self.query_firmware || self.tracking_mode != TrackingMode::Keep
    }
}

fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut cfg = Config {
        once: false,
        interval: DEFAULT_INTERVAL,
        device: DEFAULT_DEVICE.into(),
        baud: DEFAULT_BAUD,
        query_firmware: false,
        tracking_mode: TrackingMode::Keep,
        api_bind: DEFAULT_API_BIND.into(),
        simulate: false,
        replay: None,
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
            "--simulate" => cfg.simulate = true,
            "--query-firmware" => cfg.query_firmware = true,
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
                    .filter(|n| BAUDS.contains(n))
                    .ok_or(format!("--baud must be one of {BAUDS:?}, got {v:?}"))?;
            }
            "--device" => {
                let v = value()?;
                validate_device(&v)?;
                cfg.device = v;
            }
            "--tracking-mode" => {
                cfg.tracking_mode = match value()?.as_str() {
                    "keep" => TrackingMode::Keep,
                    "single" => TrackingMode::Single,
                    "multi" => TrackingMode::Multi,
                    v => {
                        return Err(format!(
                            "--tracking-mode must be keep|single|multi, got {v:?}"
                        ))
                    }
                }
            }
            "--api-bind" => {
                let v = value()?;
                v.parse::<SocketAddr>()
                    .map_err(|_| format!("--api-bind must be host:port, got {v:?}"))?;
                cfg.api_bind = v;
            }
            "--replay" => {
                let v = value()?;
                validate_replay_path(&v)?;
                cfg.replay = Some(v);
            }
            #[cfg(feature = "spatial-evidence")]
            f if spatial::Args::FLAGS.contains(&f) => sp.set(f, &value()?)?,
            other => return Err(format!("unknown argument {other:?} (see --help)")),
        }
    }
    #[cfg(feature = "spatial-evidence")]
    {
        cfg.spatial = sp.finish()?;
        if let Some(s) = &cfg.spatial {
            if cfg.replay.is_some() {
                return Err("--spatial-out is not supported with --replay".into());
            }
            if cfg.once && s.sink == spatial::Sink::Export {
                return Err("--spatial-out export needs continuous mode (no --once)".into());
            }
        }
    }
    if cfg.simulate && cfg.replay.is_some() {
        return Err("--simulate and --replay are exclusive".into());
    }
    if cfg.replay.is_some() && cfg.writes() {
        return Err("--replay cannot send commands (--query-firmware, --tracking-mode)".into());
    }
    Ok(cfg)
}

/// Least authority: only device paths under /dev/, no traversal, bounded length.
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

/// A replay is a captured byte file, never a device or a traversal.
fn validate_replay_path(path: &str) -> Result<(), String> {
    let ok = !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with("/dev/")
        && !path.starts_with("/proc/")
        && !path.starts_with("/sys/")
        && !path.split('/').any(|c| c == "..")
        && !path.contains('\0');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "--replay must be a capture file path, got {path:?}"
        ))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn open_uart(cfg: &Config) -> Result<Box<dyn serialport::SerialPort>, String> {
    serialport::new(&cfg.device, cfg.baud)
        .data_bits(serialport::DataBits::Eight)
        .stop_bits(serialport::StopBits::One)
        .parity(serialport::Parity::None)
        .flow_control(serialport::FlowControl::None)
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|e| e.to_string())
}

fn emit(report: &window::Report, shared: Option<&Shared>) {
    match serde_json::to_string(report) {
        Ok(line) => {
            if let Some(ring) = shared.and_then(|s| s.lock().ok()).as_mut() {
                ring.latest_report = serde_json::from_str(&line).ok();
            }
            let mut out = std::io::stdout().lock();
            // A closed stdout (runner gone) is not recoverable; stop cleanly.
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                std::process::exit(0);
            }
        }
        Err(e) => eprintln!("ld2450-radar: serialize failed: {e}"),
    }
}

/// Decoder state that outlives a window, so frames that straddle a window boundary are
/// not lost.
struct Session {
    framer: Framer,
    shared: Option<Shared>,
    #[cfg(feature = "spatial-evidence")]
    spatial: Option<spatial::Emitter>,
    /// The last spatial-evidence write error since the previous window.
    #[cfg(feature = "spatial-evidence")]
    spatial_error: Option<String>,
}

impl Session {
    /// One read from `src`; report frames go into `win` (and the export), ACKs are returned.
    fn pump(
        &mut self,
        src: &mut dyn ByteSource,
        win: &mut Window,
        bytes_read: &mut u64,
    ) -> std::io::Result<Vec<Ack>> {
        let mut buf = [0u8; 4096];
        let n = src.read(&mut buf)?;
        let mut acks = Vec::new();
        if n == 0 {
            return Ok(acks);
        }
        *bytes_read += n as u64;
        let t_ms = now_ms(); // every frame in this read finished parsing now
        for f in self.framer.feed(&buf[..n]) {
            match f {
                Frame::Report(slots) => {
                    let targets = win.ingest_report(&slots, t_ms);
                    #[cfg(feature = "spatial-evidence")]
                    self.emit_spatial(t_ms, &targets);
                    if let Some(ring) = self.shared.as_ref().and_then(|s| s.lock().ok()).as_mut() {
                        ring.push(Sample { t_ms, targets });
                    }
                }
                Frame::Ack(a) => {
                    win.ingest_ack(&a);
                    acks.push(a);
                }
            }
        }
        Ok(acks)
    }

    /// Turn one frame's targets into `spatial.evidence.v1` lines and hand them to the sink.
    #[cfg(feature = "spatial-evidence")]
    fn emit_spatial(&mut self, t_ms: u64, targets: &[window::Target]) {
        let Some(e) = self.spatial.as_mut() else {
            return;
        };
        let lines: Vec<String> = e
            .records(t_ms, targets)
            .iter()
            .filter_map(|r| serde_json::to_string(r).ok())
            .collect();
        if lines.is_empty() {
            return;
        }
        match e.sink().clone() {
            spatial::Sink::File(path) => {
                if let Err(err) = spatial::append(&path, &lines) {
                    self.spatial_error = Some(format!("{path}: {err}"));
                }
            }
            spatial::Sink::Export => {
                if let Some(ring) = self.shared.as_ref().and_then(|s| s.lock().ok()).as_mut() {
                    for l in lines {
                        ring.push_evidence(t_ms, l);
                    }
                }
            }
        }
    }

    /// Send one command and wait for its ACK.
    fn command(
        &mut self,
        src: &mut dyn ByteSource,
        win: &mut Window,
        bytes_read: &mut u64,
        bytes: &[u8],
        word: u16,
    ) -> Result<Ack, String> {
        src.write_all(bytes).map_err(|e| format!("write: {e}"))?;
        let deadline = Instant::now() + ACK_TIMEOUT;
        while Instant::now() < deadline {
            let acks = self
                .pump(src, win, bytes_read)
                .map_err(|e| format!("read: {e}"))?;
            if let Some(a) = acks.into_iter().find(|a| a.command == word) {
                return if a.status == 0 {
                    Ok(a)
                } else {
                    Err(format!("command {word:#06x} refused"))
                };
            }
        }
        Err(format!("no ACK for command {word:#06x}"))
    }

    /// Enable config, apply/query, end config. Returns (firmware, tracking mode).
    fn configure(
        &mut self,
        cfg: &Config,
        src: &mut dyn ByteSource,
        win: &mut Window,
        bytes_read: &mut u64,
    ) -> Result<(Option<String>, Option<&'static str>), String> {
        self.command(
            src,
            win,
            bytes_read,
            &frame::enable_config(),
            cmd::ENABLE_CONFIG,
        )?;
        let mut run = || -> Result<(Option<String>, Option<&'static str>), String> {
            match cfg.tracking_mode {
                TrackingMode::Keep => {}
                TrackingMode::Single => {
                    let c = frame::encode_command(cmd::SINGLE_TARGET, &[]);
                    self.command(src, win, bytes_read, &c, cmd::SINGLE_TARGET)?;
                }
                TrackingMode::Multi => {
                    let c = frame::encode_command(cmd::MULTI_TARGET, &[]);
                    self.command(src, win, bytes_read, &c, cmd::MULTI_TARGET)?;
                }
            }
            let c = frame::encode_command(cmd::QUERY_TRACKING_MODE, &[]);
            let mode = self.command(src, win, bytes_read, &c, cmd::QUERY_TRACKING_MODE)?;
            let mut firmware = None;
            if cfg.query_firmware {
                let c = frame::encode_command(cmd::READ_FIRMWARE, &[]);
                let a = self.command(src, win, bytes_read, &c, cmd::READ_FIRMWARE)?;
                firmware = frame::firmware_version(&a.value);
            }
            Ok((firmware, frame::tracking_mode(&mode.value)))
        };
        let result = run();
        // Always try to leave config mode, or the radar stays silent.
        let end = frame::encode_command(cmd::END_CONFIG, &[]);
        let ended = self.command(src, win, bytes_read, &end, cmd::END_CONFIG);
        let out = result?;
        ended.map(|_| out)
    }
}

/// Live loop over a UART or the simulator.
fn run_live(cfg: &Config) {
    let kind = if cfg.simulate {
        SourceKind::Simulated
    } else {
        SourceKind::Uart
    };
    let device = if cfg.simulate {
        "simulator".to_string()
    } else {
        cfg.device.clone()
    };
    // The export comes up first (continuous mode only) so a hook-up tool can watch
    // no_source turn into targets while the wiring is finished.
    let shared: Option<Shared> = (!cfg.once).then(|| {
        std::sync::Arc::new(std::sync::Mutex::new(Ring {
            source: kind.name().into(),
            ..Default::default()
        }))
    });
    if let Some(s) = &shared {
        match export::serve(&cfg.api_bind, s.clone()) {
            Ok(addr) => eprintln!("ld2450-radar: export on http://{addr}/"),
            Err(e) => eprintln!("ld2450-radar: export disabled: {e}"),
        }
    }
    let window_len = Duration::from_secs(cfg.interval);
    let mut port: Option<Box<dyn ByteSource>> = None;
    #[cfg(feature = "spatial-evidence")]
    let (emitter, spatial_refusal, spatial_status) = match cfg.spatial.clone() {
        None => (None, None, None),
        Some(s) => {
            let status = s.refusal_status();
            match spatial::Emitter::new(s, cfg.simulate) {
                Ok(e) => (Some(e), None, Some("emitting")),
                Err(why) => {
                    eprintln!("ld2450-radar: not emitting spatial.evidence.v1: {why}");
                    (None, Some(why), status)
                }
            }
        }
    };
    let mut session = Session {
        framer: Framer::new(),
        shared: shared.clone(),
        #[cfg(feature = "spatial-evidence")]
        spatial: emitter,
        #[cfg(feature = "spatial-evidence")]
        spatial_error: None,
    };
    let mut configured = false;
    let (mut firmware, mut tracking_mode) = (None, None);
    loop {
        let start_ms = now_ms();
        let deadline = Instant::now() + window_len;
        let mut win = Window::default();
        let mut bytes_read = 0;
        let mut device_error = None;
        let mut config_error = None;

        if port.is_none() {
            let opened: Result<Box<dyn ByteSource>, String> = if cfg.simulate {
                Ok(Box::new(Simulator::new()))
            } else {
                open_uart(cfg).map(|p| Box::new(p) as Box<dyn ByteSource>)
            };
            match opened {
                Ok(p) => {
                    eprintln!("ld2450-radar: opened {device} at {} baud", cfg.baud);
                    session.framer = Framer::new(); // never splice bytes across a reopen
                    port = Some(p);
                }
                Err(e) => device_error = Some(e),
            }
        }
        let before = session.framer.stats();
        if let Some(p) = port.as_mut() {
            if cfg.writes() && !configured {
                configured = true;
                match session.configure(cfg, p.as_mut(), &mut win, &mut bytes_read) {
                    Ok((fw, mode)) => (firmware, tracking_mode) = (fw, mode),
                    Err(e) => {
                        eprintln!("ld2450-radar: configure: {e}");
                        config_error = Some(e);
                    }
                }
            }
            while Instant::now() < deadline {
                if let Err(e) = session.pump(p.as_mut(), &mut win, &mut bytes_read) {
                    eprintln!("ld2450-radar: read error on {device}: {e}");
                    device_error = Some(e.to_string());
                    port = None;
                    break;
                }
            }
        }
        let report = win.finish(WindowContext {
            kind,
            start_ms,
            end_ms: now_ms(),
            device: device.clone(),
            stats: session.framer.stats().since(&before),
            bytes_read,
            device_error: device_error.clone(),
            config_error,
            firmware: firmware.clone(),
            tracking_mode,
        });
        #[cfg(feature = "spatial-evidence")]
        let report = {
            let mut r = report;
            r.spatial = spatial_status;
            if let Some(why) = &spatial_refusal {
                r.reasons.push(format!("spatial_evidence_refused: {why}"));
            }
            if let Some(e) = session.spatial_error.take() {
                r.reasons.push(format!("spatial_evidence_error: {e}"));
                r.spatial = Some("write_error");
            }
            r
        };
        emit(&report, shared.as_ref());
        if cfg.once {
            return;
        }
        if device_error.is_some() {
            // Wait out the rest of the window before retrying the open.
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
    }
}

/// Decode a captured byte file as fast as it parses. Windows hold about `interval x 10`
/// report frames (the radar's nominal 10 Hz); `frame_rate_hz` is null because a file has
/// no timing.
fn run_replay(cfg: &Config, path: &str) {
    let ctx = |stats, bytes_read, start_ms, err: Option<String>| WindowContext {
        kind: SourceKind::Replay,
        start_ms,
        end_ms: now_ms(),
        device: path.to_string(),
        stats,
        bytes_read,
        device_error: err,
        config_error: None,
        firmware: None,
        tracking_mode: None,
    };
    let bytes = match read_capture(path) {
        Ok(b) => b,
        Err(e) => {
            let r = Window::default().finish(ctx(Default::default(), 0, now_ms(), Some(e)));
            emit(&r, None);
            return;
        }
    };
    let per_window = cfg.interval * 10;
    let mut framer = Framer::new();
    let mut win = Window::default();
    let mut before = framer.stats();
    let (mut start_ms, mut bytes_read, mut emitted) = (now_ms(), 0u64, false);
    for chunk in bytes.chunks(32) {
        bytes_read += chunk.len() as u64;
        let t_ms = now_ms();
        for f in framer.feed(chunk) {
            match f {
                Frame::Report(s) => {
                    win.ingest_report(&s, t_ms);
                }
                Frame::Ack(a) => win.ingest_ack(&a),
            }
        }
        if framer.stats().since(&before).reports >= per_window {
            let stats = framer.stats().since(&before);
            let done = std::mem::take(&mut win);
            emit(&done.finish(ctx(stats, bytes_read, start_ms, None)), None);
            emitted = true;
            if cfg.once {
                return;
            }
            (before, start_ms, bytes_read) = (framer.stats(), now_ms(), 0);
        }
    }
    let stats = framer.stats().since(&before);
    if stats.reports > 0 || !emitted {
        emit(&win.finish(ctx(stats, bytes_read, start_ms, None)), None);
    }
}

fn read_capture(path: &str) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path}: not a regular file"));
    }
    if meta.len() > MAX_REPLAY_BYTES {
        return Err(format!("{path}: larger than {MAX_REPLAY_BYTES} bytes"));
    }
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
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
        Ok(cfg) => match cfg.replay.clone() {
            Some(path) => run_replay(&cfg, &path),
            None => run_live(&cfg),
        },
        Err(e) => {
            eprintln!("ld2450-radar: {e}");
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
    fn defaults_are_read_only() {
        let c = parse_args(&[]).unwrap();
        assert_eq!(c.device, "/dev/serial0");
        assert_eq!(c.baud, 256_000);
        assert_eq!(c.interval, 1);
        assert_eq!(c.api_bind, "127.0.0.1:8052");
        assert!(!c.writes());
        assert!(!c.once && !c.simulate && c.replay.is_none());
    }

    #[test]
    fn every_console_command_parses() {
        let toml = include_str!("../cog.toml");
        let line = toml
            .lines()
            .find(|l| l.starts_with("allowed_commands"))
            .unwrap();
        let cmds: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        assert!(cmds.len() >= 5);
        for c in cmds.iter().filter(|c| **c != "--help") {
            let cfg = parse_args(&args(c)).unwrap_or_else(|e| panic!("{c}: {e}"));
            assert!(cfg.once, "{c}: console commands must be --once");
            assert_eq!(
                cfg.tracking_mode,
                TrackingMode::Keep,
                "{c}: no config writes"
            );
        }
    }

    #[test]
    fn flags() {
        let c = parse_args(&args(
            "--once --interval 5 --device /dev/ttyUSB0 --baud 115200 --query-firmware \
             --tracking-mode single --api-bind 0.0.0.0:8052 --simulate",
        ))
        .unwrap();
        assert!(c.once && c.query_firmware && c.simulate && c.writes());
        assert_eq!((c.interval, c.baud), (5, 115_200));
        assert_eq!(c.device, "/dev/ttyUSB0");
        assert_eq!(c.tracking_mode, TrackingMode::Single);
        assert_eq!(c.api_bind, "0.0.0.0:8052");
        let r = parse_args(&args("--replay captures/ld2450.bin")).unwrap();
        assert_eq!(r.replay.as_deref(), Some("captures/ld2450.bin"));
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "--interval 0",
            "--interval 3601",
            "--interval",
            "--baud 250000",
            "--baud 9601",
            "--device /etc/passwd",
            "--device /dev/../etc/passwd",
            "--tracking-mode both",
            "--api-bind 8052",
            "--replay /dev/serial0",
            "--replay ../../etc/shadow",
            "--simulate --replay x.bin",
            "--replay x.bin --query-firmware",
            "--verbose",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn configure_against_the_simulator_reads_version_and_mode() {
        let cfg = parse_args(&args("--simulate --query-firmware --tracking-mode single")).unwrap();
        let mut sim = Simulator::new();
        let mut s = Session {
            framer: Framer::new(),
            shared: None,
            #[cfg(feature = "spatial-evidence")]
            spatial: None,
            #[cfg(feature = "spatial-evidence")]
            spatial_error: None,
        };
        let mut win = Window::default();
        let mut n = 0;
        let (fw, mode) = s.configure(&cfg, &mut sim, &mut win, &mut n).unwrap();
        assert_eq!(fw.as_deref(), Some("V0.00.00000000"));
        assert_eq!(mode, Some("single"));
    }

    #[test]
    fn simulated_window_end_to_end() {
        // Simulator bytes -> framer -> window -> JSON, the --simulate path minus the clock.
        let mut f = Framer::new();
        let mut win = Window::default();
        let mut bytes = Vec::new();
        for k in 150..160 {
            bytes.extend(frame::encode_report(&Simulator::frame(k, true)));
        }
        for fr in f.feed(&bytes) {
            if let Frame::Report(s) = fr {
                win.ingest_report(&s, 1);
            }
        }
        let r = win.finish(WindowContext {
            kind: SourceKind::Simulated,
            start_ms: 0,
            end_ms: 1000,
            device: "simulator".into(),
            stats: f.stats(),
            bytes_read: bytes.len() as u64,
            device_error: None,
            config_error: None,
            firmware: None,
            tracking_mode: None,
        });
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["frames"], 10);
        assert_eq!(v["frame_rate_hz"], 10.0);
        assert_eq!(v["target_count"], 2);
        assert_eq!(v["source"]["simulated"], true);
        assert_eq!(v["source"]["verified"], false);
        let t = &v["targets"][0];
        assert!(t["y_m"].as_f64().unwrap() > 1.0);
        assert_eq!(t["resolution_mm"], 360);
    }
}
