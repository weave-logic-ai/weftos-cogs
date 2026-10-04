//! Cognitum Cog: mentra-live
//!
//! A companion-side bridge that lets a pair of Mentra Live smart glasses join the mesh as a node.
//! The glasses run Android (the MentraOS ASG client) and cannot host a cog, so this cog runs on a
//! companion that can reach them over ADB — USB, or ADB-over-TCP (`adb connect ip:port`) — and
//! reports their liveness. Each interval it reads:
//!
//!   * presence  — is the device reachable (`adb get-state` == "device")
//!   * battery   — level, charging state, plug type, pack temperature and voltage (`dumpsys battery`)
//!   * link      — ADB round-trip time
//!
//! and emits them two ways: a store vector to the Seed (127.0.0.1:80), and a fleet heartbeat to the
//! weft-cog-host so the glasses show up as an online node. Telemetry-first by design; HUD, sensors
//! and audio are built on top of this node once it is joined. Implements the ADR-001 cog contract.
//!
//! Usage:
//!   cog-mentra-live --once                       # one telemetry snapshot, emit it, exit
//!   cog-mentra-live --interval 5                 # continuous, a snapshot + heartbeat every 5 s
//!   cog-mentra-live --once --simulate            # synthetic glasses, no hardware needed
//!   cog-mentra-live --addr 192.168.1.50:5555     # ADB-over-Wi-Fi
//!   cog-mentra-live --serial ABCD1234            # a specific USB device
//!   cog-mentra-live --host http://203.0.113.10:9480   # where to post the fleet heartbeat

mod adb;
mod export;
mod guide;
mod heartbeat;

use adb::{AdbSource, Sim, TeleSource, Telemetry};
use export::{round, Sample, Shared, State};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TAG: &str = "[cog-mentra-live]";
/// Per-cog id in the store vector tuple (21=ecg, 22=tof, 23=sound, 24=rd-03e, 25=hlk-as201, 26=mentra).
const STORE_ID: u32 = 26;

