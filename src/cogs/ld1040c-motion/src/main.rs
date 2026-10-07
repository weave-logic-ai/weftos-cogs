//! cog-ld1040c-motion: Hi-Link HLK-LD1040C 10.525 GHz Doppler motion radar.
//!
//! Usage:
//!   cog-ld1040c-motion [--once] [--interval SECS] [--gpio BCM] [--uart /dev/serial0]
//!                      [--baud 9600] [--api-bind 127.0.0.1:8053] [--simulate]
//!
//! The reliable signal is the module's OUT pin, read as a GPIO input line on /dev/gpiochip0. Each
//! `--interval` window prints one JSON line to stdout (diagnostics go to stderr) and POSTs an
//! 8-float vector to the Seed store. `--once` reads one window and exits 0. A missing OUT line
//! yields a `status:"no_source"` line with null values, never a fabricated reading. The first ~9 s
//! after start are a warm-up and are flagged. UART telemetry is optional and off by default.

mod export;
mod guide;
mod source;
mod uart;

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use export::{edges_and_fraction, round, serve, Shared, State};
use source::{read_loop, unix_ms, GpioLine, OutLine, Simulator};

const TAG: &str = "[cog-ld1040c-motion]";
/// Per-cog id in the store vector tuple (21=ecg, 22=tof, 23=sound, 24=rd-03e, 25=as201, 26=mentra).
const STORE_ID: u32 = 27;
/// The GPIO character device the OUT pin's BCM line lives on.
const GPIO_CHIP: &str = "/dev/gpiochip0";
/// OUT-line sampling period: 100 Hz is ample for a signal that holds for seconds.
const SAMPLE_MS: u64 = 10;
/// Module warm-up after power-on; readings are flagged invalid until this elapses.
const WARMUP_MS: u64 = 9_000;
const DEFAULT_INTERVAL: u64 = 1;
const DEFAULT_GPIO: u32 = 17;
const DEFAULT_BAUD: u32 = 9_600;
const DEFAULT_API_BIND: &str = "127.0.0.1:8053";

// Store-vector normalisation constants (see guide/api.md). Initial estimates — tune to observed
// ranges. The 5 s hold + 2 s block caps motion onsets near ~8/min in practice; 30 leaves headroom.
const MAX_EVENTS_PER_MIN: f64 = 30.0;
const QUIET_NORM_S: f64 = 60.0;
const AMP_NORM: f64 = 255.0; // mid_freq_ad is one byte
const SIGNAL_NORM: f64 = 1024.0; // signal_sum2 / 64, heuristic ceiling

#[derive(Debug, PartialEq)]
struct Opts {
    once: bool,
    interval: u64,
    gpio: u32,
    uart: Option<String>,
    baud: u32,
    api_bind: String,
    simulate: bool,
}

fn parse_args(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        once: false,
        interval: DEFAULT_INTERVAL,
        gpio: DEFAULT_GPIO,
        uart: None,
        baud: DEFAULT_BAUD,
        api_bind: DEFAULT_API_BIND.into(),
        simulate: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--once" => o.once = true,
            "--simulate" => o.simulate = true,
            "--interval" => {
                let v = value()?;
                o.interval = v
                    .parse()
                    .ok()
                    .filter(|n| (1..=3600).contains(n))
                    .ok_or(format!("--interval must be an integer 1..=3600, got {v:?}"))?;
            }
            "--gpio" => {
                let v = value()?;
                o.gpio = v
                    .parse()
                    .ok()
                    .filter(|n| (0..=27).contains(n))
                    .ok_or(format!("--gpio must be a BCM number 0..=27, got {v:?}"))?;
            }
            "--baud" => {
                let v = value()?;
                o.baud = v
                    .parse()
                    .ok()
                    .filter(|n| (1200..=921_600).contains(n))
                    .ok_or(format!("--baud must be 1200..=921600, got {v:?}"))?;
            }
            "--uart" => {
                let v = value()?;
                if v.is_empty() {
                    o.uart = None;
                } else {
                    validate_device(&v)?;
                    o.uart = Some(v);
                }
            }
            "--api-bind" => {
                let v = value()?;
                v.parse::<SocketAddr>()
                    .map_err(|_| format!("--api-bind must be host:port, got {v:?}"))?;
                o.api_bind = v;
            }
            other => return Err(format!("unknown argument {other:?} (see --help)")),
        }
    }
    Ok(o)
}

