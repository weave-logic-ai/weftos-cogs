//! Cognitum Cog: SEN0628 matrix ToF
//!
//! DFRobot SEN0628 (VL53L7CX + RP2040, 8x8 or 4x4 zones, 20-3500 mm, 60 deg FOV) on the Seed's
//! I2C header (bus 1, default 0x33). Reads depth frames and reports the nearest object and its
//! zone, the valid-zone share, frame-to-frame motion, presence against a learned background,
//! left/middle/right sector distances and per-zone noise. ADR-159. Implements the ADR-001
//! cog-as-plugin contract.
//!
//! Usage:
//!   cog-sen0628-tof --once                 # set the mode, capture --window seconds, report, exit
//!   cog-sen0628-tof --interval 1           # run continuously, one report per second
//!   cog-sen0628-tof --once --simulate      # synthetic room with a walking person
//!
//! Builds with the cargo feature `spatial-evidence` also take `--spatial-out export|FILE`,
//! `--tof-pose ...` and `--spatial-region ...` and write `tof_depth` lines (src/spatial.rs).

mod export;
mod guide;
mod scene;
#[cfg(feature = "spatial-evidence")]
mod spatial;
mod tof;

use export::{Frame, Ring, Shared};
use scene::Background;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tof::{FrameSource, Sen0628, Simulator};

const TAG: &str = "[cog-sen0628-tof]";
/// Per-cog id used in the store vector tuple.
const STORE_ID: u32 = 22;

struct Opts {
    once: bool,
    interval: u64,
    window: u64,
    side: usize,
    rate_hz: u32,
    bus: u8,
    addr: u8,
    max_mm: u16,
    presence_mm: u16,
    learn_seconds: u64,
    api_bind: String,
    simulate: bool,
}

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn num<T: std::str::FromStr + PartialOrd + Copy>(
    args: &[String],
    flag: &str,
    default: T,
    lo: T,
    hi: T,
) -> T {
    match arg(args, flag).and_then(|v| v.parse::<T>().ok()) {
        Some(v) if v >= lo && v <= hi => v,
        Some(_) => {
            eprintln!("{TAG} {flag} out of range, using default");
            default
        }
        None => default,
    }
}

fn parse_addr(args: &[String]) -> u8 {
    let v = arg(args, "--i2c-addr").and_then(|s| match s.strip_prefix("0x") {
        Some(h) => u8::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    });
    match v {
        Some(a) if (0x30..=0x33).contains(&a) => a,
        Some(_) => {
            eprintln!("{TAG} --i2c-addr must be 48-51 (0x30-0x33), using 51");
            0x33
        }
        None => 0x33,
    }
}

fn parse_opts(args: &[String]) -> Opts {
    let side = match arg(args, "--mode") {
        Some("4") | Some("4x4") => 4,
        _ => 8,
    };
    Opts {
        once: args.iter().any(|a| a == "--once"),
        interval: num(args, "--interval", 1, 1, 60),
        window: num(args, "--window", 3, 1, 8),
        side,
        rate_hz: num(args, "--rate-hz", 10, 1, 15),
        bus: num(args, "--i2c-bus", 1, 0, 9),
        addr: parse_addr(args),
        max_mm: num(args, "--max-range-mm", 3500, 100, 4000),
        presence_mm: num(args, "--presence-mm", 150, 30, 1000),
        learn_seconds: num(args, "--learn-seconds", 5, 1, 60),
        api_bind: arg(args, "--api-bind")
            .unwrap_or("0.0.0.0:8047")
            .to_string(),
        simulate: args.iter().any(|a| a == "--simulate"),
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Default)]
struct Counters {
    frames: AtomicU64,
    read_errors: AtomicU64,
}

