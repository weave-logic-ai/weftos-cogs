//! Cognitum Cog: SEN0213 ECG
//!
//! DFRobot SEN0213 (AD8232 single-lead ECG front end) -> ADS1115 16-bit I2C ADC -> Seed GPIO
//! header (I2C bus 1). Exports the raw and filtered waveform, detects R-peaks and reports heart
//! rate, RR intervals, SDNN, RMSSD and signal quality. ADR-158. Implements the ADR-001
//! cog-as-plugin contract. Not a medical device.
//!
//! Usage:
//!   cog-sen0213-ecg --once                 # capture --window seconds (default 10), report, exit
//!   cog-sen0213-ecg --interval 5           # run continuously, one report every 5 s
//!   cog-sen0213-ecg --once --simulate      # synthetic 72 bpm ECG, no hardware needed

mod ads1115;
mod dsp;
mod export;
mod guide;

use ads1115::{Ads1115, EcgSource, Simulator};
use dsp::{DisplayFilter, LeadState, QrsDetector};
use export::{Ring, Sample, Shared};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TAG: &str = "[cog-sen0213-ecg]";
/// Per-cog id used in the store vector tuple.
const STORE_ID: u32 = 21;
/// Cap on samples per array in a stdout report (keeps --once under the 64 KiB console limit).
const EMIT_CAP: usize = 3000;
/// HR uses the last 10 s of beats; HRV the whole 60 s ring.
const HR_SECONDS: f64 = 10.0;

struct Opts {
    once: bool,
    interval: u64,
    window: u64,
    fs: u32,
    bus: u8,
    addr: u8,
    channel: u8,
    mains_hz: u32,
    api_bind: String,
    emit_samples: bool,
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
        Some(a) if (0x48..=0x4B).contains(&a) => a,
        Some(_) => {
            eprintln!("{TAG} --i2c-addr must be 72-75 (0x48-0x4b), using 72");
            0x48
        }
        None => 0x48,
    }
}