/// Least authority: UART device paths must be under /dev/, no traversal, bounded length.
fn validate_device(path: &str) -> Result<(), String> {
    let ok = path.starts_with("/dev/")
        && path.len() <= 128
        && !path.split('/').any(|c| c == "..")
        && path.bytes().all(|b| b.is_ascii_graphic());
    if ok {
        Ok(())
    } else {
        Err(format!("--uart must be a path under /dev/, got {path:?}"))
    }
}

fn open_line(o: &Opts) -> Result<Box<dyn OutLine>, String> {
    if o.simulate {
        Ok(Box::new(Simulator::new()))
    } else {
        GpioLine::open(GPIO_CHIP, o.gpio).map(|l| Box::new(l) as Box<dyn OutLine>)
    }
}

/// Build the output line and the store vector from the current state.
fn build_report(
    st: &State,
    window_s: f64,
    simulate: bool,
    export_url: &Option<String>,
) -> (serde_json::Value, Vec<f64>) {
    let now = unix_ms();
    let tail = st.tail(window_s);
    let (edges, active_fraction) = edges_and_fraction(&tail);
    let present = st.last_level;
    let warming = !simulate && now < st.start_ms.saturating_add(WARMUP_MS);
    let seconds_since_motion = st
        .last_event_ms
        .map(|e| (now.saturating_sub(e)) as f64 / 1000.0);
    let tele = st.telemetry;
    let amplitude = tele.and_then(|t| t.motion_amplitude);
    let signal = tele.and_then(|t| t.signal);
    let noise = tele.and_then(|t| t.noise);

    let status = if st.samples_total == 0 {
        "no_source"
    } else if warming {
        "warming"
    } else if present {
        "present"
    } else {
        "clear"
    };

    let report = serde_json::json!({
        "status": status,
        "source": st.source,
        "simulated": simulate,
        "present": present,
        "warming": warming,
        "motion_events": edges,
        "active_fraction": round(active_fraction, 3),
        "seconds_since_motion": seconds_since_motion.map(|q| round(q, 1)),
        "detections_per_min": st.events_in(60, now),
        "total_detections": st.total_events,
        "samples": tail.len(),
        "window_s": window_s,
        "motion_amplitude": amplitude,
        "signal": signal.map(|s| round(s, 2)),
        "noise": noise.map(|n| round(n, 2)),
        "telemetry_ms": tele.map(|t| t.t_ms),
        "export": export_url,
        "timestamp": now / 1000,
    });

    // 8-float store vector (indices documented in guide/api.md), all clamped to 0.0..=1.0.
    let events_per_min = edges as f64 / window_s * 60.0;
    let vector: Vec<f64> = [
        if present { 1.0 } else { 0.0 },
        events_per_min / MAX_EVENTS_PER_MIN,
        active_fraction,
        seconds_since_motion
            .map(|q| q / QUIET_NORM_S)
            .unwrap_or(1.0),
        amplitude.map(|a| f64::from(a) / AMP_NORM).unwrap_or(0.0),
        signal.map(|s| s / SIGNAL_NORM).unwrap_or(0.0),
        if warming { 1.0 } else { 0.0 },
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    cog_sensor_sources::handle_help(
        &args,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        include_str!("../cog.toml"),
    );
    let o = match parse_args(&args[1..]) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{TAG} {e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "{TAG} start (once={}, interval={}s, gpio={}, uart={:?}, simulate={})",
        o.once, o.interval, o.gpio, o.uart, o.simulate
    );

    let shared: Shared = Arc::new(Mutex::new(State::new("none".into(), unix_ms())));

    // Bring the export up first (continuous only) so a hook-up tool can watch no_source turn into
    // motion while the wiring is finished.
    let export_url = if o.once {
        None
    } else {
        match serve(&o.api_bind, shared.clone()) {
            Ok(addr) => Some(format!("http://{addr}/status")),
            Err(e) => {
                eprintln!("{TAG} export disabled: {e}");
                None
            }
        }
    };

    // Open the OUT line, retrying in continuous mode; fail honest and exit on --once.
    let line = loop {
        match open_line(&o) {
            Ok(l) => break l,
            Err(e) => {
                let (r, _) = build_report(
                    &shared.lock().unwrap(),
                    o.interval as f64,
                    o.simulate,
                    &export_url,
                );
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
        st.source = line.describe();
    }

    std::thread::spawn({
        let shared = shared.clone();
        move || read_loop(line, shared, SAMPLE_MS)
    });
    if let Some(dev) = o.uart.clone() {
        let shared = shared.clone();
        let baud = o.baud;
        std::thread::spawn(move || uart::telemetry_loop(dev, baud, shared));
    }

    loop {
        std::thread::sleep(Duration::from_secs(o.interval));
        let (report, vector) = match shared.lock() {
            Ok(mut st) => {
                let out = build_report(&st, o.interval as f64, o.simulate, &export_url);
                st.latest_report = Some(out.0.clone());
                out
            }
            Err(_) => {
                eprintln!("{TAG} state poisoned");
                return;
            }
        };
        println!("{report}");
        if report["status"] != "no_source" {
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
    use export::Sample;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn defaults() {
        let o = parse_args(&[]).unwrap();
        assert_eq!(o.interval, 1);
        assert_eq!(o.gpio, 17);
        assert_eq!(o.baud, 9600);
        assert_eq!(o.api_bind, "127.0.0.1:8053");
        assert!(o.uart.is_none());
        assert!(!o.once && !o.simulate);
    }

    #[test]
    fn flags() {
        let o = parse_args(&args(
            "--once --interval 5 --gpio 22 --uart /dev/serial0 --baud 115200 --api-bind 0.0.0.0:8053 --simulate",
        ))
        .unwrap();
        assert!(o.once && o.simulate);
        assert_eq!((o.interval, o.gpio, o.baud), (5, 22, 115_200));
        assert_eq!(o.uart.as_deref(), Some("/dev/serial0"));
        assert_eq!(o.api_bind, "0.0.0.0:8053");
    }

    #[test]
    fn empty_uart_disables() {
        assert!(parse_args(&args("--uart")).is_err()); // missing value
        let o = parse_args(&["--uart".into(), "".into()]).unwrap();
        assert!(o.uart.is_none());
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
            let o = parse_args(&args(c)).unwrap_or_else(|e| panic!("{c}: {e}"));
            assert!(o.once, "{c}: console commands must be --once");
        }
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "--interval 0",
            "--interval 3601",
            "--gpio 28",
            "--gpio -1",
            "--baud 100",
            "--uart /etc/passwd",
            "--uart /dev/../etc/passwd",
            "--api-bind 8053",
            "--verbose",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad} should be rejected");
        }
    }

    /// Feed the real aggregation path from the simulator's OUT pattern and check the window summary
    /// and the bounded 8-float store vector.
    #[test]
    fn simulated_window_summary_and_vector_bounded() {
        let mut st = State::new("sim".into(), 0);
        // One window: 100 samples at 10 ms over 2 s of sim time, starting mid-walk.
        for i in 0..200u64 {
            let elapsed = 1.5 + i as f64 * 0.01; // 1.5 s .. 3.5 s: inside the walk, OUT high
            let high = Simulator::level_at(elapsed);
            let t = i * 10;
            if high && !st.last_level {
                st.mark_event(t);
            }
            st.push(Sample { t_ms: t, high });
            st.last_level = high;
            st.samples_total += 1;
        }
        let (report, vector) = build_report(&st, 2.0, true, &None);
        assert_eq!(report["status"], "present");
        assert_eq!(report["present"], true);
        assert_eq!(report["warming"], false); // simulate suppresses warm-up
        assert_eq!(report["motion_amplitude"], serde_json::Value::Null);
        assert_eq!(vector.len(), 8);
        assert!(vector.iter().all(|x| (0.0..=1.0).contains(x)));
        assert_eq!(vector[0], 1.0); // present
        assert_eq!(vector[6], 0.0); // not warming
    }

    #[test]
    fn no_samples_is_no_source() {
        let st = State::new("none".into(), 0);
        let (report, _) = build_report(&st, 1.0, false, &None);
        assert_eq!(report["status"], "no_source");
        assert_eq!(report["present"], false);
        assert_eq!(report["seconds_since_motion"], serde_json::Value::Null);
    }
}
