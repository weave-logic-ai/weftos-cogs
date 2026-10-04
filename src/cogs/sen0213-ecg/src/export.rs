//! Raw and processed signal export: a 60 s in-memory ring of samples and R-peaks, served over
//! a small read-only HTTP/1.0 endpoint (std only, no async runtime).
//!
//!   GET /           index
//!   GET /status     latest report (JSON)
//!   GET /guide      the cog's sensor guide bundle (WeftOS ADR-104)
//!   GET /raw?seconds=N       JSON: [[t_ms, raw_v, filtered_mv], ...] plus R-peak times
//!   GET /raw.csv?seconds=N   CSV:  t_ms,raw_v,filtered_mv,r_peak

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const RING_SECONDS: f64 = 60.0;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub idx: u64,
    pub t_ms: u64,
    pub raw_v: f64,
    pub filtered_mv: f64,
}

pub struct Ring {
    pub fs: f64,
    pub source: String,
    pub samples: VecDeque<Sample>,
    pub peaks: VecDeque<(u64, u64)>, // (sample index, t_ms)
    pub latest_report: Option<serde_json::Value>,
    cap: usize,
}

pub type Shared = Arc<Mutex<Ring>>;

impl Ring {
    pub fn new(fs: f64, source: String) -> Self {
        let cap = (fs * RING_SECONDS) as usize;
        Self {
            fs,
            source,
            samples: VecDeque::with_capacity(cap),
            peaks: VecDeque::new(),
            latest_report: None,
            cap,
        }
    }

    pub fn push(&mut self, s: Sample) {
        self.samples.push_back(s);
        while self.samples.len() > self.cap {
            self.samples.pop_front();
        }
        let oldest = self.samples.front().map_or(0, |s| s.idx);
        while self.peaks.front().is_some_and(|&(i, _)| i < oldest) {
            self.peaks.pop_front();
        }
    }

    pub fn push_peak(&mut self, idx: u64, t_ms: u64) {
        self.peaks.push_back((idx, t_ms));
    }

    /// Samples and peaks of the last `seconds` (clamped to the ring).
    pub fn tail(&self, seconds: f64) -> (Vec<Sample>, Vec<(u64, u64)>) {
        let n = ((seconds.clamp(0.0, RING_SECONDS)) * self.fs) as usize;
        let skip = self.samples.len().saturating_sub(n);
        let s: Vec<Sample> = self.samples.iter().skip(skip).copied().collect();
        let first = s.first().map_or(u64::MAX, |x| x.idx);
        let p = self
            .peaks
            .iter()
            .filter(|&&(i, _)| i >= first)
            .copied()
            .collect();
        (s, p)
    }
}

fn query_seconds(path: &str) -> f64 {
    path.split_once('?')
        .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("seconds=")))
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(10.0)
}

pub fn render(ring: &Ring, method: &str, path: &str) -> (u16, &'static str, String) {
    if method != "GET" {
        return (405, "text/plain", "GET only\n".into());
    }
    let route = path.split('?').next().unwrap_or("");
    match route {
        "/" => (
            200,
            "text/plain",
            "sen0213-ecg export\n  /status\n  /guide              (this cog's sensor guide, ADR-104)\n  /raw?seconds=N      (JSON, N <= 60)\n  /raw.csv?seconds=N  (CSV)\n".into(),
        ),
        "/status" => (
            200,
            "application/json",
            ring.latest_report.as_ref().map_or_else(|| "{\"status\":\"starting\"}".into(), |r| r.to_string()),
        ),
        "/guide" => (200, "application/json", crate::guide::bundle_json().to_string()),
        "/raw" => {
            let (s, p) = ring.tail(query_seconds(path));
            let body = serde_json::json!({
                "source": ring.source,
                "sample_rate_hz": ring.fs,
                "columns": ["t_ms", "raw_v", "filtered_mv"],
                "samples": s.iter().map(|x| serde_json::json!([x.t_ms, round(x.raw_v, 5), round(x.filtered_mv, 3)])).collect::<Vec<_>>(),
                "r_peaks_ms": p.iter().map(|&(_, t)| t).collect::<Vec<_>>(),
            });
            (200, "application/json", body.to_string())
        }
        "/raw.csv" => {
            let (s, p) = ring.tail(query_seconds(path));
            let mut out = String::from("t_ms,raw_v,filtered_mv,r_peak\n");
            for x in &s {
                let peak = p.iter().any(|&(i, _)| i == x.idx);
                out.push_str(&format!("{},{:.5},{:.3},{}\n", x.t_ms, x.raw_v, x.filtered_mv, u8::from(peak)));
            }
            (200, "text/csv", out)
        }
        _ => (404, "text/plain", "not found\n".into()),
    }
}

pub fn round(v: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (v * m).round() / m
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

/// Starts the export server on its own thread. A bind failure is reported, not fatal: the cog
/// still measures and ingests without the export.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_with(n: u64) -> Ring {
        let mut r = Ring::new(250.0, "test".into());
        for i in 0..n {
            r.push(Sample {
                idx: i,
                t_ms: 1000 + i * 4,
                raw_v: 1.5,
                filtered_mv: 0.0,
            });
        }
        r.push_peak(n - 10, 1000 + (n - 10) * 4);
        r
    }

    #[test]
    fn ring_is_capped_at_sixty_seconds_and_drops_stale_peaks() {
        let mut r = ring_with(10);
        for i in 10..20_000 {
            r.push(Sample {
                idx: i,
                t_ms: i * 4,
                raw_v: 1.5,
                filtered_mv: 0.0,
            });
        }
        assert_eq!(r.samples.len(), 15_000);
        assert!(r.peaks.is_empty());
    }

    #[test]
    fn raw_json_and_csv_cover_the_requested_seconds() {
        let r = ring_with(5000);
        let (code, _, body) = render(&r, "GET", "/raw?seconds=2");
        assert_eq!(code, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["samples"].as_array().unwrap().len(), 500);
        assert_eq!(v["r_peaks_ms"].as_array().unwrap().len(), 1);
        let (_, ctype, csv) = render(&r, "GET", "/raw.csv?seconds=1");
        assert_eq!(ctype, "text/csv");
        assert_eq!(csv.lines().count(), 251);
        assert_eq!(csv.lines().filter(|l| l.ends_with(",1")).count(), 1);
    }

    #[test]
    fn bad_requests_are_refused() {
        let r = ring_with(100);
        assert_eq!(render(&r, "POST", "/raw").0, 405);
        assert_eq!(render(&r, "GET", "/etc/passwd").0, 404);
        // A hostile seconds value is clamped, not trusted.
        assert_eq!(render(&r, "GET", "/raw?seconds=1e300").0, 200);
        assert_eq!(render(&r, "GET", "/raw?seconds=NaN").0, 200);
    }
}
