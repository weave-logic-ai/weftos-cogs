//! Read-only HTTP export for a hook-up/companion tool: the latest report and a decimated recent
//! distance trace. Plain HTTP with `Access-Control-Allow-Origin: *` so a browser build can poll it.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

pub const RING_SECONDS: f64 = 15.0;
const EVENT_SECONDS: u64 = 60;
#[cfg(feature = "spatial-evidence")]
const EVIDENCE_SECONDS: u64 = 30;

#[derive(Clone, Copy)]
pub struct Sample {
    pub t_ms: u64,
    /// Distance in cm at this instant (0 when no target).
    pub v: f64,
}

pub struct State {
    pub fs: f64,
    pub source: String,
    pub ring: VecDeque<Sample>,
    /// Presence onsets (clear -> target) within the event window.
    pub events: VecDeque<u64>,
    pub total_events: u64,
    pub last_event_ms: Option<u64>,
    /// Most recent decoded frame.
    pub last_state: u8,
    pub last_distance_cm: u16,
    pub frames: u64,
    pub latest_report: Option<serde_json::Value>,
    /// `spatial.evidence.v1` lines with their frame time (feature `spatial-evidence`).
    #[cfg(feature = "spatial-evidence")]
    pub evidence: VecDeque<(u64, String)>,
    /// The last spatial-evidence write error since the previous report.
    #[cfg(feature = "spatial-evidence")]
    pub spatial_error: Option<String>,
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
            last_state: 0,
            last_distance_cm: 0,
            frames: 0,
            latest_report: None,
            #[cfg(feature = "spatial-evidence")]
            evidence: VecDeque::new(),
            #[cfg(feature = "spatial-evidence")]
            spatial_error: None,
        }
    }

    /// Keep the last 30 s of evidence lines for `GET /spatial`.
    #[cfg(feature = "spatial-evidence")]
    pub fn push_evidence(&mut self, t_ms: u64, line: String) {
        let cutoff = t_ms.saturating_sub(EVIDENCE_SECONDS * 1000);
        self.evidence.push_back((t_ms, line));
        while self.evidence.front().is_some_and(|x| x.0 < cutoff) {
            self.evidence.pop_front();
        }
    }

    /// Evidence lines of the last `seconds` (at most 30) as JSONL.
    #[cfg(feature = "spatial-evidence")]
    pub fn evidence_jsonl(&self, seconds: f64) -> String {
        let newest = self.evidence.back().map_or(0, |x| x.0);
        let s = if seconds.is_finite() {
            seconds.clamp(0.0, EVIDENCE_SECONDS as f64)
        } else {
            5.0
        };
        let cutoff = newest.saturating_sub((s * 1000.0) as u64);
        let mut out = String::new();
        for (_, line) in self.evidence.iter().filter(|x| x.0 >= cutoff) {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Push a (decimated) display sample, pruning beyond RING_SECONDS.
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

    /// Events within the last `seconds`, relative to `now_ms`.
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

    // `GET /spatial?seconds=N`: spatial.evidence.v1 JSONL (feature `spatial-evidence`).
    #[cfg(feature = "spatial-evidence")]
    if path.starts_with("/spatial") {
        let seconds = path
            .split_once("seconds=")
            .and_then(|(_, v)| v.split('&').next())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(5.0);
        let body = shared.lock().unwrap().evidence_jsonl(seconds);
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
            body.len()
        );
        s.write_all(resp.as_bytes())?;
        s.write_all(body.as_bytes())?;
        return s.flush();
    }

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

#[cfg(all(test, feature = "spatial-evidence"))]
mod evidence_tests {
    use super::*;

    #[test]
    fn evidence_ring_keeps_thirty_seconds_and_serves_jsonl() {
        let mut st = State::new(256_000.0, "sim".into());
        for i in 0..400u64 {
            st.push_evidence(1_000 + i * 100, format!("{{\"i\":{i}}}"));
        }
        assert_eq!(st.evidence.len(), 301);
        let body = st.evidence_jsonl(1.0);
        assert_eq!(body.lines().count(), 11);
        assert!(body.ends_with("{\"i\":399}\n"));
        assert_eq!(st.evidence_jsonl(f64::NAN).lines().count(), 51);
    }
}
