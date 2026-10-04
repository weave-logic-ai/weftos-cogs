//! Read-only HTTP/1.0 export of recent depth frames (std only):
//!
//!   GET /                    index
//!   GET /status              latest report (JSON)
//!   GET /frame               latest frame: {t_ms, side, mm: [...]} (row-major, X left->right, Y top->bottom)
//!   GET /frames?seconds=N    frames of the last N s (N <= 30)
//!   GET /raw.csv?seconds=N   CSV: t_ms,side,z0..z(n-1)
//!   GET /guide               this cog's sensor guide (WeftOS ADR-104)
//!   GET /spatial?seconds=N   spatial.evidence.v1 JSONL (feature `spatial-evidence`,
//!                            with `--spatial-out export`)

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const RING_SECONDS: u64 = 30;

#[derive(Clone, Debug)]
pub struct Frame {
    pub t_ms: u64,
    pub side: usize,
    pub mm: Vec<u16>,
}

#[derive(Default)]
pub struct Ring {
    pub source: String,
    pub frames: VecDeque<Frame>,
    pub latest_report: Option<serde_json::Value>,
    /// `spatial.evidence.v1` lines with their frame time (feature `spatial-evidence`).
    #[cfg(feature = "spatial-evidence")]
    pub evidence: VecDeque<(u64, String)>,
    /// The last spatial-evidence write error since the previous report.
    #[cfg(feature = "spatial-evidence")]
    pub spatial_error: Option<String>,
}

pub type Shared = Arc<Mutex<Ring>>;

impl Ring {
    pub fn push(&mut self, f: Frame) {
        let cutoff = f.t_ms.saturating_sub(RING_SECONDS * 1000);
        self.frames.push_back(f);
        while self.frames.front().is_some_and(|x| x.t_ms < cutoff) {
            self.frames.pop_front();
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

    pub fn since(&self, seconds: f64) -> Vec<&Frame> {
        let Some(newest) = self.frames.back().map(|f| f.t_ms) else {
            return Vec::new();
        };
        let s = seconds.clamp(0.0, RING_SECONDS as f64);
        let cutoff = newest.saturating_sub((s * 1000.0) as u64);
        self.frames.iter().filter(|f| f.t_ms >= cutoff).collect()
    }
}

fn query_seconds(path: &str) -> f64 {
    path.split_once('?')
        .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("seconds=")))
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(5.0)
}

fn frame_json(f: &Frame) -> serde_json::Value {
    serde_json::json!({ "t_ms": f.t_ms, "side": f.side, "mm": f.mm })
}

pub fn render(ring: &Ring, method: &str, path: &str) -> (u16, &'static str, String) {
    if method != "GET" {
        return (405, "text/plain", "GET only\n".into());
    }
    match path.split('?').next().unwrap_or("") {
        "/" => (
            200,
            "text/plain",
            "sen0628-tof export\n  /status\n  /frame\n  /frames?seconds=N   (JSON, N <= 30)\n  /raw.csv?seconds=N  (CSV)\n  /guide              (this cog's sensor guide, ADR-104)\n".into(),
        ),
        "/status" => (
            200,
            "application/json",
            ring.latest_report.as_ref().map_or_else(|| "{\"status\":\"starting\"}".into(), |r| r.to_string()),
        ),
        "/frame" => match ring.frames.back() {
            Some(f) => (200, "application/json", frame_json(f).to_string()),
            None => (200, "application/json", "{\"side\":0,\"mm\":[]}".into()),
        },
        "/frames" => {
            let fr: Vec<serde_json::Value> = ring.since(query_seconds(path)).into_iter().map(frame_json).collect();
            (200, "application/json", serde_json::json!({ "source": ring.source, "frames": fr }).to_string())
        }
        "/raw.csv" => {
            let fr = ring.since(query_seconds(path));
            let zones = fr.first().map_or(0, |f| f.mm.len());
            let mut out = String::from("t_ms,side");
            for z in 0..zones {
                out.push_str(&format!(",z{z}"));
            }
            out.push('\n');
            for f in fr {
                out.push_str(&format!("{},{}", f.t_ms, f.side));
                for d in &f.mm {
                    out.push_str(&format!(",{d}"));
                }
                out.push('\n');
            }
            (200, "text/csv", out)
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
            r.push(Frame {
                t_ms: 1_000 + i * 100,
                side: 4,
                mm: vec![1000; 16],
            });
        }
        r
    }

    #[test]
    fn ring_keeps_thirty_seconds() {
        let r = ring(500);
        assert_eq!(r.frames.len(), 301);
    }

    #[test]
    fn routes_return_frames_and_csv() {
        let r = ring(50);
        let (_, _, f) = render(&r, "GET", "/frame");
        assert!(f.contains("\"side\":4"));
        let (_, _, fs) = render(&r, "GET", "/frames?seconds=1");
        let v: serde_json::Value = serde_json::from_str(&fs).unwrap();
        assert_eq!(v["frames"].as_array().unwrap().len(), 11);
        let (_, ctype, csv) = render(&r, "GET", "/raw.csv?seconds=1");
        assert_eq!(ctype, "text/csv");
        assert!(csv.lines().next().unwrap().ends_with(",z15"));
        assert_eq!(render(&r, "POST", "/frame").0, 405);
        assert_eq!(render(&r, "GET", "/nope").0, 404);
        #[cfg(not(feature = "spatial-evidence"))]
        assert_eq!(render(&r, "GET", "/spatial").0, 404);
        assert_eq!(render(&r, "GET", "/frames?seconds=NaN").0, 200);
    }
}
