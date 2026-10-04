//! Read-only HTTP export for a hook-up/companion tool: the latest report and a decimated recent
//! motion trace. Plain HTTP with `Access-Control-Allow-Origin: *` so a browser build can poll it.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

pub const RING_SECONDS: f64 = 15.0;
const EVENT_SECONDS: u64 = 60;

#[derive(Clone, Copy)]
pub struct Sample {
    pub t_ms: u64,
    /// Overall angular-rate magnitude (deg/s) at this instant — the motion trace.
    pub v: f64,
}

pub struct State {
    pub fs: f64,
    pub source: String,
    pub ring: VecDeque<Sample>,
    /// Motion onsets (still -> moving) within the event window.
    pub events: VecDeque<u64>,
    pub total_events: u64,
    pub last_event_ms: Option<u64>,
    /// Latest decoded IMU values (HLK-AS201 Hi-Link protocol).
    pub accel: [f64; 3], // g
    pub gyro: [f64; 3],  // deg/s
    pub euler: [f64; 3], // roll, pitch, yaw (deg)
    pub mag: [f64; 3],   // µT
    pub quat: [f64; 4],  // w, x, y, z
    pub temp_c: f64,
    pub pressure_hpa: f64,
    pub height_m: f64,
    pub mag_accuracy: u8, // 0 uncalibrated/strong interference, 3 calibrated
    pub packets: u64,
    pub latest_report: Option<serde_json::Value>,
}

impl State {
    pub fn new(fs: f64, source: String) -> Self {
        Self {
            fs,
            source,
            ring: VecDeque::new(),
            events: VecDeque::new(),
            total_events: 0,
            last_event_ms: None,
            accel: [0.0; 3],
            gyro: [0.0; 3],
            euler: [0.0; 3],
            mag: [0.0; 3],
            quat: [0.0; 4],
            temp_c: 0.0,
            pressure_hpa: 0.0,
            height_m: 0.0,
            mag_accuracy: 0,
            packets: 0,
            latest_report: None,
        }
    }

    pub fn push(&mut self, s: Sample) {
        let newest = s.t_ms;
        self.ring.push_back(s);
        let cutoff = newest.saturating_sub((RING_SECONDS * 1000.0) as u64);
        while self.ring.front().is_some_and(|f| f.t_ms < cutoff) {
            self.ring.pop_front();
        }
    }

    pub fn mark_event(&mut self, t_ms: u64) {
        self.events.push_back(t_ms);
        self.total_events += 1;
        self.last_event_ms = Some(t_ms);
        let cutoff = t_ms.saturating_sub(EVENT_SECONDS * 1000);
        while self.events.front().is_some_and(|&e| e < cutoff) {
            self.events.pop_front();
        }
    }

    pub fn tail(&self, seconds: f64) -> Vec<Sample> {
        let Some(newest) = self.ring.back().map(|s| s.t_ms) else {
            return Vec::new();
        };
        let cutoff = newest.saturating_sub((seconds * 1000.0) as u64);
        self.ring
            .iter()
            .filter(|s| s.t_ms >= cutoff)
            .copied()
            .collect()
    }

    pub fn events_in(&self, seconds: u64, now_ms: u64) -> usize {
        let cutoff = now_ms.saturating_sub(seconds * 1000);
        self.events.iter().filter(|&&e| e >= cutoff).count()
    }
}

pub type Shared = Arc<Mutex<State>>;

pub fn round(x: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (x * f).round() / f
}

pub fn serve(bind: &str, shared: Shared) -> Result<SocketAddr, String> {
    let listener = TcpListener::bind(bind).map_err(|e| format!("bind {bind}: {e}"))?;
    let addr = listener.local_addr().map_err(|e| format!("addr: {e}"))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = handle(stream, &shared);
        }
    });
    Ok(addr)
}

fn handle(mut s: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let mut buf = [0u8; 1024];
    let n = s.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("/");

    let body: String = if path.starts_with("/status") {
        let st = shared.lock().unwrap();
        st.latest_report
            .clone()
            .unwrap_or_else(|| serde_json::json!({"status":"starting"}))
            .to_string()
    } else if path.starts_with("/raw") {
        let st = shared.lock().unwrap();
        let samples: Vec<serde_json::Value> = st
            .tail(RING_SECONDS)
            .iter()
            .map(|s| serde_json::json!({"t_ms": s.t_ms, "v": round(s.v, 3)}))
            .collect();
        serde_json::json!({"source": st.source, "fs": st.fs, "samples": samples}).to_string()
    } else if path.starts_with("/guide") {
        crate::guide::bundle_json().to_string()
    } else if path.starts_with("/healthz") {
        r#"{"ok":true}"#.to_string()
    } else {
        r#"{"error":"not found"}"#.to_string()
    };

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(resp.as_bytes())?;
    s.write_all(body.as_bytes())?;
    s.flush()
}