/// Turn one frame into a `spatial.evidence.v1` line (rate-limited) and hand it to the sink.
#[cfg(feature = "spatial-evidence")]
fn emit_spatial(e: &mut spatial::Emitter, f: &Frame, shared: &Shared) {
    let Some(line) = e
        .record(f.t_ms, f.side, &f.mm)
        .and_then(|r| serde_json::to_string(&r).ok())
    else {
        return;
    };
    match e.sink() {
        spatial::Sink::File(path) => {
            if let Err(err) = spatial::append(path, &[line]) {
                if let Ok(mut ring) = shared.lock() {
                    ring.spatial_error = Some(format!("{path}: {err}"));
                }
            }
        }
        spatial::Sink::Export => {
            if let Ok(mut ring) = shared.lock() {
                ring.push_evidence(f.t_ms, line);
            }
        }
    }
}

fn frame_loop(
    mut src: Box<dyn FrameSource>,
    rate_hz: u32,
    shared: Shared,
    counters: Arc<Counters>,
    #[cfg(feature = "spatial-evidence")] mut spatial: Option<spatial::Emitter>,
) {
    let period = Duration::from_secs_f64(1.0 / f64::from(rate_hz));
    let t0 = Instant::now();
    for n in 0u64.. {
        let deadline = t0 + Duration::from_secs_f64(n as f64 / f64::from(rate_hz));
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait.min(period));
        }
        match src.frame() {
            Ok(mm) => {
                counters.frames.fetch_add(1, Ordering::Relaxed);
                let frame = Frame {
                    t_ms: unix_ms(),
                    side: src.side(),
                    mm,
                };
                #[cfg(feature = "spatial-evidence")]
                if let Some(e) = spatial.as_mut() {
                    emit_spatial(e, &frame, &shared);
                }
                if let Ok(mut ring) = shared.lock() {
                    ring.push(frame);
                }
            }
            Err(e) => {
                if counters
                    .read_errors
                    .fetch_add(1, Ordering::Relaxed)
                    .is_multiple_of(50)
                {
                    eprintln!("{TAG} frame error: {e}");
                }
            }
        }
    }
}

