//! Cognitum Cog: rd-03e
//!
//! An Ai-Thinker RD-03E 24 GHz FMCW presence + ranging radar (S3KM111L SoC) read over a 3.3 V TTL
//! UART at 256000 8N1. The module streams a short fixed report frame on power-up (no command needed):
//!
//!   byte 0 = 0xAA (header) · 1 = distance_lo · 2 = distance_hi · 3 = state · 4 = 0x55 (footer)
//!
//! distance = buf[1] | (buf[2] << 8) in cm; state 0 = no target, 1 = present, 2..8 = motion/gesture
//! codes. No checksum — validated on the 0xAA header + 0x55 footer, resyncing on the next 0xAA.
//! Frame layout CONFIRMED by community captures; gesture-code meanings are vendor-specific and only
//! partially documented (reported as raw `state`). Implements the ADR-001 cog-as-plugin contract.
//!
//! Usage:
//!   cog-rd-03e --once                      # read until one report, emit it, exit
//!   cog-rd-03e --interval 1                # continuous, one report per second
//!   cog-rd-03e --once --simulate           # synthetic frames, no hardware needed
//!   cog-rd-03e --port /dev/ttyUSB0         # serial device (default autodetects USB-serial)
//!
//! Builds with the cargo feature `spatial-evidence` also take `--spatial-out export|FILE`,
//! `--radar-pose ...` and `--spatial-region ...` and write `radar_range` lines (src/spatial.rs).

mod export;
mod guide;
mod serial;
#[cfg(feature = "spatial-evidence")]
mod spatial;

use export::{round, Sample, Shared, State};
use serial::Serial;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TAG: &str = "[cog-rd-03e]";
/// Per-cog id in the store vector tuple (21=ecg, 22=tof, 23=sound, 24=rd-03e radar).
const STORE_ID: u32 = 24;
/// Default UART baud for the RD-03E report stream.
const BAUD: u32 = 256_000;
/// Clamp distance to this maximum (module spec tops out ~6 m) when normalizing the store vector.
const MAX_CM: f64 = 600.0;

struct Opts {
    once: bool,
    interval: u64,
    window: u64,
    port: String,
    baud: u32,
    api_bind: String,
    simulate: bool,
}

fn arg<'a>(a: &'a [String], f: &str) -> Option<&'a str> {
    a.iter()
        .position(|x| x == f)
        .and_then(|i| a.get(i + 1))
        .map(String::as_str)
}

fn num<T: std::str::FromStr + PartialOrd + Copy>(a: &[String], f: &str, d: T, lo: T, hi: T) -> T {
    match arg(a, f).and_then(|v| v.parse::<T>().ok()) {
        Some(v) if v >= lo && v <= hi => v,
        Some(_) => {
            eprintln!("{TAG} {f} out of range, using default");
            d
        }
        None => d,
    }
}

/// Pick a plausible USB-serial device if the user did not name one.
fn default_port() -> String {
    const CANDIDATES: [&str; 5] = [
        "/dev/ttyUSB0",
        "/dev/ttyACM0",
        "/dev/tty.usbserial-0001",
        "/dev/tty.usbserial",
        "/dev/tty.SLAB_USBtoUART",
    ];
    for c in CANDIDATES {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    CANDIDATES[0].to_string()
}

fn parse_opts(a: &[String]) -> Opts {
    Opts {
        once: a.iter().any(|x| x == "--once"),
        interval: num(a, "--interval", 1, 1, 60),
        window: num(a, "--window", 2, 1, 30),
        port: arg(a, "--port")
            .map(str::to_string)
            .unwrap_or_else(default_port),
        baud: num(a, "--baud", BAUD, 9600, 921_600),
        api_bind: arg(a, "--api-bind").unwrap_or("0.0.0.0:8050").to_string(),
        simulate: a.iter().any(|x| x == "--simulate"),
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// A decoded RD-03E report frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    distance_cm: u16,
    state: u8,
}

/// Drain complete 5-byte `AA .. .. .. 55` frames from the accumulator, resyncing on bad framing.
fn parse_frames(acc: &mut Vec<u8>) -> Vec<Frame> {
    let mut out = Vec::new();
    loop {
        // Discard anything before the next header.
        match acc.iter().position(|&b| b == 0xAA) {
            Some(0) => {}
            Some(i) => {
                acc.drain(..i);
            }
            None => {
                acc.clear();
                break;
            }
        }
        if acc.len() < 5 {
            break; // wait for the rest of the frame
        }
        if acc[4] == 0x55 {
            out.push(Frame {
                distance_cm: u16::from(acc[1]) | (u16::from(acc[2]) << 8),
                state: acc[3],
            });
            acc.drain(..5);
        } else {
            // Misaligned (e.g. a stray trailing 0x55): drop this header and resync on the next 0xAA.
            acc.drain(..1);
        }
    }
    out
}

/// A source of raw UART bytes. Both the real serial port and the simulator implement this, so the
/// exact same frame parser runs in `--simulate` as against hardware.
trait ByteSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn describe(&self) -> String;
}

struct SerialSource {
    dev: Serial,
    desc: String,
}

impl ByteSource for SerialSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.dev.read(buf)
    }
    fn describe(&self) -> String {
        self.desc.clone()
    }
}