struct Opts {
    once: bool,
    interval: u64,
    serial: Option<String>,
    addr: Option<String>,
    adb: Option<String>,
    node_id: String,
    host: String,
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

fn opt_str(a: &[String], f: &str) -> Option<String> {
    arg(a, f).map(str::to_string).filter(|s| !s.is_empty())
}

fn parse_opts(a: &[String]) -> Opts {
    let host = opt_str(a, "--host")
        .or_else(|| std::env::var("WEFTOS_HOST").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "http://127.0.0.1:9480".to_string());
    Opts {
        once: a.iter().any(|x| x == "--once"),
        interval: num(a, "--interval", 5, 1, 120),
        serial: opt_str(a, "--serial"),
        addr: opt_str(a, "--addr"),
        adb: opt_str(a, "--adb"),
        node_id: opt_str(a, "--node-id").unwrap_or_else(|| "mentra-01".to_string()),
        host,
        api_bind: arg(a, "--api-bind").unwrap_or("0.0.0.0:8061").to_string(),
        simulate: a.iter().any(|x| x == "--simulate"),
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Map a telemetry snapshot to the JSON report and the 8-float store vector.
///
/// vector: [online, battery/100, charging, on_power, rtt/2000, temp/60, voltage/5, reserved].
fn build_report(
    t: &Telemetry,
    node_id: &str,
    joined: bool,
    export_url: &Option<String>,
) -> (serde_json::Value, Vec<f64>) {
    let status = if t.online { "online" } else { "offline" };
    let report = serde_json::json!({
        "status": status,
        "node_id": node_id,
        "joined": joined,
        "online": t.online,
        "serial": t.serial,
        "model": t.model,
        "fw": t.fw,
        "ip": t.ip,
        "battery_pct": t.battery_pct,
        "charging": t.charging,
        "plugged": t.plugged.as_str(),
        "battery_temp_c": t.temp_c.map(|x| round(x, 1)),
        "battery_voltage_v": t.voltage_v.map(|x| round(x, 3)),
        "link_rtt_ms": t.rtt_ms.map(|x| round(x, 1)),
        "export": export_url,
        "timestamp": unix_ms() / 1000,
    });
    let vector: Vec<f64> = [
        if t.online { 1.0 } else { 0.0 },
        t.battery_pct.map(|b| f64::from(b) / 100.0).unwrap_or(0.0),
        if t.charging { 1.0 } else { 0.0 },
        t.plugged.on_power(),
        t.rtt_ms.map(|r| r / 2000.0).unwrap_or(0.0),
        t.temp_c.map(|c| c / 60.0).unwrap_or(0.0),
        t.voltage_v.map(|v| v / 5.0).unwrap_or(0.0),
        0.0,
    ]
    .into_iter()
    .map(|v| v.clamp(0.0, 1.0))
    .collect();
    (report, vector)
}

/// POST the store vector to the Seed runtime, exactly like the other cogs.
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

fn open_source(o: &Opts) -> Result<Box<dyn TeleSource + Send>, String> {
    if o.simulate {
        return Ok(Box::new(Sim::new()));
    }
    Ok(Box::new(AdbSource::connect(&o.adb, &o.serial, &o.addr)?))
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
        "{TAG} start (once={}, interval={}s, node={}, host={}, simulate={})",
        o.once, o.interval, o.node_id, o.host, o.simulate
    );

    let shared: Shared = Arc::new(Mutex::new(State::new("none".into())));

    // Bring up the ADB source. On --once with no device, fail honestly and exit without the server.
    let mut src = match open_source(&o) {
        Ok(s) => s,
        Err(e) => {
            let r = serde_json::json!({"status":"no_source","node_id":o.node_id,"error":e,"timestamp":unix_ms()/1000});
            println!("{r}");
            eprintln!("{TAG} no source: {e}");
            if o.once {
                return;
            }
            // continuous: keep retrying the connect below
            loop {
                std::thread::sleep(Duration::from_secs(o.interval.clamp(1, 10)));
                match open_source(&o) {
                    Ok(s) => break s,
                    Err(e) => {
                        let r = serde_json::json!({"status":"no_source","node_id":o.node_id,"error":e,"timestamp":unix_ms()/1000});
                        println!("{r}");
                        if let Ok(mut st) = shared.lock() {
                            st.latest_report = Some(r);
                        }
                    }
                }
            }
        }
    };
    if let Ok(mut st) = shared.lock() {
        st.source = src.describe();
    }

    let export_url = match export::serve(&o.api_bind, shared.clone()) {
        Ok(addr) => Some(format!("http://{addr}/status")),
        Err(e) => {
            eprintln!("{TAG} export disabled: {e}");
            None
        }
    };

    loop {
        let t = match src.poll() {
            Ok(t) => t,
            Err(e) => {
                let r = serde_json::json!({"status":"no_source","node_id":o.node_id,"error":e,"timestamp":unix_ms()/1000});
                println!("{r}");
                eprintln!("{TAG} poll error: {e}");
                if let Ok(mut st) = shared.lock() {
                    st.latest_report = Some(r);
                    st.online = false;
                }
                if o.once {
                    return;
                }
                std::thread::sleep(Duration::from_secs(o.interval));
                continue;
            }
        };

        // Join the fleet only while the glasses are actually reachable: a successful heartbeat is
        // what makes (and keeps) the node online; dropping the link lets the host age it out.
        let joined = if t.online {
            match heartbeat::send(&o.host, &o.node_id, &t) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("{TAG} heartbeat: {e}");
                    false
                }
            }
        } else {
            false
        };

        let (report, vector) = build_report(&t, &o.node_id, joined, &export_url);
        if let Ok(mut st) = shared.lock() {
            st.serial = t.serial.clone();
            st.model = t.model.clone();
            st.online = t.online;
            st.polls += 1;
            if t.online {
                st.last_seen_ms = Some(unix_ms());
                if let Some(b) = t.battery_pct {
                    st.push(Sample {
                        t_ms: unix_ms(),
                        v: f64::from(b),
                    });
                }
            }
            st.latest_report = Some(report.clone());
        }
        println!("{report}");

        if t.online {
            if let Err(e) = store_to_seed(&vector) {
                eprintln!("{TAG} store error: {e}");
            }
        }

        if o.once {
            return;
        }
        std::thread::sleep(Duration::from_secs(o.interval));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adb::Plug;

    fn s(v: &[&str]) -> Vec<String> {
        std::iter::once("cog")
            .chain(v.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn options_bounded_and_defaulted() {
        let o = parse_opts(&s(&["--once", "--interval", "999", "--node-id", "g7"]));
        assert!(o.once);
        assert_eq!(o.interval, 5); // out of range -> default
        assert_eq!(o.node_id, "g7");
        assert_eq!(parse_opts(&s(&[])).api_bind, "0.0.0.0:8061");
        assert_eq!(parse_opts(&s(&[])).node_id, "mentra-01");
    }

    #[test]
    fn online_report_vector_is_bounded_and_shaped() {
        let t = Telemetry {
            online: true,
            serial: "X".into(),
            model: "Mentra Live".into(),
            fw: "1".into(),
            ip: Some("10.0.0.2".into()),
            battery_pct: Some(72),
            charging: true,
            plugged: Plug::Usb,
            temp_c: Some(28.4),
            voltage_v: Some(4.12),
            rtt_ms: Some(18.0),
        };
        let (r, v) = build_report(&t, "mentra-01", true, &None);
        assert_eq!(r["status"], "online");
        assert_eq!(r["battery_pct"], 72);
        assert_eq!(r["plugged"], "usb");
        assert_eq!(v.len(), 8);
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
        assert_eq!(v[0], 1.0);
        assert!((v[1] - 0.72).abs() < 1e-9);
        assert_eq!(v[2], 1.0);
        assert_eq!(v[3], 1.0);
    }

    #[test]
    fn offline_report_zeros_liveness() {
        let t = Telemetry {
            online: false,
            serial: "X".into(),
            model: "Mentra Live".into(),
            fw: String::new(),
            ip: None,
            battery_pct: None,
            charging: false,
            plugged: Plug::None,
            temp_c: None,
            voltage_v: None,
            rtt_ms: None,
        };
        let (r, v) = build_report(&t, "mentra-01", false, &None);
        assert_eq!(r["status"], "offline");
        assert_eq!(v[0], 0.0);
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
    }
}
