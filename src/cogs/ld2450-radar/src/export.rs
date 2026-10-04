//! Read-only HTTP/1.0 export of recent targets (std only), sen0628-tof's pattern
//! (ADR-159). Started only in continuous mode; `--once` never binds a port.
//!
//!   GET /                    index
//!   GET /status              latest report (JSON)
//!   GET /targets             latest frame's targets: {t_ms, targets: [...]}
//!   GET /frames?seconds=N    decoded frames of the last N s (N <= 30)
//!   GET /guide               this cog's sensor guide (WeftOS ADR-104)
//!
//! The export carries people's positions, so it binds 127.0.0.1 by default.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::window::Target;

pub const RING_SECONDS: u64 = 30;

#[derive(Clone, Debug)]
pub struct Sample {
    pub t_ms: u64,
    pub targets: Vec<Target>,
}

#[derive(Default)]
pub struct Ring {
    pub source: String,
    pub samples: VecDeque<Sample>,
    pub latest_report: Option<serde_json::Value>,
    /// `spatial.evidence.v1` lines with their frame time (feature `spatial-evidence`).
    #[cfg(feature = "spatial-evidence")]
    pub evidence: VecDeque<(u64, String)>,
}

pub type Shared = Arc<Mutex<Ring>>;

impl Ring {
    pub fn push(&mut self, s: Sample) {
        let cutoff = s.t_ms.saturating_sub(RING_SECONDS * 1000);
        self.samples.push_back(s);
        while self.samples.front().is_some_and(|x| x.t_ms < cutoff) {
            self.samples.pop_front();
        }
    }

    #[cfg(feature = "spatial-evidence")]
    pub fn push_evidence(&mut self, t_ms: u64, line: String) {
        let cutoff = t_ms.saturating_sub(RING_SECONDS * 1000);
        self.evidence.push_back((t_ms, line));
        while self.evidence.front().is_some_and(|x| x.0 < cutoff) {
            self.evidence.pop_front();
        }
    }

    pub fn since(&self, seconds: f64) -> Vec<&Sample> {
        let Some(newest) = self.samples.back().map(|s| s.t_ms) else {
            return Vec::new();
        };
        let s = seconds.clamp(0.0, RING_SECONDS as f64);
        let cutoff = newest.saturating_sub((s * 1000.0) as u64);
        self.samples.iter().filter(|x| x.t_ms >= cutoff).collect()
    }
}

fn query_seconds(path: &str) -> f64 {
    path.split_once('?')
        .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("seconds=")))
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(5.0)
}

fn sample_json(s: &Sample) -> serde_json::Value {
    serde_json::json!({ "t_ms": s.t_ms, "targets": s.targets })
}

pub fn render(ring: &Ring, method: &str, path: &str) -> (u16, &'static str, String) {
    if method != "GET" {
        return (405, "text/plain", "GET only\n".into());
    }
    match path.split('?').next().unwrap_or("") {
        "/" => (
            200,
            "text/plain",
            "ld2450-radar export (radar_local frame, metres)\n  /status\n  /targets\n  /frames?seconds=N   (JSON, N <= 30)\n  /guide              (this cog's sensor guide, ADR-104)\n".into(),
        ),
        "/status" => (
            200,
            "application/json",
            ring.latest_report
                .as_ref()
                .map_or_else(|| "{\"health\":\"starting\"}".into(), |r| r.to_string()),
        ),
        "/targets" => match ring.samples.back() {
            Some(s) => (200, "application/json", sample_json(s).to_string()),
            None => (200, "application/json", "{\"t_ms\":null,\"targets\":[]}".into()),
        },
        "/frames" => {
            let fr: Vec<serde_json::Value> =
                ring.since(query_seconds(path)).into_iter().map(sample_json).collect();
            (
                200,
                "application/json",
                serde_json::json!({ "source": ring.source, "frame": crate::window::TARGET_FRAME, "frames": fr })
                    .to_string(),
            )
        }
        "/guide" => (200, "application/json", crate::guide::bundle_json().to_string()),
        #[cfg(feature = "spatial-evidence")]
        "/spatial" => {
            let newest = ring.evidence.back().map_or(0, |x| x.0);
            let s = query_seconds(path).clamp(0.0, RING_SECONDS as f64);
            let cutoff = newest.saturating_sub((s * 1000.0) as u64);
            let mut out = String::new();
            for (_, line) in ring.evidence.iter().filter(|x| x.0 >= cutoff) {
                out.push_str(line);
                out.push('\n');
            }
            (200, "application/x-ndjson", out)
        }
        _ => (404, "text/plain", "not found\n".into()),
    }
}