/// Emits synthetic `AA .. .. .. 55` frames at ~20 Hz: a person that drifts toward and away, dropping
/// to "no target" at the far end. Paces itself so the read loop does not spin.
struct RadarSim {
    next: Instant,
    phase: f64,
}

impl RadarSim {
    fn new() -> Self {
        Self {
            next: Instant::now(),
            phase: 0.0,
        }
    }
}

impl ByteSource for RadarSim {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let now = Instant::now();
        if now < self.next {
            std::thread::sleep(self.next - now);
        }
        self.next += Duration::from_millis(50); // ~20 Hz
        self.phase += 0.08;
        // 50..350 cm sweep; "leave" (no target) across part of the cycle.
        let d = 200.0 + 150.0 * self.phase.sin();
        let present = self.phase.cos() > -0.3;
        let (dist, state) = if present {
            (d.round() as u16, 1u8)
        } else {
            (0u16, 0u8)
        };
        let frame = [0xAA, (dist & 0xFF) as u8, (dist >> 8) as u8, state, 0x55];
        let n = frame.len().min(buf.len());
        buf[..n].copy_from_slice(&frame[..n]);
        Ok(n)
    }
    fn describe(&self) -> String {
        "simulated RD-03E (20 Hz sweep)".to_string()
    }
}

/// Turn one frame into a `spatial.evidence.v1` line (rate-limited) and hand it to the sink.
/// Called with the state lock held.
#[cfg(feature = "spatial-evidence")]
fn emit_spatial(e: &mut spatial::Emitter, t_ms: u64, f: Frame, st: &mut State) {
    let Some(line) = e
        .record(t_ms, f.distance_cm, f.state)
        .and_then(|r| serde_json::to_string(&r).ok())
    else {
        return;
    };
    match e.sink() {
        spatial::Sink::File(path) => {
            if let Err(err) = spatial::append(path, &[line]) {
                st.spatial_error = Some(format!("{path}: {err}"));
            }
        }
        spatial::Sink::Export => st.push_evidence(t_ms, line),
    }
}

/// Read bytes, parse frames, and keep shared state current: a distance trace and presence onsets.
fn read_loop(
    mut src: Box<dyn ByteSource + Send>,
    shared: Shared,
    #[cfg(feature = "spatial-evidence")] mut spatial: Option<spatial::Emitter>,
) {
    let mut acc: Vec<u8> = Vec::with_capacity(64);
    let mut buf = [0u8; 256];
    let mut errors = 0u64;
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => continue, // idle read timeout (VTIME); keep waiting
            Ok(n) => n,
            Err(e) => {
                errors += 1;
                if errors.is_multiple_of(200) {
                    eprintln!("{TAG} read error: {e}");
                }
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
        };
        acc.extend_from_slice(&buf[..n]);
        if acc.len() > 4096 {
            acc.drain(..acc.len() - 4096); // bound a runaway buffer
        }
        let frames = parse_frames(&mut acc);
        if frames.is_empty() {
            continue;
        }
        let t_ms = unix_ms();
        if let Ok(mut st) = shared.lock() {
            for f in frames {
                #[cfg(feature = "spatial-evidence")]
                if let Some(e) = spatial.as_mut() {
                    emit_spatial(e, t_ms, f, &mut st);
                }
                let was_present = st.last_state != 0;
                let now_present = f.state != 0;
                if now_present && !was_present {
                    st.mark_event(t_ms);
                }
                st.last_state = f.state;
                st.last_distance_cm = f.distance_cm;
                st.frames += 1;
                st.push(Sample {
                    t_ms,
                    v: f.distance_cm as f64,
                });
            }
        }
    }
}

fn state_name(state: u8) -> &'static str {
    match state {
        0 => "no_target",
        1 => "present",
        _ => "gesture", // 2..8 are vendor-specific motion/gesture codes; raw value kept in `state`
    }
}

