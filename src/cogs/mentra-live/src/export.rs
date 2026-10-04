//! Read-only HTTP export for a companion/console tool: the latest telemetry report and a decimated
//! recent battery trace. Plain HTTP with `Access-Control-Allow-Origin: *` so a browser build can poll
//! it. Mirrors the other cogs' `/status` · `/raw` · `/guide` · `/healthz` surface.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// How much battery history `/raw` keeps (seconds) — long enough to see a charge/discharge trend.
pub const RING_SECONDS: f64 = 1800.0;

#[derive(Clone, Copy)]
pub struct Sample {
    pub t_ms: u64,
    /// Battery percent at this instant.
    pub v: f64,
}

pub struct State {
    pub source: String,
    pub serial: String,
    pub model: String,
    pub online: bool,
    pub polls: u64,
    pub last_seen_ms: Option<u64>,
    pub ring: VecDeque<Sample>,
    pub latest_report: Option<serde_json::Value>,
}

impl State {
    pub fn new(source: String) -> Self {
        Self {
            source,
            serial: String::new(),
            model: String::new(),
            online: false,
            polls: 0,
            last_seen_ms: None,
            ring: VecDeque::new(),
            latest_report: None,
        }
    }

    /// Push a battery sample, pruning beyond RING_SECONDS.
    pub fn push(&mut self, s: Sample) {
        let newest = s.t_ms;
        self.ring.push_back(s);
        let cutoff = newest.saturating_sub((RING_SECONDS * 1000.0) as u64);
        while self.ring.front().is_some_and(|f| f.t_ms < cutoff) {
            self.ring.pop_front();
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
            .map(|s| serde_json::json!({"t_ms": s.t_ms, "v": round(s.v, 1)}))
            .collect();
        serde_json::json!({"source": st.source, "serial": st.serial, "model": st.model, "battery_pct": samples}).to_string()
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