/// One report over the last `window_s` of frames; returns the report and the store vector.
fn build_report(
    ring: &Ring,
    window_s: f64,
    o: &Opts,
    bg: &mut Background,
    counters: &Counters,
    export_url: &Option<String>,
) -> (serde_json::Value, Vec<f64>) {
    let frames = ring.since(window_s);
    let Some(latest) = frames.last() else {
        let r = serde_json::json!({ "status": "no_frames", "source": ring.source, "frames": 0,
            "read_errors": counters.read_errors.load(Ordering::Relaxed), "timestamp": unix_ms() / 1000 });
        return (r, vec![0.0; 8]);
    };
    for f in &frames {
        bg.update(&f.mm);
    }
    let side = latest.side;
    let span_s = frames.len().saturating_sub(1) as f64
        / ((latest.t_ms.saturating_sub(frames[0].t_ms)) as f64 / 1000.0).max(1e-9);
    let fps = if frames.len() > 1 { span_s } else { 0.0 };
    let near = scene::nearest(&latest.mm, side, o.max_mm);
    let valid = scene::valid_fraction(&latest.mm, o.max_mm);
    let motion: Vec<f64> = frames
        .windows(2)
        .filter_map(|w| scene::motion_mm(&w[0].mm, &w[1].mm, o.max_mm))
        .collect();
    let motion_mm = if motion.is_empty() {
        None
    } else {
        Some(motion.iter().sum::<f64>() / motion.len() as f64)
    };
    let occupied = bg.occupied(&latest.mm, o.presence_mm, o.max_mm);
    let presence = bg.learned() && occupied.len() >= 2;
    let sectors = scene::sectors(&latest.mm, side, o.max_mm);
    let owned: Vec<Vec<u16>> = frames.iter().map(|f| f.mm.clone()).collect();
    let noise = scene::zone_noise(&owned, o.max_mm);
    let noise_vals: Vec<f64> = noise.iter().flatten().copied().collect();
    let mean_noise = if noise_vals.is_empty() {
        None
    } else {
        Some(noise_vals.iter().sum::<f64>() / noise_vals.len() as f64)
    };
    let status = if valid == 0.0 { "no_targets" } else { "ok" };
    let round1 = |v: Option<f64>| {
        v.map_or(serde_json::Value::Null, |x| {
            serde_json::json!((x * 10.0).round() / 10.0)
        })
    };
    let mut report = serde_json::json!({
        "cog": "sen0628-tof",
        "status": status,
        "source": ring.source,
        "mode": format!("{side}x{side}"),
        "frames": frames.len(),
        "frame_rate_hz": (fps * 10.0).round() / 10.0,
        "read_errors": counters.read_errors.load(Ordering::Relaxed),
        "valid_pct": (valid * 1000.0).round() / 10.0,
        "nearest": near,
        "median_mm": round1(scene::median_mm(&latest.mm, o.max_mm)),
        "motion_mm": round1(motion_mm),
        "background_learned": bg.learned(),
        "presence": presence,
        "occupied_zones": occupied,
        "sectors": sectors,
        "mean_zone_noise_mm": round1(mean_noise),
        "zone_noise_mm": noise.iter().map(|n| n.map_or(serde_json::Value::Null, |x| serde_json::json!((x * 10.0).round() / 10.0))).collect::<Vec<_>>(),
        "background_mm": bg.background,
        "frame": { "t_ms": latest.t_ms, "side": side, "mm": latest.mm },
        "export": export_url,
        "timestamp": unix_ms() / 1000,
    });
    let max = f64::from(o.max_mm);
    let vector = vec![
        near.as_ref().map_or(1.0, |n| f64::from(n.mm) / max),
        valid,
        if presence { 1.0 } else { 0.0 },
        occupied.len() as f64 / (side * side) as f64,
        motion_mm.map_or(0.0, |m| m / 500.0),
        sectors.left.map_or(1.0, |d| f64::from(d) / max),
        sectors.middle.map_or(1.0, |d| f64::from(d) / max),
        sectors.right.map_or(1.0, |d| f64::from(d) / max),
    ]
    .into_iter()
    .map(|v| v.clamp(0.0, 1.0))
    .collect::<Vec<f64>>();
    report["vector"] = serde_json::json!(vector.clone());
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
    // The agent keeps the socket open after replying (cogs repo ADR-158 finding): stop at
    // Content-Length instead of reading to EOF.
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

fn open_source(o: &Opts) -> Result<Box<dyn FrameSource>, String> {
    if o.simulate {
        return Ok(Box::new(Simulator::new(o.side, f64::from(o.rate_hz))));
    }
    Sen0628::open(o.bus, o.addr, o.side).map(|s| Box::new(s) as Box<dyn FrameSource>)
}

fn no_source(err: &str) -> serde_json::Value {
    let r =
        serde_json::json!({ "status": "no_source", "error": err, "timestamp": unix_ms() / 1000 });
    println!("{r}");
    eprintln!("{TAG} no source: {err}");
    r
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
        "{TAG} start (once={}, interval={}s, window={}s, mode={}x{}, rate={}Hz, i2c-{} 0x{:02x}, simulate={})",
        o.once, o.interval, o.window, o.side, o.side, o.rate_hz, o.bus, o.addr, o.simulate
    );

    // The export comes up first so a hook-up tool can watch no_source turn into frames.
    let shared: Shared = Arc::new(Mutex::new(Ring {
        source: "none".into(),
        ..Default::default()
    }));
    let export_url = match export::serve(&o.api_bind, shared.clone()) {
        Ok(addr) => Some(format!("http://{addr}/frame")),
        Err(e) => {
            eprintln!("{TAG} export disabled: {e}");
            None
        }
    };
    let src = loop {
        match open_source(&o) {
            Ok(s) => break s,
            Err(e) => {
                let r = no_source(&e);
                if let Ok(mut ring) = shared.lock() {
                    ring.latest_report = Some(r);
                }
                if o.once {
                    return;
                }
                std::thread::sleep(Duration::from_secs(o.interval.clamp(2, 5)));
            }
        }
    };
    if let Ok(mut ring) = shared.lock() {
        ring.source = src.describe();
    }
    let counters = Arc::new(Counters::default());
    #[cfg(feature = "spatial-evidence")]
    let (emitter, spatial_refusal, spatial_status) = match spatial_settings {
        None => (None, None, None),
        Some(s) => {
            let status = s.refusal_status();
            match spatial::Emitter::new(s, o.simulate, o.max_mm) {
                Ok(e) => (Some(e), None, Some("emitting")),
                Err(why) => {
                    eprintln!("{TAG} not emitting spatial.evidence.v1: {why}");
                    (None, Some(why), status)
                }
            }
        }
    };
    {
        let (shared, counters, rate) = (shared.clone(), counters.clone(), o.rate_hz);
        std::thread::spawn(move || {
            frame_loop(
                src,
                rate,
                shared,
                counters,
                #[cfg(feature = "spatial-evidence")]
                emitter,
            )
        });
    }

    let mut bg = Background::new((o.learn_seconds * u64::from(o.rate_hz)) as usize);
    let period = if o.once { o.window } else { o.interval };
    loop {
        std::thread::sleep(Duration::from_secs(period));
        let (report, vector) = match shared.lock() {
            Ok(mut ring) => {
                let out = build_report(&ring, period as f64, &o, &mut bg, &counters, &export_url);
                #[cfg(feature = "spatial-evidence")]
                let out = {
                    let mut out = out;
                    let mut status = spatial_status;
                    let mut reasons = Vec::new();
                    if let Some(why) = &spatial_refusal {
                        reasons.push(format!("spatial_evidence_refused: {why}"));
                    }
                    if let Some(e) = ring.spatial_error.take() {
                        reasons.push(format!("spatial_evidence_error: {e}"));
                        status = Some("write_error");
                    }
                    out.0["spatial"] = serde_json::json!(status);
                    if !reasons.is_empty() {
                        out.0["reasons"] = serde_json::json!(reasons);
                    }
                    out
                };
                ring.latest_report = Some(out.0.clone());
                out
            }
            Err(_) => return,
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
    fn options_are_bounded_and_defaulted() {
        let o = parse_opts(&s(&[
            "--once",
            "--mode",
            "4",
            "--i2c-addr",
            "0x31",
            "--rate-hz",
            "99",
        ]));
        assert!(o.once);
        assert_eq!(o.side, 4);
        assert_eq!(o.addr, 0x31);
        assert_eq!(o.rate_hz, 10);
        assert_eq!(parse_opts(&s(&["--i2c-addr", "50"])).addr, 0x32);
        assert_eq!(parse_opts(&s(&["--i2c-addr", "0x48"])).addr, 0x33);
        assert_eq!(parse_opts(&s(&[])).api_bind, "0.0.0.0:8047");
        assert_eq!(parse_opts(&s(&[])).side, 8);
    }

    #[test]
    fn simulated_run_reports_presence_and_a_near_target() {
        let o = parse_opts(&s(&["--simulate"]));
        let mut ring = Ring {
            source: "sim".into(),
            ..Default::default()
        };
        let mut sim = Simulator::new(8, 10.0);
        // Learn an empty room (the wall) first, then let the person walk in.
        let mut bg = Background::new(20);
        let empty: Vec<u16> = (0..64)
            .map(|i| 2400 + 600 * (7 - (i / 8) as u16) / 7)
            .collect();
        for _ in 0..20 {
            bg.update(&empty);
        }
        for i in 0..30 {
            ring.push(Frame {
                t_ms: 1_000 + i * 100,
                side: 8,
                mm: sim.frame().unwrap(),
            });
        }
        let (r, v) = build_report(&ring, 3.0, &o, &mut bg, &Counters::default(), &None);
        assert_eq!(r["status"], "ok");
        assert_eq!(r["presence"], true, "{r}");
        assert!(r["nearest"]["mm"].as_u64().unwrap() < 1300);
        assert!(r["frame_rate_hz"].as_f64().unwrap() > 9.0);
        assert_eq!(v.len(), 8);
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
        assert!(serde_json::to_string(&r).unwrap().len() < 65_536);
    }

    #[test]
    fn ingest_response_is_complete_at_content_length() {
        assert!(!response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}"
        ));
        assert!(response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"
        ));
    }
}