fn build_report(
    st: &State,
    window_s: f64,
    export_url: &Option<String>,
) -> (serde_json::Value, Vec<f64>) {
    let now_ms = unix_ms();
    let win = st.tail(window_s);
    let present = st.last_state != 0;
    let distance_cm = st.last_distance_cm;
    let events_60s = st.events_in(60, now_ms);
    let quiet_s = st
        .last_event_ms
        .map(|e| (now_ms.saturating_sub(e)) as f64 / 1000.0);
    let status = if st.frames == 0 {
        "no_frames"
    } else if present {
        "present"
    } else {
        "clear"
    };

    let report = serde_json::json!({
        "status": status,
        "source": st.source,
        "present": present,
        "distance_cm": distance_cm,
        "state": st.last_state,
        "state_name": state_name(st.last_state),
        "detections_per_min": events_60s,
        "total_detections": st.total_events,
        "quiet_s": quiet_s.map(|q| round(q, 1)),
        "frames": st.frames,
        "window_s": window_s,
        "samples": win.len(),
        "export": export_url,
        "timestamp": now_ms / 1000,
    });

    let vector: Vec<f64> = [
        if present { 1.0 } else { 0.0 },
        f64::from(distance_cm) / MAX_CM,
        f64::from(st.last_state) / 8.0,
        events_60s as f64 / 60.0,
        quiet_s.map(|q| q / 60.0).unwrap_or(1.0),
        0.0,
        0.0,
        0.0,
    ]
    .into_iter()
    .map(|v| v.clamp(0.0, 1.0))
    .collect();
    (report, vector)
}

fn store_to_seed(vector: &[f64]) -> Result<(), String> {
    let payload = serde_json::json!({ "vectors": [[STORE_ID, vector]], "dedup": true });
    let body = serde_json::to_vec(&payload).map_err(|e| format!("json: {e}"))?;
    let mut conn =
        std::net::TcpStream::connect("127.0.0.1:80").map_err(|e| format!("connect: {e}"))?;
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    conn.set_write_timeout(Some(Duration::from_secs(5))).ok();
    write!(conn, "POST /api/v1/store/ingest HTTP/1.0\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).map_err(|e| format!("write: {e}"))?;
    conn.write_all(&body).map_err(|e| format!("body: {e}"))?;
    let mut resp = Vec::new();
    let mut buf = [0u8; 1024];
    while !response_complete(&resp) {
        match conn.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => resp.extend_from_slice(&buf[..n]),
        }
    }
    let status = String::from_utf8_lossy(&resp)
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    if status.starts_with('2') {
        Ok(())
    } else {
        Err(format!("ingest replied {status:?}"))
    }
}

fn response_complete(resp: &[u8]) -> bool {
    let Some(end) = resp.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&resp[..end]).to_ascii_lowercase();
    let len = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    resp.len() >= end + 4 + len
}

