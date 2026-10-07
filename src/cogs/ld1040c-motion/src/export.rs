//! Shared motion state and a read-only HTTP export for a hook-up / companion tool, following
//! rd-03e's pattern (ADR-104/ADR-159). Plain HTTP/1.1 with `Access-Control-Allow-Origin: *` so a
//! browser build can poll it. Started only in continuous mode; `--once` never binds a port.
//!
//!   GET /status     latest report (JSON)
//!   GET /raw        decimated recent OUT-line trace (0/1 per sample)
//!   GET /guide      this cog's sensor guide (WeftOS ADR-104)
//!   GET /healthz    {"ok":true}

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// Keep this many seconds of OUT-line samples for `GET /raw` and window summaries.
pub const RING_SECONDS: f64 = 30.0;
const EVENT_SECONDS: u64 = 60;

/// One OUT-line reading at an instant.
#[derive(Clone, Copy)]
pub struct Sample {
    pub t_ms: u64,
    /// OUT logic level at this instant (true = motion asserted).
    pub high: bool,
}

/// Best-effort UART telemetry from the most recent parsed frame. Every field is optional; a
/// field the parser did not read stays `None` and is emitted as JSON null, never fabricated.
#[derive(Clone, Copy, Default)]
pub struct Telemetry {
    pub t_ms: u64,
    /// Mid-frequency AD average: the module's motion amplitude (higher with a moving target).
    pub motion_amplitude: Option<u8>,
    /// Signal energy, SUM2 / 64.
    pub signal: Option<f64>,
    /// Noise floor, SUM0 / 64.
    pub noise: Option<f64>,
}

pub struct State {
    pub source: String,
    pub start_ms: u64,
    pub ring: VecDeque<Sample>,
    /// Rising-edge (motion onset) times within the event window.
    pub events: VecDeque<u64>,
    pub total_events: u64,
    pub last_event_ms: Option<u64>,
    pub last_level: bool,
    pub samples_total: u64,
    pub telemetry: Option<Telemetry>,
    pub latest_report: Option<serde_json::Value>,
}

impl State {
    pub fn new(source: String, start_ms: u64) -> Self {
        Self {
            source,
            start_ms,
            ring: VecDeque::new(),
            events: VecDeque::new(),
            total_events: 0,
            last_event_ms: None,
            last_level: false,
            samples_total: 0,
            telemetry: None,
            latest_report: None,
        }
    }

    /// Push a sample, pruning beyond RING_SECONDS.
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

    /// Motion onsets within the last `seconds`, relative to `now_ms`.
    pub fn events_in(&self, seconds: u64, now_ms: u64) -> usize {
        let cutoff = now_ms.saturating_sub(seconds * 1000);
        self.events.iter().filter(|&&e| e >= cutoff).count()
    }

    /// Samples within the last `seconds`.
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

/// Rising edges (false -> true) and the fraction of samples high, over an ordered slice.
pub fn edges_and_fraction(tail: &[Sample]) -> (u64, f64) {
    if tail.is_empty() {
        return (0, 0.0);
    }
    let mut edges = 0u64;
    let mut high = 0u64;
    let mut prev = false;
    for (i, s) in tail.iter().enumerate() {
        if s.high {
            high += 1;
        }
        if i > 0 && s.high && !prev {
            edges += 1;
        }
        prev = s.high;
    }
    (edges, high as f64 / tail.len() as f64)
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
            .map(|s| serde_json::json!({"t_ms": s.t_ms, "v": u8::from(s.high)}))
            .collect();
        serde_json::json!({"source": st.source, "samples": samples}).to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_thirty_seconds() {
        let mut st = State::new("sim".into(), 0);
        for i in 0..400u64 {
            st.push(Sample {
                t_ms: 1_000 + i * 100,
                high: i % 2 == 0,
            });
        }
        // 30 s at 100 ms spacing = 301 samples retained.
        assert_eq!(st.ring.len(), 301);
    }

    #[test]
    fn edges_count_rising_transitions_only() {
        let seq = [false, true, true, false, true, false];
        let tail: Vec<Sample> = seq
            .iter()
            .enumerate()
            .map(|(i, &h)| Sample {
                t_ms: i as u64,
                high: h,
            })
            .collect();
        let (edges, frac) = edges_and_fraction(&tail);
        assert_eq!(edges, 2); // false->true at index 1 and index 4
        assert_eq!(frac, 3.0 / 6.0);
    }

    #[test]
    fn empty_tail_is_zero_not_nan() {
        let (edges, frac) = edges_and_fraction(&[]);
        assert_eq!(edges, 0);
        assert_eq!(frac, 0.0);
    }
}
