//! Cognitum Cog: sound-detect
//!
//! A KY-038 / LM393 sound-detection module (electret mic + comparator + sensitivity trimpot) whose
//! OUT wire is read through an ADS1115 I2C ADC on the Seed's GPIO header (OUT -> A1 by default, so
//! it coexists with the sen0213-ecg cog on A0). Reports whether sound is present, the per-minute
//! event rate, the quiet time since the last event, the activity level and peak. Works for the
//! common active-high digital OUT and, degraded, for analog-OUT variants. Implements the ADR-001
//! cog-as-plugin contract.
//!
//! Usage:
//!   cog-sound-detect --once               # one <window>-second capture, report, exit
//!   cog-sound-detect --interval 5         # continuous, one report every 5 s
//!   cog-sound-detect --once --simulate    # synthetic bursts, no hardware needed

mod adc;
mod export;
mod guide;

use adc::{AdcSource, Ads1115, SoundSim};
use export::{round, Sample, Shared, State};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TAG: &str = "[cog-sound-detect]";
/// Per-cog id in the store vector tuple (21=ecg, 22=tof, 23=sound).
const STORE_ID: u32 = 23;
/// Ignore a second rising edge within this of the last (debounce one event per burst).
const DEBOUNCE_MS: u64 = 120;
/// Decimate the display ring to about this rate regardless of the sample rate.
const RING_HZ: f64 = 100.0;