fn parse_opts(args: &[String]) -> Opts {
    let api_bind = arg(args, "--api-bind")
        .unwrap_or("0.0.0.0:8046")
        .to_string();
    Opts {
        once: args.iter().any(|a| a == "--once"),
        interval: num(args, "--interval", 5, 1, 60),
        window: num(args, "--window", 10, 3, 12),
        fs: num(args, "--sample-rate", 250, 100, 500),
        bus: num(args, "--i2c-bus", 1, 0, 9),
        addr: parse_addr(args),
        channel: num(args, "--channel", 0, 0, 3),
        mains_hz: num(args, "--mains-hz", 60, 0, 60),
        api_bind,
        emit_samples: args.iter().any(|a| a == "--emit-samples"),
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
struct Timing {
    read_errors: AtomicU64,
    late: AtomicU64,
    max_late_us: AtomicU64,
}

/// Reads the source at a fixed rate against absolute deadlines (no drift), filters, detects
/// beats and pushes everything into the shared ring. Runs until the process exits.
fn sample_loop(
    mut src: Box<dyn EcgSource>,
    fs: u32,
    mains_hz: u32,
    shared: Shared,
    timing: Arc<Timing>,
) {
    let fsf = f64::from(fs);
    let period = Duration::from_secs_f64(1.0 / fsf);
    let mut display = DisplayFilter::new(fsf, f64::from(mains_hz));
    let mut qrs = QrsDetector::new(fsf);
    let (t0, t0_ms) = (Instant::now(), unix_ms());
    for idx in 0u64.. {
        let deadline = t0 + Duration::from_secs_f64(idx as f64 / fsf);
        let now = Instant::now();
        if now < deadline {
            std::thread::sleep(deadline - now);
        } else {
            let late = (now - deadline).as_micros() as u64;
            if late > period.as_micros() as u64 {
                timing.late.fetch_add(1, Ordering::Relaxed);
            }
            timing.max_late_us.fetch_max(late, Ordering::Relaxed);
        }
        let raw_v = match src.read_volts() {
            Ok(v) => v,
            Err(e) => {
                if timing
                    .read_errors
                    .fetch_add(1, Ordering::Relaxed)
                    .is_multiple_of(500)
                {
                    eprintln!("{TAG} read error: {e}");
                }
                continue;
            }
        };
        let t_ms = t0_ms + idx * 1000 / u64::from(fs);
        let filtered_mv = display.step(raw_v);
        let peak = qrs.step(raw_v);
        if let Ok(mut ring) = shared.lock() {
            ring.push(Sample {
                idx,
                t_ms,
                raw_v,
                filtered_mv,
            });
            if let Some(p) = peak {
                ring.push_peak(p, t0_ms + p * 1000 / u64::from(fs));
            }
        }
    }
}

fn stats(v: &[f64]) -> serde_json::Value {
    if v.is_empty() {
        return serde_json::Value::Null;
    }
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let std = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    let min = v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    serde_json::json!({ "min_v": export::round(min, 4), "max_v": export::round(max, 4), "mean_v": export::round(mean, 4), "std_v": export::round(std, 5) })
}

fn opt_round(v: Option<f64>, places: i32) -> serde_json::Value {
    v.map_or(serde_json::Value::Null, |x| {
        serde_json::json!(export::round(x, places))
    })
}

/// Builds one report from the last `window_s` of the ring (HRV from the whole ring).
fn build_report(
    ring: &Ring,
    window_s: f64,
    opts: &Opts,
    timing: &Timing,
    export_url: &Option<String>,
) -> (serde_json::Value, Vec<f64>) {
    let (win, win_peaks) = ring.tail(window_s);
    let raw: Vec<f64> = win.iter().map(|s| s.raw_v).collect();
    let lead = dsp::lead_state(&raw);
    let fs = ring.fs;
    let idx = |p: &[(u64, u64)]| p.iter().map(|&(i, _)| i).collect::<Vec<_>>();
    let (_, hr_peaks) = ring.tail(HR_SECONDS.max(window_s));
    let (_, all_peaks) = ring.tail(export::RING_SECONDS);
    let rr_hr = dsp::rr_intervals_ms(&idx(&hr_peaks), fs);
    let rr_all = dsp::rr_intervals_ms(&idx(&all_peaks), fs);
    let rr_win = dsp::rr_intervals_ms(&idx(&win_peaks), fs);
    let ok = lead == LeadState::Ok;
    let hr = if ok {
        dsp::heart_rate_bpm(&rr_hr)
    } else {
        None
    };
    let sdnn = if ok { dsp::sdnn_ms(&rr_all) } else { None };
    let rmssd = if ok { dsp::rmssd_ms(&rr_all) } else { None };
    let quality = dsp::quality(lead, &rr_hr);
    let status = match lead {
        LeadState::Ok if hr.is_some() => "ok",
        LeadState::Ok => "no_beats",
        LeadState::Saturated => "leads_off",
        LeadState::Flat => "flat",
        LeadState::Unknown => "no_samples",
    };
    let mean_rr = dsp::median(&rr_hr);
    let mut report = serde_json::json!({
        "cog": "sen0213-ecg",
        "status": status,
        "source": ring.source,
        "sample_rate_hz": fs,
        "window_s": window_s,
        "samples": win.len(),
        "read_errors": timing.read_errors.load(Ordering::Relaxed),
        "late_samples": timing.late.load(Ordering::Relaxed),
        "max_late_ms": export::round(timing.max_late_us.load(Ordering::Relaxed) as f64 / 1000.0, 2),
        "lead_state": lead,
        "raw": stats(&raw),
        "heart_rate_bpm": opt_round(hr, 1),
        "beats": win_peaks.len(),
        "r_peaks_ms": win_peaks.iter().map(|&(_, t)| t).collect::<Vec<_>>(),
        "rr_ms": rr_win.iter().map(|r| export::round(*r, 1)).collect::<Vec<_>>(),
        "sdnn_ms": opt_round(sdnn, 1),
        "rmssd_ms": opt_round(rmssd, 1),
        "hrv_window_s": export::RING_SECONDS.min(ring.samples.len() as f64 / fs),
        "quality": export::round(quality, 3),
        "export": export_url,
        "medical": false,
        "timestamp": unix_ms() / 1000,
    });
    if opts.emit_samples {
        let skip = win.len().saturating_sub(EMIT_CAP);
        report["samples_raw_v"] = win
            .iter()
            .skip(skip)
            .map(|s| export::round(s.raw_v, 4))
            .collect();
        report["samples_filtered_mv"] = win
            .iter()
            .skip(skip)
            .map(|s| export::round(s.filtered_mv, 2))
            .collect();
        report["samples_truncated"] = serde_json::json!(skip > 0);
    }
    let vector = vec![
        hr.map_or(0.0, |h| h / 200.0),
        quality,
        if ok { 1.0 } else { 0.0 },
        sdnn.map_or(0.0, |v| v / 200.0),
        rmssd.map_or(0.0, |v| v / 200.0),
        win_peaks.len() as f64 / 20.0,
        mean_rr.map_or(0.0, |v| v / 2000.0),
        raw.iter().copied().fold(0.0_f64, f64::max) / 3.3,
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
    // The Seed agent answers with HTTP/1.1 and keeps the socket open even for an HTTP/1.0
    // request, so reading to EOF stalls for the whole read timeout (about 5 s per cycle).
    // Stop as soon as the headers and Content-Length bytes have arrived.
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

/// True once `resp` holds the full header block and Content-Length bytes of body.
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

fn open_source(o: &Opts) -> Result<Box<dyn EcgSource>, String> {
    if o.simulate {
        return Ok(Box::new(Simulator::new(
            f64::from(o.fs),
            72.0,
            f64::from(o.mains_hz),
        )));
    }
    Ads1115::open(o.bus, o.addr, o.channel).map(|a| Box::new(a) as Box<dyn EcgSource>)
}

fn no_source(err: &str) -> serde_json::Value {
    let r = serde_json::json!({ "status": "no_source", "error": err, "medical": false, "timestamp": unix_ms() / 1000 });
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
    let opts = parse_opts(&args);
    eprintln!(
        "{TAG} start (once={}, interval={}s, window={}s, fs={}Hz, i2c-{} 0x{:02x} A{}, mains={}Hz, simulate={})",
        opts.once, opts.interval, opts.window, opts.fs, opts.bus, opts.addr, opts.channel, opts.mains_hz, opts.simulate
    );

    // The export comes up first, so a hook-up tool can watch "no_source" turn into a signal
    // while the ADC is being wired. In --once mode a missing ADC is reported and we exit.
    let shared: Shared = Arc::new(Mutex::new(Ring::new(f64::from(opts.fs), "none".into())));
    let export_url = if opts.once && !opts.simulate && open_source(&opts).is_err() {
        None
    } else {
        match export::serve(&opts.api_bind, shared.clone()) {
            Ok(addr) => Some(format!("http://{addr}/raw")),
            Err(e) => {
                eprintln!("{TAG} export disabled: {e}");
                None
            }
        }
    };

    // The ADC may be plugged in after start: in continuous mode keep retrying.
    let src = loop {
        match open_source(&opts) {
            Ok(s) => break s,
            Err(e) => {
                let r = no_source(&e);
                if let Ok(mut ring) = shared.lock() {
                    ring.latest_report = Some(r);
                }
                if opts.once {
                    return;
                }
                std::thread::sleep(Duration::from_secs(opts.interval.clamp(1, 5)));
            }
        }
    };
    if let Ok(mut ring) = shared.lock() {
        ring.source = src.describe();
    }

    let timing = Arc::new(Timing::default());
    {
        let (shared, timing, fs, mains) = (shared.clone(), timing.clone(), opts.fs, opts.mains_hz);
        std::thread::spawn(move || sample_loop(src, fs, mains, shared, timing));
    }

    let period = if opts.once {
        opts.window
    } else {
        opts.interval
    };
    loop {
        std::thread::sleep(Duration::from_secs(period));
        let (report, vector) = match shared.lock() {
            Ok(mut ring) => {
                let out = build_report(&ring, period as f64, &opts, &timing, &export_url);
                ring.latest_report = Some(out.0.clone());
                out
            }
            Err(_) => {
                eprintln!("{TAG} state poisoned");
                return;
            }
        };
        println!("{report}");
        if report["status"] != "no_samples" {
            if let Err(e) = store_to_seed(&vector) {
                eprintln!("{TAG} store error: {e}");
            }
        }
        if opts.once {
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
    fn ingest_response_is_complete_at_content_length_not_eof() {
        assert!(!response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n"
        ));
        assert!(!response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}"
        ));
        assert!(response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{\"a\"}"
        ));
        assert!(response_complete(b"HTTP/1.1 204 No Content\r\n\r\n"));
    }

    #[test]
    fn options_are_bounded_and_defaulted() {
        let o = parse_opts(&s(&[
            "--once",
            "--sample-rate",
            "9999",
            "--window",
            "5",
            "--i2c-addr",
            "0x49",
        ]));
        assert!(o.once);
        assert_eq!(o.fs, 250);
        assert_eq!(o.window, 5);
        assert_eq!(o.addr, 0x49);
        assert_eq!(parse_opts(&s(&["--i2c-addr", "74"])).addr, 0x4A);
        assert_eq!(parse_opts(&s(&["--i2c-addr", "0x20"])).addr, 0x48);
        assert_eq!(parse_opts(&s(&[])).api_bind, "0.0.0.0:8046");
    }

    #[test]
    fn simulated_run_reports_heart_rate_and_export_fields() {
        let opts = parse_opts(&s(&["--simulate", "--emit-samples"]));
        let mut ring = Ring::new(250.0, "sim".into());
        let mut src = Simulator::new(250.0, 72.0, 60.0);
        let mut disp = DisplayFilter::new(250.0, 60.0);
        let mut qrs = QrsDetector::new(250.0);
        for idx in 0..(250 * 12) {
            let raw_v = src.read_volts().unwrap();
            let filtered_mv = disp.step(raw_v);
            ring.push(Sample {
                idx,
                t_ms: idx * 4,
                raw_v,
                filtered_mv,
            });
            if let Some(p) = qrs.step(raw_v) {
                ring.push_peak(p, p * 4);
            }
        }
        let (r, v) = build_report(&ring, 10.0, &opts, &Timing::default(), &None);
        assert_eq!(r["status"], "ok");
        let hr = r["heart_rate_bpm"].as_f64().unwrap();
        assert!((hr - 72.0).abs() < 2.0, "hr {hr}");
        assert_eq!(r["samples_raw_v"].as_array().unwrap().len(), 2500);
        assert_eq!(r["medical"], false);
        assert_eq!(v.len(), 8);
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
        assert!(serde_json::to_string(&r).unwrap().len() < 65_536);
    }
}