fn handle(mut conn: TcpStream, shared: &Shared) {
    conn.set_read_timeout(Some(Duration::from_secs(3))).ok();
    conn.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let mut line = String::new();
    if BufReader::new(&conn).read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let (code, ctype, body) = match shared.lock() {
        Ok(ring) => render(&ring, method, path),
        Err(_) => (500, "text/plain", "state poisoned\n".into()),
    };
    let reason = match code {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let _ = write!(
        conn,
        "HTTP/1.0 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// Starts the export on its own thread; a bind failure is reported, not fatal.
pub fn serve(bind: &str, shared: Shared) -> Result<String, String> {
    let listener = TcpListener::bind(bind).map_err(|e| format!("bind {bind}: {e}"))?;
    let addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| bind.to_string());
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            handle(conn, &shared);
        }
    });
    Ok(addr)
}

#[cfg(all(test, feature = "spatial-evidence"))]
mod evidence_tests {
    use super::*;

    #[test]
    fn evidence_route_serves_recent_jsonl() {
        let mut r = Ring::default();
        for i in 0..400u64 {
            r.push_evidence(1_000 + i * 100, format!("{{\"i\":{i}}}"));
        }
        assert_eq!(r.evidence.len(), 301);
        let (code, ctype, body) = render(&r, "GET", "/spatial?seconds=1");
        assert_eq!((code, ctype), (200, "application/x-ndjson"));
        assert_eq!(body.lines().count(), 11);
        assert!(body.ends_with("{\"i\":399}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(n: u64) -> Ring {
        let mut r = Ring::default();
        for i in 0..n {
            r.push(Sample {
                t_ms: 1_000 + i * 100,
                targets: vec![Target {
                    slot: 1,
                    x_m: -0.5,
                    y_m: 2.0,
                    speed_mps: 0.1,
                    resolution_mm: 360,
                }],
            });
        }
        r
    }

    #[test]
    fn ring_keeps_thirty_seconds() {
        assert_eq!(ring(500).samples.len(), 301);
    }

    #[test]
    fn routes() {
        let r = ring(50);
        let (_, _, t) = render(&r, "GET", "/targets");
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["targets"][0]["x_m"], -0.5);
        let (_, _, fs) = render(&r, "GET", "/frames?seconds=1");
        let v: serde_json::Value = serde_json::from_str(&fs).unwrap();
        assert_eq!(v["frames"].as_array().unwrap().len(), 11);
        assert_eq!(v["frame"], "radar_local");
        assert_eq!(render(&r, "GET", "/status").2, "{\"health\":\"starting\"}");
        assert_eq!(render(&r, "POST", "/targets").0, 405);
        assert_eq!(render(&r, "GET", "/nope").0, 404);
        #[cfg(not(feature = "spatial-evidence"))]
        assert_eq!(render(&r, "GET", "/spatial").0, 404);
        assert_eq!(render(&r, "GET", "/frames?seconds=NaN").0, 200);
        let (_, _, empty) = render(&Ring::default(), "GET", "/targets");
        assert_eq!(empty, "{\"t_ms\":null,\"targets\":[]}");
    }
}