fn open_source(o: &Opts) -> Result<Box<dyn ByteSource + Send>, String> {
    if o.simulate {
        return Ok(Box::new(RadarSim::new()));
    }
    let dev = Serial::open(&o.port, o.baud)?;
    Ok(Box::new(SerialSource {
        dev,
        desc: format!("RD-03E on {} @ {} 8N1", o.port, o.baud),
    }))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    cog_sensor_sources::handle_help(
        &args,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        include_str!("../cog.toml"),
    );
    let o = parse_opts(&args);
    #[cfg(feature = "spatial-evidence")]
    let spatial_settings = match spatial::Args::from_args(&args).and_then(|a| a.finish()) {
        Ok(Some(s)) if o.once && s.sink == spatial::Sink::Export => {
            eprintln!("{TAG} --spatial-out export needs continuous mode (no --once)");
            std::process::exit(2);
        }
        Ok(s) => s,
        Err(e) => {
            eprintln!("{TAG} {e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "{TAG} start (once={}, interval={}s, window={}s, port={}, baud={}, simulate={})",
        o.once, o.interval, o.window, o.port, o.baud, o.simulate
    );

    let shared: Shared = Arc::new(Mutex::new(State::new(f64::from(o.baud), "none".into())));
    let export_url = if o.once && !o.simulate && open_source(&o).is_err() {
        None
    } else {
        match export::serve(&o.api_bind, shared.clone()) {
            Ok(addr) => Some(format!("http://{addr}/raw")),
            Err(e) => {
                eprintln!("{TAG} export disabled: {e}");
                None
            }
        }
    };

    let src = loop {
        match open_source(&o) {
            Ok(s) => break s,
            Err(e) => {
                let r =
                    serde_json::json!({"status":"no_source","error":e,"timestamp":unix_ms()/1000});
                println!("{r}");
                eprintln!("{TAG} no source: {e}");
                if let Ok(mut st) = shared.lock() {
                    st.latest_report = Some(r);
                }
                if o.once {
                    return;
                }
                std::thread::sleep(Duration::from_secs(o.interval.clamp(1, 5)));
            }
        }
    };
    if let Ok(mut st) = shared.lock() {
        st.source = src.describe();
    }
    #[cfg(feature = "spatial-evidence")]
    let (emitter, spatial_refusal, spatial_status) = match spatial_settings {
        None => (None, None, None),
        Some(s) => {
            let status = s.refusal_status();
            match spatial::Emitter::new(s, o.simulate) {
                Ok(e) => (Some(e), None, Some("emitting")),
                Err(why) => {
                    eprintln!("{TAG} not emitting spatial.evidence.v1: {why}");
                    (None, Some(why), status)
                }
            }
        }
    };
    std::thread::spawn({
        let shared = shared.clone();
        move || {
            read_loop(
                src,
                shared,
                #[cfg(feature = "spatial-evidence")]
                emitter,
            )
        }
    });

    let period = if o.once { o.window } else { o.interval };
    loop {
        std::thread::sleep(Duration::from_secs(period));
        let (report, vector) = match shared.lock() {
            Ok(mut st) => {
                let out = build_report(&st, period as f64, &export_url);
                #[cfg(feature = "spatial-evidence")]
                let out = {
                    let mut out = out;
                    let mut status = spatial_status;
                    let mut reasons = Vec::new();
                    if let Some(why) = &spatial_refusal {
                        reasons.push(format!("spatial_evidence_refused: {why}"));
                    }
                    if let Some(e) = st.spatial_error.take() {
                        reasons.push(format!("spatial_evidence_error: {e}"));
                        status = Some("write_error");
                    }
                    out.0["spatial"] = serde_json::json!(status);
                    if !reasons.is_empty() {
                        out.0["reasons"] = serde_json::json!(reasons);
                    }
                    out
                };
                st.latest_report = Some(out.0.clone());
                out
            }
            Err(_) => {
                eprintln!("{TAG} state poisoned");
                return;
            }
        };
        println!("{report}");
        if report["status"] != "no_frames" {
            if let Err(e) = store_to_seed(&vector) {
                eprintln!("{TAG} store error: {e}");
            }
        }
        if o.once {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        std::iter::once("cog")
            .chain(v.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn options_bounded_and_defaulted() {
        let o = parse_opts(&s(&["--once", "--interval", "999", "--baud", "115200"]));
        assert!(o.once);
        assert_eq!(o.interval, 1); // out of range -> default
        assert_eq!(o.baud, 115_200);
        assert_eq!(parse_opts(&s(&[])).api_bind, "0.0.0.0:8050");
    }

    #[test]
    fn parses_confirmed_example_captures() {
        // AA 2D 00 00 55 -> 45 cm, no target; AA 36 00 01 55 -> 54 cm, present.
        let mut acc = vec![0xAA, 0x2D, 0x00, 0x00, 0x55, 0xAA, 0x36, 0x00, 0x01, 0x55];
        let f = parse_frames(&mut acc);
        assert_eq!(
            f,
            vec![
                Frame {
                    distance_cm: 45,
                    state: 0
                },
                Frame {
                    distance_cm: 54,
                    state: 1
                }
            ]
        );
        assert!(acc.is_empty());
    }

    #[test]
    fn resyncs_on_garbage_and_waits_for_partial() {
        // Leading noise, then a good frame, then a partial that must be retained.
        let mut acc = vec![0x11, 0x22, 0xAA, 0x64, 0x00, 0x01, 0x55, 0xAA, 0x10];
        let f = parse_frames(&mut acc);
        assert_eq!(
            f,
            vec![Frame {
                distance_cm: 100,
                state: 1
            }]
        );
        assert_eq!(acc, vec![0xAA, 0x10]); // partial kept for the next read
    }

    #[test]
    fn bad_footer_drops_one_byte_and_resyncs() {
        // Header with a wrong footer followed by a valid frame.
        let mut acc = vec![0xAA, 0x01, 0x00, 0x00, 0x99, 0xAA, 0x0A, 0x00, 0x01, 0x55];
        let f = parse_frames(&mut acc);
        assert_eq!(
            f,
            vec![Frame {
                distance_cm: 10,
                state: 1
            }]
        );
    }

    #[test]
    fn simulator_feeds_the_real_parser_and_vector_is_bounded() {
        let mut sim = RadarSim::new();
        let mut acc = Vec::new();
        let mut buf = [0u8; 64];
        let mut st = State::new(256_000.0, "sim".into());
        for _ in 0..40 {
            let n = sim.read(&mut buf).unwrap();
            acc.extend_from_slice(&buf[..n]);
            for f in parse_frames(&mut acc) {
                let was = st.last_state != 0;
                if f.state != 0 && !was {
                    st.mark_event(unix_ms());
                }
                st.last_state = f.state;
                st.last_distance_cm = f.distance_cm;
                st.frames += 1;
                st.push(Sample {
                    t_ms: unix_ms(),
                    v: f.distance_cm as f64,
                });
            }
        }
        assert!(st.frames >= 30, "expected frames, got {}", st.frames);
        let (_r, v) = build_report(&st, 2.0, &None);
        assert_eq!(v.len(), 8);
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
    }
}