struct Opts {
    once: bool,
    interval: u64,
    window: u64,
    fs: u32,
    threshold_v: f64,
    bus: u8,
    addr: u8,
    channel: u8,
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

fn parse_addr(a: &[String]) -> u8 {
    let v = arg(a, "--i2c-addr").and_then(|s| match s.strip_prefix("0x") {
        Some(h) => u8::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    });
    match v {
        Some(x) if (0x48..=0x4B).contains(&x) => x,
        Some(_) => {
            eprintln!("{TAG} --i2c-addr must be 72-75 (0x48-0x4b), using 72");
            0x48
        }
        None => 0x48,
    }
}

fn parse_opts(a: &[String]) -> Opts {
    Opts {
        once: a.iter().any(|x| x == "--once"),
        interval: num(a, "--interval", 5, 1, 60),
        window: num(a, "--window", 5, 1, 30),
        fs: num(a, "--sample-rate", 1000, 100, 2000),
        threshold_v: num(a, "--threshold", 1.5, 0.05, 3.3),
        bus: num(a, "--i2c-bus", 1, 0, 9),
        addr: parse_addr(a),
        channel: num(a, "--channel", 1, 0, 3),
        api_bind: arg(a, "--api-bind").unwrap_or("0.0.0.0:8049").to_string(),
        simulate: a.iter().any(|x| x == "--simulate"),
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Reads the ADC at `fs`, detects rising threshold crossings (one event per burst, debounced), and
/// pushes a decimated trace into the shared ring. Runs until the process exits.
fn sample_loop(mut src: Box<dyn AdcSource>, fs: u32, threshold_v: f64, shared: Shared) {
    let fsf = f64::from(fs);
    let (t0, t0_ms) = (Instant::now(), unix_ms());
    let decim = (fsf / RING_HZ).max(1.0) as u64;
    let mut above = false;
    let mut errors = 0u64;
    for idx in 0u64.. {
        let deadline = t0 + Duration::from_secs_f64(idx as f64 / fsf);
        let now = Instant::now();
        if now < deadline {
            std::thread::sleep(deadline - now);
        }
        let v = match src.read_volts() {
            Ok(v) => v,
            Err(e) => {
                errors += 1;
                if errors.is_multiple_of(500) {
                    eprintln!("{TAG} read error: {e}");
                }
                continue;
            }
        };
        let t_ms = t0_ms + idx * 1000 / u64::from(fs);
        let now_above = v >= threshold_v;
        if now_above && !above {
            // rising edge
            if let Ok(mut st) = shared.lock() {
                let debounced = st
                    .last_event_ms
                    .is_some_and(|last| t_ms.saturating_sub(last) < DEBOUNCE_MS);
                if !debounced {
                    st.mark_event(t_ms);
                }
            }
        }
        above = now_above;
        if idx.is_multiple_of(decim) {
            if let Ok(mut st) = shared.lock() {
                st.push(Sample { t_ms, v });
            }
        }
    }
}

fn build_report(
    st: &State,
    window_s: f64,
    threshold_v: f64,
    export_url: &Option<String>,
) -> (serde_json::Value, Vec<f64>) {
    let now_ms = unix_ms();
    let win = st.tail(window_s);
    let n = win.len().max(1) as f64;
    let above = win.iter().filter(|s| s.v >= threshold_v).count();
    let activity = above as f64 / n;
    let peak_v = win.iter().map(|s| s.v).fold(0.0_f64, f64::max);
    let level_v = win.iter().map(|s| s.v).sum::<f64>() / n;
    let events_60s = st.events_in(60, now_ms);
    let events_win = st.events_in(window_s.ceil() as u64, now_ms);
    let events_per_min = events_60s as f64;
    let quiet_s = st
        .last_event_ms
        .map(|e| (now_ms.saturating_sub(e)) as f64 / 1000.0)
        .unwrap_or(f64::INFINITY);
    let present = events_win > 0 || activity > 0.0;
    let status = if win.is_empty() {
        "no_samples"
    } else if present {
        "sound"
    } else {
        "quiet"
    };

    let report = serde_json::json!({
        "status": status,
        "source": st.source,
        "present": present,
        "events_per_min": round(events_per_min, 1),
        "events_in_window": events_win,
        "total_events": st.total_events,
        "quiet_s": if quiet_s.is_finite() { serde_json::json!(round(quiet_s, 1)) } else { serde_json::Value::Null },
        "activity_pct": round(activity * 100.0, 1),
        "level_v": round(level_v, 4),
        "peak_v": round(peak_v, 4),
        "threshold_v": round(threshold_v, 3),
        "window_s": window_s,
        "samples": win.len(),
        "export": export_url,
        "timestamp": now_ms / 1000,
    });

    let vector: Vec<f64> = [
        if present { 1.0 } else { 0.0 },
        activity,
        events_per_min / 60.0,
        peak_v / 3.3,
        if quiet_s.is_finite() {
            quiet_s / 60.0
        } else {
            1.0
        },
        level_v / 3.3,
        threshold_v / 3.3,
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
    // The agent answers HTTP/1.1 and keeps the socket open; stop at Content-Length, not EOF.
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

fn open_source(o: &Opts) -> Result<Box<dyn AdcSource>, String> {
    if o.simulate {
        return Ok(Box::new(SoundSim::new(f64::from(o.fs))));
    }
    Ads1115::open(o.bus, o.addr, o.channel).map(|a| Box::new(a) as Box<dyn AdcSource>)
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
    eprintln!(
        "{TAG} start (once={}, interval={}s, window={}s, fs={}Hz, i2c-{} 0x{:02x} A{}, threshold={}V, simulate={})",
        o.once, o.interval, o.window, o.fs, o.bus, o.addr, o.channel, o.threshold_v, o.simulate
    );

    let shared: Shared = Arc::new(Mutex::new(State::new(f64::from(o.fs), "none".into())));
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
    {
        let (shared, fs, th) = (shared.clone(), o.fs, o.threshold_v);
        std::thread::spawn(move || sample_loop(src, fs, th, shared));
    }

    let period = if o.once { o.window } else { o.interval };
    loop {
        std::thread::sleep(Duration::from_secs(period));
        let (report, vector) = match shared.lock() {
            Ok(mut st) => {
                let out = build_report(&st, period as f64, o.threshold_v, &export_url);
                st.latest_report = Some(out.0.clone());
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
        let o = parse_opts(&s(&[
            "--once",
            "--sample-rate",
            "99999",
            "--threshold",
            "1.0",
            "--channel",
            "2",
        ]));
        assert!(o.once);
        assert_eq!(o.fs, 1000);
        assert_eq!(o.threshold_v, 1.0);
        assert_eq!(o.channel, 2);
        assert_eq!(parse_opts(&s(&[])).api_bind, "0.0.0.0:8049");
        assert_eq!(parse_opts(&s(&[])).channel, 1);
    }

    #[test]
    fn response_complete_stops_at_content_length() {
        assert!(!response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n"
        ));
        assert!(response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"
        ));
    }

    #[test]
    fn report_from_simulated_bursts_detects_sound_and_vector_is_bounded() {
        let mut st = State::new(1000.0, "sim".into());
        let mut src = SoundSim::new(1000.0);
        // Feed ~3 s at 1 kHz through the same detection logic the loop uses.
        let mut above = false;
        for idx in 0..3000u64 {
            let v = src.read_volts().unwrap();
            let t_ms = idx; // 1 kHz -> 1 ms/sample
            let na = v >= 1.5;
            if na && !above {
                let deb = st
                    .last_event_ms
                    .is_some_and(|l| t_ms.saturating_sub(l) < DEBOUNCE_MS);
                if !deb {
                    st.mark_event(t_ms);
                }
            }
            above = na;
            if idx.is_multiple_of(10) {
                st.push(Sample { t_ms, v });
            }
        }
        let (r, v) = build_report(&st, 3.0, 1.5, &None);
        assert_eq!(r["status"], "sound");
        assert_eq!(r["present"], true);
        assert!(
            st.total_events >= 2,
            "expected bursts, got {}",
            st.total_events
        );
        assert_eq!(v.len(), 8);
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
        assert!(serde_json::to_string(&r).unwrap().len() < 65_536);
    }
}
